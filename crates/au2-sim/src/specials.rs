//! Special objects: orbiting suns and wormholes (positions), fragile suns (break / rebuild), wormholes (jumps),
//! white holes and supernovas (investment tug of war), gamma-ray lanes and shockwaves.

use crate::math::{dir_deg, Rng, Vec2};
use crate::rules::*;
use crate::{has_orbit, orbit_pos, v2, Event, FragilePhase, Mode, Obj, Ship, Sim, Target, NO_SUN};
use au2_map::{GammaRayDef, OrbitDef, ShockwaveDef, SupernovaDef, WhiteHoleDef, WormholeDef};

#[derive(Debug, Clone, PartialEq)]
pub struct Wormhole {
    pub pos: Vec2,
    pub centre: Vec2,
    pub orbit: Option<OrbitDef>,
    pub size: f64,
    pub dest: i32,
}

impl Wormhole {
    pub(crate) fn new(w: &WormholeDef) -> Wormhole {
        let centre = v2(w.pos);
        let orbit = has_orbit(&w.orbit).then(|| w.orbit.clone());
        let pos = orbit.as_ref().map(|or| orbit_pos(centre, or, 0)).unwrap_or(centre);
        Wormhole { pos, centre, orbit, size: w.size.max(50.0), dest: w.destination }
    }
}

/// Investment tug of war shared by white holes and supernovas: a rival's ships first remove the current investor's
/// stake, then start their own. Returns the strength not used (the stake is capped at `required`).
fn tug(investor: &mut i8, invest: &mut u32, required: u32, faction: u8, strength: u32) -> u32 {
    let mut left = strength;
    if *investor >= 0 && *investor != faction as i8 {
        let k = left.min(*invest);
        *invest -= k;
        left -= k;
        if *invest == 0 {
            *investor = -1;
        }
    }
    if left > 0 {
        *investor = faction as i8;
        let k = left.min(required.saturating_sub(*invest));
        *invest += k;
        left -= k;
    }
    left
}

// ------------------------------------------------------------------------------------------------ white hole
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhitePhase {
    /// Accepting investment; the prize grows in steps since `since`.
    Growing { since: u64 },
    /// Streaming `total` ships' worth out of one side for the winner.
    Firing { faction: u8, from: u64, total: u32, dir: Vec2, sent: u32 },
    Spent { until: u64 },
    Regrow { from: u64, until: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhiteHole {
    pub def: WhiteHoleDef,
    pub pos: Vec2,
    pub investor: i8,
    pub invest: u32,
    pub phase: WhitePhase,
    /// Visible size, 0..1 (swells in steps while the prize grows).
    pub size: f64,
}

impl WhiteHole {
    pub(crate) fn new(d: &WhiteHoleDef) -> WhiteHole {
        WhiteHole { def: d.clone(), pos: v2(d.pos), investor: -1, invest: 0, phase: WhitePhase::Growing { since: 0 }, size: 0.4 }
    }
    pub fn required(&self) -> u32 {
        self.def.required_investment.max(1)
    }
    /// Current prize step 0..=WHITE_STEPS.
    pub fn step(&self, tick: u64) -> u32 {
        match self.phase {
            WhitePhase::Growing { since } => ((tick.saturating_sub(since) * WHITE_STEPS as u64 / WHITE_GROW_TICKS) as u32).min(WHITE_STEPS),
            _ => 0,
        }
    }
    /// Ships the hole would release if it fired now.
    pub fn prize(&self, tick: u64) -> u32 {
        let (a, b) = (self.def.start_return, self.def.end_return.max(self.def.start_return));
        a + (b - a) * self.step(tick) / WHITE_STEPS
    }
    pub fn accepting(&self) -> bool {
        matches!(self.phase, WhitePhase::Growing { .. })
    }
    pub fn radius(&self) -> f64 {
        (self.def.game_size.max(120.0)) * self.size
    }
}

// ------------------------------------------------------------------------------------------------ supernova
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NovaPhase {
    Ready,
    /// Pulling ships in; explodes at `until`.
    Collapse { faction: u8, until: u64 },
    /// Blast front expanding since `from`.
    Blast { faction: u8, from: u64 },
    Dormant { until: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Supernova {
    pub def: SupernovaDef,
    pub pos: Vec2,
    pub investor: i8,
    pub invest: u32,
    pub phase: NovaPhase,
    pub last_blast: Option<u64>,
}

impl Supernova {
    pub(crate) fn new(d: &SupernovaDef) -> Supernova {
        Supernova { def: d.clone(), pos: v2(d.pos), investor: -1, invest: 0, phase: NovaPhase::Ready, last_blast: None }
    }
    pub fn required(&self) -> u32 {
        self.def.required_investment.max(1)
    }
    /// Drawn / reach radius: grows with investment (`startscale` .. `endscale` are diameters).
    pub fn radius(&self) -> f64 {
        let (a, b) = (self.def.start_scale.max(200.0) / 2.0, self.def.end_scale.max(self.def.start_scale.max(200.0)) / 2.0);
        let k = match self.phase {
            NovaPhase::Ready => self.invest as f64 / self.required() as f64,
            NovaPhase::Collapse { .. } => 1.0,
            _ => 0.0,
        };
        a + (b - a) * k
    }
    /// Blast front radius at `tick` (None when not exploding).
    pub fn front(&self, tick: u64) -> Option<f64> {
        match self.phase {
            NovaPhase::Blast { from, .. } => Some(self.def.blast_radius * ((tick - from) as f64 / NOVA_BLAST_TICKS as f64).min(1.0)),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------------------------------ gamma ray
#[derive(Debug, Clone, PartialEq)]
pub struct Gamma {
    pub def: GammaRayDef,
    pub pos: Vec2,
    /// Unit direction along the lane (the pulse sweeps this way).
    pub dir: Vec2,
    /// Tick at which the next pulse starts sweeping (the warning starts `GAMMA_WARN_TICKS` earlier).
    pub next_pulse: u64,
}

impl Gamma {
    pub(crate) fn new(d: &GammaRayDef, rng: &mut Rng) -> Gamma {
        let i = Self::interval_of(d);
        let first = if d.random_start { i / 4 + rng.below((i * 3 / 4).max(1) as u32) as u64 } else { i };
        Gamma { def: d.clone(), pos: v2(d.pos), dir: dir_deg(d.rotation_deg), next_pulse: first.max(GAMMA_WARN_TICKS + 1) }
    }
    fn interval_of(d: &GammaRayDef) -> u64 {
        d.period.max(1) as u64 * GAMMA_TICKS_PER_PERIOD as u64
    }
    pub fn interval(&self) -> u64 {
        Self::interval_of(&self.def)
    }
    pub fn warning(&self, tick: u64) -> bool {
        tick + GAMMA_WARN_TICKS >= self.next_pulse && tick < self.next_pulse
    }
    /// Position of the sweeping pulse along the lane (-L..L) at `tick`, if sweeping.
    pub fn front(&self, tick: u64) -> Option<f64> {
        if tick < self.next_pulse || tick > self.next_pulse + GAMMA_SWEEP_TICKS {
            return None;
        }
        Some(-GAMMA_HALF_LENGTH + 2.0 * GAMMA_HALF_LENGTH * (tick - self.next_pulse) as f64 / GAMMA_SWEEP_TICKS as f64)
    }
    /// (along, across) coordinates of `p` relative to the lane.
    pub fn lane_coords(&self, p: Vec2) -> (f64, f64) {
        let rel = p - self.pos;
        (rel.dot(self.dir), rel.cross(self.dir).abs())
    }
    /// Is `p` (with radius `extra`) inside the lane?
    pub fn in_lane(&self, p: Vec2, extra: f64) -> bool {
        let (along, across) = self.lane_coords(p);
        across < self.def.width / 2.0 + extra && along.abs() <= GAMMA_HALF_LENGTH
    }
}

// ------------------------------------------------------------------------------------------------ shockwave
#[derive(Debug, Clone, PartialEq)]
pub struct Shock {
    pub def: ShockwaveDef,
    pub pos: Vec2,
}

/// Shape of a shockwave wall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShockShape {
    /// Ring of this radius around the shock's position.
    Ring(f64),
    /// Straight wall: points with `(p - pos) . normal == offset`.
    Line { normal: Vec2, offset: f64 },
}

impl Shock {
    pub fn speed(&self) -> f64 {
        if self.def.speed > 0.0 {
            self.def.speed / TICKS_PER_SEC as f64
        } else {
            SHOCK_DEFAULT_SPEED
        }
    }
    /// The wall at `tick`, if one is active.
    pub fn wall(&self, tick: u64) -> Option<ShockShape> {
        let d = &self.def;
        if tick < d.dormant as u64 {
            return None;
        }
        let since = tick - d.dormant as u64;
        let age = if d.period > 0 { since % d.period as u64 } else { since };
        let travel = 24_000.0;
        let life = if d.lifetime > 0 { d.lifetime as u64 } else { (travel / self.speed()) as u64 };
        if age >= life {
            return None;
        }
        let dist = self.speed() * age as f64;
        let kind = d.kind.to_ascii_lowercase();
        if kind.contains("in") && kind.contains("circle") {
            let r0 = if d.start_offset > 0.0 { d.start_offset } else { 12_000.0 };
            let r = r0 - dist;
            (r > 0.0).then_some(ShockShape::Ring(r))
        } else if kind.contains("circle") {
            Some(ShockShape::Ring(d.start_offset.max(0.0) + dist))
        } else {
            Some(ShockShape::Line { normal: dir_deg(d.line_rotation_deg + 90.0), offset: -12_000.0 + d.start_offset + dist })
        }
    }
    /// Is `p` inside the wall at `tick`?
    pub fn hits(&self, p: Vec2, tick: u64) -> bool {
        match self.wall(tick) {
            Some(ShockShape::Ring(r)) => (p.dist(self.pos) - r).abs() < SHOCK_THICKNESS / 2.0,
            Some(ShockShape::Line { normal, offset }) => ((p - self.pos).dot(normal) - offset).abs() < SHOCK_THICKNESS / 2.0,
            None => false,
        }
    }
}

// ------------------------------------------------------------------------------------------------ the sim side
impl Sim {
    /// Positions, timers and state machines of the special objects (start of the tick).
    pub(crate) fn update_objects(&mut self) {
        let tick = self.tick;
        for i in 0..self.objs.len() {
            let mut emit: Option<(u8, Vec2, Vec2, u32, u32)> = None; // white hole: faction, pos, dir, dots, strength
            let mut nova: Option<(u8, Vec2, u32)> = None;
            match &mut self.objs[i] {
                Obj::Sun(s) => {
                    if let Some(or) = &s.orbit {
                        s.pos = orbit_pos(s.centre, or, tick);
                    }
                    if let Some(f) = &mut s.fragile {
                        match f.phase {
                            FragilePhase::Broken { until } if tick >= until => {
                                f.phase = FragilePhase::Rebuilding { from: tick, until: tick + f.rebuild_ticks as u64 };
                            }
                            FragilePhase::Rebuilding { until, .. } if tick >= until => {
                                f.phase = FragilePhase::Intact;
                                self.events.push(Event::SunRebuilt { sun: i });
                            }
                            _ => {}
                        }
                    }
                }
                Obj::Wormhole(w) => {
                    if let Some(or) = &w.orbit {
                        w.pos = orbit_pos(w.centre, or, tick);
                    }
                }
                Obj::WhiteHole(w) => {
                    match w.phase {
                        WhitePhase::Growing { .. } => {
                            w.size = 0.55 + 0.45 * w.step(tick) as f64 / WHITE_STEPS as f64;
                        }
                        WhitePhase::Firing { faction, from, total, dir, sent } => {
                            let strength = 1 + total / WHITE_STRONG_ABOVE.max(1);
                            let dots = total.div_ceil(strength);
                            let due = ((dots as u64 * (tick - from + 1)) / WHITE_FIRE_TICKS).min(dots as u64) as u32;
                            if due > sent {
                                emit = Some((faction, w.pos, dir, due - sent, strength));
                                w.phase = WhitePhase::Firing { faction, from, total, dir, sent: due };
                            }
                            if due >= dots {
                                let spent = (w.def.dormant_period as u64 * NOVA_TICKS_PER_UNIT).max(WHITE_MIN_SPENT);
                                w.phase = WhitePhase::Spent { until: tick + spent };
                                w.investor = -1;
                                w.invest = 0;
                            }
                            w.size = 0.55 * (1.0 - (tick - from) as f64 / WHITE_FIRE_TICKS as f64).max(0.0) + 0.15;
                        }
                        WhitePhase::Spent { until } => {
                            w.size = 0.15;
                            if tick >= until {
                                w.phase = WhitePhase::Regrow { from: tick, until: tick + WHITE_REGROW_TICKS };
                            }
                        }
                        WhitePhase::Regrow { from, until } => {
                            w.size = 0.15 + 0.4 * (tick - from) as f64 / (until - from).max(1) as f64;
                            if tick >= until {
                                w.phase = WhitePhase::Growing { since: tick };
                            }
                        }
                    }
                }
                Obj::Supernova(n) => match n.phase {
                    NovaPhase::Collapse { faction, until } if tick >= until => {
                        n.phase = NovaPhase::Blast { faction, from: tick };
                        n.last_blast = Some(tick);
                        n.investor = -1;
                        n.invest = 0;
                        nova = Some((faction, n.pos, n.def.unit_residuals));
                    }
                    NovaPhase::Blast { from, .. } if tick >= from + NOVA_BLAST_TICKS => {
                        n.phase = NovaPhase::Dormant { until: tick + n.def.dormant_period as u64 * NOVA_TICKS_PER_UNIT };
                    }
                    NovaPhase::Dormant { until } if tick >= until => {
                        n.phase = NovaPhase::Ready;
                    }
                    _ => {}
                },
                Obj::Gamma(g) => {
                    if tick + GAMMA_WARN_TICKS == g.next_pulse {
                        self.events.push(Event::GammaWarning { obj: i });
                    }
                    if tick == g.next_pulse {
                        self.events.push(Event::GammaPulse { obj: i });
                    }
                }
                Obj::Terrain(t) => t.tick_switch(tick),
                _ => {}
            }
            if let Some((faction, pos, dir, n, strength)) = emit {
                // a fan out of one side
                let r = self.white_radius(i);
                for _ in 0..n {
                    let h = dir.rotate(self.rng.signed_unit() * 35.0);
                    let mut s = self.new_ship(faction, pos + h * r, Target::None);
                    let reach = 450.0 + self.rng.unit() * 500.0;
                    s.target = Target::Point(pos + h * (r + reach));
                    s.mode = Mode::Fly;
                    s.heading = h;
                    s.strength = strength;
                    self.spawn(s);
                }
            }
            if let Some((faction, pos, n)) = nova {
                self.events.push(Event::Supernova { obj: i, faction });
                for _ in 0..n {
                    let p = pos + dir_deg(self.rng.unit() * 360.0) * (200.0 * self.rng.unit().sqrt());
                    let mut s = self.new_ship(faction, p, Target::None);
                    s.mode = Mode::Hover;
                    self.spawn(s);
                }
            }
        }
    }

    fn white_radius(&self, i: usize) -> f64 {
        match &self.objs[i] {
            Obj::WhiteHole(w) => w.radius(),
            _ => 0.0,
        }
    }

    /// A ship of `faction` with `strength` reaches a white hole or supernova. Returns unused strength.
    pub(crate) fn invest(&mut self, obj: usize, faction: u8, strength: u32) -> u32 {
        let tick = self.tick;
        let mut fired = None;
        let left = match &mut self.objs[obj] {
            Obj::WhiteHole(w) if w.accepting() => {
                let req = w.required();
                let left = tug(&mut w.investor, &mut w.invest, req, faction, strength);
                if w.investor == faction as i8 && w.invest >= req {
                    fired = Some(w.prize(tick));
                }
                left
            }
            Obj::Supernova(n) if n.phase == NovaPhase::Ready => {
                let req = n.required();
                let left = tug(&mut n.investor, &mut n.invest, req, faction, strength);
                if n.investor == faction as i8 && n.invest >= req {
                    n.phase = NovaPhase::Collapse { faction, until: tick + NOVA_COLLAPSE_TICKS };
                    self.events.push(Event::SupernovaCollapse { obj, faction });
                }
                left
            }
            _ => strength,
        };
        if let Some(total) = fired {
            let pos = self.obj_pos(obj).unwrap();
            let dir = self.nearest_own_sun(faction, pos).and_then(|j| self.obj_pos(j)).map(|p| (p - pos).norm_or(Vec2::new(1.0, 0.0))).unwrap_or(Vec2::new(1.0, 0.0));
            if let Obj::WhiteHole(w) = &mut self.objs[obj] {
                w.phase = WhitePhase::Firing { faction, from: tick, total, dir, sent: 0 };
            }
            self.events.push(Event::WhiteHole { obj, faction, ships: total });
        }
        left
    }

    /// A ship ordered to wormhole `w` enters it: it comes out of the destination keeping speed and heading, idle.
    pub(crate) fn jump(&mut self, s: &mut Ship, w: usize) {
        let k = self.wormholes.iter().position(|&x| x == w).unwrap_or(0);
        let dest = match &self.objs[w] {
            Obj::Wormhole(wh) => wh.dest,
            _ => return,
        };
        let di = if dest >= 0 && matches!(self.objs.get(dest as usize), Some(Obj::Wormhole(_))) {
            dest as usize
        } else if self.wormholes.len() < 2 {
            self.make_idle(s);
            return;
        } else {
            let n = self.wormholes.len() as u32;
            let mut pick = self.rng.below(n - 1) as usize;
            if pick >= k {
                pick += 1;
            }
            self.wormholes[pick]
        };
        if let Some(Obj::Wormhole(d)) = self.objs.get(di) {
            s.pos = d.pos + s.heading * (d.size + WORMHOLE_EXIT_MARGIN);
        }
        self.events.push(Event::WormholeJump { obj: w });
        self.make_idle(s);
    }

    /// Lethal and pushing hazards, after movement: gamma pulses, supernova collapse / blast, shockwaves.
    pub(crate) fn hazards(&mut self) {
        let tick = self.tick;
        for oi in 0..self.objs.len() {
            match &self.objs[oi] {
                Obj::Gamma(g) => {
                    let Some(front) = g.front(tick) else { continue };
                    let prev = if tick == g.next_pulse { f64::NEG_INFINITY } else { g.front(tick - 1).unwrap_or(f64::NEG_INFINITY) };
                    let g = g.clone();
                    let swept = |p: Vec2, extra: f64| {
                        let (along, _) = g.lane_coords(p);
                        g.in_lane(p, extra) && along > prev && along <= front
                    };
                    if g.def.kill_suns {
                        let hit: Vec<usize> = self.suns().filter(|(_, s)| s.present() && swept(s.pos, s.radius)).map(|(j, _)| j).collect();
                        for j in hit {
                            self.kill_sun(j);
                        }
                    }
                    if g.def.kill_units {
                        for s in self.ships.iter_mut() {
                            if s.alive && swept(s.pos, 0.0) {
                                s.alive = false;
                            }
                        }
                    }
                    if tick >= g.next_pulse + GAMMA_SWEEP_TICKS {
                        let i = g.interval();
                        let jit = (i as f64 * g.def.randomization.max(0.0) / 100.0 * self.rng.signed_unit()) as i64;
                        let next = (tick as i64 + i as i64 + jit).max((tick + i / 3 + GAMMA_WARN_TICKS) as i64) as u64;
                        if let Obj::Gamma(gm) = &mut self.objs[oi] {
                            gm.next_pulse = next;
                        }
                    }
                }
                Obj::Supernova(n) => match n.phase {
                    NovaPhase::Collapse { .. } => {
                        let (c, r2) = (n.pos, n.def.collapse_radius * n.def.collapse_radius);
                        for s in self.ships.iter_mut() {
                            if s.alive && s.pos.dist2(c) <= r2 && s.pos.dist2(c) > 100.0 {
                                s.pos = s.pos + (c - s.pos).norm_or(Vec2::ZERO) * NOVA_PULL;
                            }
                        }
                    }
                    NovaPhase::Blast { faction, from } => {
                        let n = n.clone();
                        let front = n.front(tick).unwrap_or(0.0);
                        let prev = if tick == from { -1.0 } else { n.front(tick - 1).unwrap_or(0.0) };
                        let c = n.pos;
                        for s in self.ships.iter_mut() {
                            if !s.alive {
                                continue;
                            }
                            let d = s.pos.dist(c);
                            if d <= prev || d > front {
                                continue;
                            }
                            if n.def.kill_enemy_units && s.faction != faction {
                                s.alive = false;
                                continue;
                            }
                            s.push += (s.pos - c).norm_or(Vec2::new(1.0, 0.0)) * NOVA_SHOVE;
                            s.target = Target::None;
                            s.mode = Mode::Hover;
                            s.home = NO_SUN;
                            s.hold = false;
                            s.guard = 0;
                        }
                        if n.def.kill_enemy_suns {
                            let hit: Vec<usize> = self
                                .suns()
                                .filter(|(_, s)| s.present() && s.faction >= 0 && s.faction != faction as i8)
                                .filter(|(_, s)| {
                                    let d = s.pos.dist(c);
                                    d > prev && d <= front
                                })
                                .map(|(j, _)| j)
                                .collect();
                            for j in hit {
                                self.kill_sun(j);
                            }
                        }
                    }
                    _ => {}
                },
                Obj::Shock(sh) => {
                    if sh.wall(tick).is_none() {
                        continue;
                    }
                    let sh = sh.clone();
                    for s in self.ships.iter_mut() {
                        if s.alive && sh.hits(s.pos, tick) {
                            s.alive = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
