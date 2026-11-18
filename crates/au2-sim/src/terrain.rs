//! Terrain zones: force (straight, radial, quarter-turning), speed, reflect, minefields that wear out, confusion and
//! neutral zones.

use crate::math::Vec2;
use crate::rules::*;
use crate::{v2, Mode, Ship, Sim, Target, NO_SUN};
use au2_map::{TerrainDef, TerrainKind};

#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    pub def: TerrainDef,
    pub enabled: bool,
    /// Quarter turns applied to the push direction so far (force zones that turn).
    pub quarter: u8,
    /// Minefields: mines left per cell (row-major over `mine_cols` x `mine_rows` cells of `mine_cell` size from
    /// `mine_origin`), and how many each cell started with.
    pub mines: Vec<u16>,
    pub mines_start: u16,
    pub mine_origin: Vec2,
    pub mine_cell: f64,
    pub mine_cols: u32,
    pub mine_rows: u32,
}

impl Terrain {
    pub(crate) fn new(d: &TerrainDef) -> Terrain {
        let mut t = Terrain {
            def: d.clone(),
            enabled: d.enabled,
            quarter: 0,
            mines: Vec::new(),
            mines_start: 0,
            mine_origin: v2(d.pos),
            mine_cell: d.tile_size.max(1.0),
            mine_cols: d.cols,
            mine_rows: d.rows,
        };
        if d.kind == TerrainKind::Minefield {
            if d.outer_radius > 0.0 {
                let r = d.outer_radius;
                t.mine_origin = v2(d.pos) - Vec2::new(r, r);
                t.mine_cell = MINE_CELL;
                t.mine_cols = (2.0 * r / MINE_CELL).ceil() as u32;
                t.mine_rows = t.mine_cols;
            }
            t.mines_start = (MINES_BASE as f64 + d.density.max(0.0) / 25.0).round().max(1.0) as u16;
            t.mines = vec![t.mines_start; (t.mine_cols * t.mine_rows) as usize];
        }
        t
    }

    pub fn circular(&self) -> bool {
        self.def.outer_radius > 0.0
    }

    pub fn contains(&self, p: Vec2) -> bool {
        if !self.enabled {
            return false;
        }
        let d = &self.def;
        let c = v2(d.pos);
        if d.outer_radius > 0.0 {
            let r = p.dist(c);
            return r >= d.inner_radius && r <= d.outer_radius;
        }
        let cx = ((p.x - c.x) / d.tile_size).floor();
        let cy = ((p.y - c.y) / d.tile_size).floor();
        if cx < 0.0 || cy < 0.0 || cx >= d.cols as f64 || cy >= d.rows as f64 {
            return false;
        }
        d.data.get(cy as usize * d.cols as usize + cx as usize).copied().unwrap_or(0) != 0
    }

    /// Minefield cell index of `p`.
    pub fn mine_cell_of(&self, p: Vec2) -> Option<usize> {
        let x = ((p.x - self.mine_origin.x) / self.mine_cell).floor();
        let y = ((p.y - self.mine_origin.y) / self.mine_cell).floor();
        if x < 0.0 || y < 0.0 || x >= self.mine_cols as f64 || y >= self.mine_rows as f64 {
            return None;
        }
        Some(y as usize * self.mine_cols as usize + x as usize)
    }

    /// Share of mines left in the cell at `p` (1 = untouched).
    pub fn mines_left(&self, p: Vec2) -> f64 {
        match self.mine_cell_of(p) {
            Some(k) if self.mines_start > 0 => self.mines[k] as f64 / self.mines_start as f64,
            _ => 0.0,
        }
    }

    /// Push (units per tick per tick) on a ship at `p`.
    pub fn push_at(&self, p: Vec2) -> Vec2 {
        let a = v2(self.def.acceleration);
        let turn = self.quarter as f64 * 90.0;
        if self.circular() {
            // radial: positive acceleration pushes outward, negative inward; turning zones swirl
            let strength = a.len();
            let sign = if a.x + a.y >= 0.0 { 1.0 } else { -1.0 };
            let out = (p - v2(self.def.pos)).norm_or(Vec2::new(1.0, 0.0));
            (out * sign).rotate(turn) * (strength * FORCE_SCALE)
        } else {
            a.rotate(turn) * FORCE_SCALE
        }
    }

    pub(crate) fn tick_switch(&mut self, tick: u64) {
        if self.def.switch_time > 0 && tick % self.def.switch_time as u64 == 0 {
            if self.def.kind == TerrainKind::Force && self.def.dir_switch != 0 {
                self.quarter = (self.quarter + 1) % 4;
            } else {
                self.enabled = !self.enabled;
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct TerrainFx {
    pub speed: f64,
    pub accel: Vec2,
}

fn rate_ppm(percent_per_sec: f64) -> u32 {
    // percent per second -> parts per million per tick
    (percent_per_sec * 10000.0 / TICKS_PER_SEC as f64).clamp(0.0, 1_000_000.0) as u32
}

impl Sim {
    pub(crate) fn refresh_strength_cache(&mut self) {
        // only confusion zones need it
        if !self.terrain_idx.iter().any(|&i| matches!(&self.objs[i], crate::Obj::Terrain(t) if t.def.kind == TerrainKind::Confusion)) {
            return;
        }
        let mut st = vec![0u32; self.factions.len()];
        for s in &self.ships {
            if s.alive {
                st[s.faction as usize] += s.strength;
            }
        }
        self.strength_cache = st;
    }

    /// Weakest faction (other than `not`) that still owns a living sun.
    fn weakest_faction(&self, not: u8) -> Option<u8> {
        let mut best: Option<(u32, u8)> = None;
        for f in 0..self.factions.len() as u8 {
            if f == not || !self.factions[f as usize].alive || self.sun_count(f) == 0 {
                continue;
            }
            let st = self.strength_cache.get(f as usize).copied().unwrap_or(0);
            if best.map(|(b, _)| st < b).unwrap_or(true) {
                best = Some((st, f));
            }
        }
        best.map(|(_, f)| f)
    }

    /// Apply the zones at the ship's position: mines, confusion, neutrality; returns speed and push.
    pub(crate) fn terrain_effects(&mut self, s: &mut Ship) -> TerrainFx {
        let mut fx = TerrainFx { speed: 1.0, accel: Vec2::ZERO };
        s.neutral = false;
        for k in 0..self.terrain_idx.len() {
            let i = self.terrain_idx[k];
            let crate::Obj::Terrain(t) = &self.objs[i] else { continue };
            if !t.contains(s.pos) {
                continue;
            }
            match t.def.kind {
                TerrainKind::Force => fx.accel += t.push_at(s.pos),
                TerrainKind::Speed => fx.speed *= (1.0 + t.def.speed_change / SPEED_CHANGE_UNIT).clamp(SPEED_MIN, SPEED_MAX),
                TerrainKind::Reflect => {}
                TerrainKind::Minefield => {
                    let Some(cell) = t.mine_cell_of(s.pos) else { continue };
                    if t.mines[cell] == 0 {
                        continue;
                    }
                    let ppm = t.def.death_rate.max(0.0) * MINE_PPM_PER_RATE * (t.def.density.max(0.0) / 100.0) * t.mines[cell] as f64 / t.mines_start.max(1) as f64;
                    if self.rng.chance_ppm(ppm as u32) {
                        if let crate::Obj::Terrain(t) = &mut self.objs[i] {
                            t.mines[cell] -= 1;
                        }
                        s.strength -= 1;
                        if s.strength == 0 {
                            s.alive = false;
                            return fx;
                        }
                    }
                }
                TerrainKind::Confusion => {
                    let (fc, lo) = (rate_ppm(t.def.faction_change_rate), rate_ppm(t.def.lose_order_rate));
                    if fc > 0 && self.rng.chance_ppm(fc) {
                        if let Some(to) = self.weakest_faction(s.faction) {
                            s.faction = to;
                            s.target = Target::None;
                            s.mode = Mode::Hover;
                            s.home = NO_SUN;
                            s.hold = false;
                            s.guard = 0;
                        }
                    }
                    if lo > 0 && self.rng.chance_ppm(lo) {
                        // lose the order and head for a random sun
                        let suns: Vec<usize> = self.suns().filter(|(_, x)| x.present()).map(|(j, _)| j).collect();
                        if !suns.is_empty() {
                            let j = suns[self.rng.below(suns.len() as u32) as usize];
                            s.target = Target::Obj(j);
                            s.mode = Mode::Fly;
                            s.home = NO_SUN;
                            s.hold = false;
                            s.guard = 0;
                        }
                    }
                }
                TerrainKind::Neutral => s.neutral = true,
            }
        }
        fx
    }

    pub(crate) fn is_wall(&self, p: Vec2) -> bool {
        self.terrain_idx.iter().any(|&i| matches!(&self.objs[i], crate::Obj::Terrain(t) if t.def.kind == TerrainKind::Reflect && t.contains(p)))
    }
}
