//! Deterministic AI. It only ever acts through [`Sim::issue`], exactly like a human, and draws randomness only from
//! the sim's seeded RNG. Difficulty 0..3 changes how often it thinks, how much it over-commits ("wastes") and how
//! cautious its attack estimates are.

use crate::math::Vec2;
use crate::rules::*;
use crate::{Command, Obj, Sim, Target};
use au2_map::TerrainKind;

struct Style {
    /// Over-commit multiplier on the ships needed for a capture (easy AIs waste ships).
    waste: f64,
    /// Required superiority before attacking an enemy sun.
    attack_margin: f64,
    max_attacks: usize,
    max_captures: usize,
    /// Avoid minefield crossings.
    avoids_mines: bool,
    uses_wormholes: bool,
}

fn style(d: u8) -> Style {
    match d.min(3) {
        0 => Style { waste: 2.5, attack_margin: 2.0, max_attacks: 1, max_captures: 1, avoids_mines: false, uses_wormholes: false },
        1 => Style { waste: 1.8, attack_margin: 1.6, max_attacks: 1, max_captures: 2, avoids_mines: true, uses_wormholes: false },
        2 => Style { waste: 1.3, attack_margin: 1.3, max_attacks: 2, max_captures: 3, avoids_mines: true, uses_wormholes: true },
        _ => Style { waste: 1.1, attack_margin: 1.15, max_attacks: 3, max_captures: 4, avoids_mines: true, uses_wormholes: true },
    }
}

/// Extra ships to send when the straight path has `samples` mine-field samples on it (each ~0.4 s of exposure).
fn mine_factor(samples: usize) -> f64 {
    (1.0 + 0.08 * samples as f64).min(4.0)
}

/// How far an enemy swarm must be to count as no threat to a sun.
const THREAT_RADIUS: f64 = 1800.0;

impl Sim {
    pub(crate) fn run_ai(&mut self) {
        for f in 0..self.factions.len() {
            if !self.is_ai(f) || !self.factions[f].alive || self.tick < self.factions[f].next_think {
                continue;
            }
            let d = self.cfg.ai_difficulty.min(3);
            let base = if self.factions[f].think_period > 0 { self.factions[f].think_period as u64 } else { AI_THINK_TICKS[d as usize] };
            let jitter = self.rng.below((base / 4).max(1) as u32) as u64;
            self.factions[f].next_think = self.tick + base + jitter;
            self.ai_think(f as u8, style(d));
        }
    }

    fn enemy_ships_near(&self, f: u8, p: Vec2, r: f64) -> usize {
        let r2 = r * r;
        self.ships.iter().filter(|s| s.alive && s.faction != f && s.pos.dist2(p) <= r2).count()
    }

    /// Ships of faction `f` currently heading for object `j`.
    fn inflight_to(&self, f: u8, j: usize) -> i32 {
        self.ships.iter().filter(|s| s.alive && s.faction == f && s.target == Target::Obj(j)).count() as i32
    }

    /// Number of mine-field samples on the straight line a -> b (0 = clear).
    fn mine_samples(&self, a: Vec2, b: Vec2) -> usize {
        let steps = ((a.dist(b) / 250.0) as usize).max(1);
        let mut n = 0;
        for k in 0..=steps {
            let t = k as f64 / steps as f64;
            let p = a + (b - a) * t;
            for &i in &self.terrain_idx {
                if let Obj::Terrain(t) = &self.objs[i] {
                    if t.def.kind == TerrainKind::Minefield && t.contains(p) {
                        n += 1;
                        break;
                    }
                }
            }
        }
        n
    }

    /// Wormhole entrance worth routing through for a flight `a` -> `b`, if any.
    fn wormhole_route(&self, a: Vec2, b: Vec2) -> Option<usize> {
        let direct = a.dist(b);
        let mut best: Option<(f64, usize)> = None;
        for &wi in &self.wormholes {
            let Obj::Wormhole(w) = &self.objs[wi] else { continue };
            if w.dest < 0 {
                continue;
            }
            let Some(Obj::Wormhole(exit)) = self.objs.get(w.dest as usize) else { continue };
            let via = a.dist(w.pos) + exit.pos.dist(b);
            if via < direct * 0.7 && best.map(|(bv, _)| via < bv).unwrap_or(true) {
                best = Some((via, wi));
            }
        }
        best.map(|(_, i)| i)
    }

    fn send(&mut self, f: u8, sources: Vec<usize>, target: usize, percent: u8, st: &Style) -> bool {
        let tpos = match self.obj_pos(target) {
            Some(p) => p,
            None => return false,
        };
        let first = self.obj_pos(sources[0]).unwrap_or(tpos);
        let mut aim = Target::Obj(target);
        let mut then = None;
        if st.uses_wormholes {
            if let Some(w) = self.wormhole_route(first, tpos) {
                aim = Target::Obj(w);
                then = Some(Target::Obj(target));
            }
        }
        self.issue(Command::Order { faction: f, sources, target: aim, then, percent: percent.clamp(1, 100), repeat: false }).is_ok()
    }

    fn ai_think(&mut self, f: u8, st: Style) {
        // snapshot of own suns
        let mine: Vec<usize> = self.suns().filter(|(_, s)| s.faction == f as i8 && s.present()).map(|(i, _)| i).collect();
        if mine.is_empty() {
            return;
        }
        // threat per own sun
        let mut threat = vec![0usize; self.objs.len()];
        for &i in &mine {
            let p = self.sun(i).unwrap().pos;
            threat[i] = self.enemy_ships_near(f, p, THREAT_RADIUS);
        }

        // --- 1. defend: threatened suns call for help
        for &i in &mine {
            let (pos, stock) = {
                let s = self.sun(i).unwrap();
                (s.pos, s.stock as usize)
            };
            if threat[i] > 0 && threat[i] * 10 > stock * 7 {
                let mut helpers: Vec<(f64, usize)> = mine
                    .iter()
                    .filter(|&&h| h != i && threat[h] == 0 && self.sun(h).unwrap().stock >= 10)
                    .map(|&h| (self.sun(h).unwrap().pos.dist2(pos), h))
                    .collect();
                helpers.sort_by(|a, b| a.partial_cmp(b).unwrap());
                if let Some(&(_, h)) = helpers.first() {
                    self.send(f, vec![h], i, 60, &st);
                    break;
                }
            }
        }

        // --- 2. upgrade safe suns
        for &i in &mine {
            let (lvl, max, prog, stock) = {
                let s = self.sun(i).unwrap();
                (s.level, s.max_level, s.progress, s.stock as i32)
            };
            if threat[i] == 0 && lvl < max {
                let mut need = UPGRADE_COST[lvl as usize] - prog - self.inflight_to(f, i);
                if need > 0 && stock > 0 {
                    let give = need.min(stock);
                    let pct = ((give as i64 * 100 + stock as i64 - 1) / stock as i64) as u8;
                    if self.issue(Command::Order { faction: f, sources: vec![i], target: Target::Obj(i), then: None, percent: pct.clamp(1, 100), repeat: false }).is_ok() {
                        need -= give;
                    }
                }
                // an under-developed sun with little stock of its own is fed from a neighbour's surplus
                if need > 0 {
                    let ipos = self.sun(i).unwrap().pos;
                    let helper = mine
                        .iter()
                        .filter(|&&h| h != i && threat[h] == 0)
                        .filter(|&&h| self.sun(h).unwrap().stock as i32 >= need + 20)
                        .map(|&h| (self.sun(h).unwrap().pos.dist2(ipos), h))
                        .fold(None, |best: Option<(f64, usize)>, c| match best {
                            Some(b) if b.0 <= c.0 => Some(b),
                            _ => Some(c),
                        });
                    if let Some((_, h)) = helper {
                        let hs = self.sun(h).unwrap().stock as i64;
                        let pct = ((need as i64 * 100 + hs - 1) / hs).clamp(1, 100) as u8;
                        self.send(f, vec![h], i, pct, &st);
                    }
                }
            }
        }

        // --- 3. capture nearby neutral suns
        let mut caps: Vec<(f64, usize, usize)> = Vec::new(); // (cost metric, source, target)
        let neutrals: Vec<usize> = self.suns().filter(|(_, s)| s.faction < 0 && s.present()).map(|(i, _)| i).collect();
        for &t in &neutrals {
            let tp = self.sun(t).unwrap().pos;
            let already = self.inflight_to(f, t);
            let sunref = self.sun(t).unwrap();
            let have = if sunref.cap_faction == f as i8 { sunref.cap_progress } else { 0 };
            let need = CAPTURE_COST - have - already;
            if need <= 0 {
                continue;
            }
            // nearest own sun that is safe and can afford it (mines on the way cost extra ships)
            let mut best: Option<(f64, usize)> = None;
            for &m in &mine {
                let ms = self.sun(m).unwrap();
                let mines = if st.avoids_mines { self.mine_samples(ms.pos, tp) } else { 0 };
                if threat[m] > 0 || (ms.stock as i32) < (need as f64 * mine_factor(mines)).ceil() as i32 {
                    continue;
                }
                let mut d = ms.pos.dist(tp);
                if mines > 1 {
                    d *= 4.0;
                }
                if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, m));
                }
            }
            if let Some((d, m)) = best {
                caps.push((d, m, t));
            }
        }
        caps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut done = 0;
        for (_, m, t) in caps {
            if done >= st.max_captures {
                break;
            }
            let (stock, tp) = (self.sun(m).unwrap().stock as i32, self.sun(t).unwrap().pos);
            let sunref = self.sun(t).unwrap();
            let have = if sunref.cap_faction == f as i8 { sunref.cap_progress } else { 0 };
            let need = CAPTURE_COST - have - self.inflight_to(f, t);
            if need <= 0 || stock < need {
                continue;
            }
            let mines = if st.avoids_mines { self.mine_samples(self.sun(m).unwrap().pos, tp) } else { 0 };
            let want = ((need as f64 * st.waste * mine_factor(mines)).ceil() as i32 + 1).min(stock);
            let pct = ((want as i64 * 100 + stock as i64 - 1) / stock as i64) as u8;
            if self.send(f, vec![m], t, pct, &st) {
                done += 1;
            }
        }

        // --- 4. attack beatable enemy suns
        let enemies: Vec<usize> = self.suns().filter(|(_, s)| s.faction >= 0 && s.faction != f as i8 && s.present()).map(|(i, _)| i).collect();
        let mut attacks = 0;
        let mut scored: Vec<(f64, usize)> = Vec::new();
        for &e in &enemies {
            let es = self.sun(e).unwrap();
            let mut hp = es.progress.max(0);
            for l in 1..=es.level as usize {
                hp += UPGRADE_COST[l - 1] / 2;
            }
            let defenders = self.enemy_ships_near(f, es.pos, 1200.0) as i32;
            let need = es.stock as i32 + hp / 2 + CAPTURE_COST + 2 + defenders - self.inflight_to(f, e);
            let nearest = mine.iter().map(|&m| self.sun(m).unwrap().pos.dist(es.pos)).fold(f64::MAX, f64::min);
            scored.push((nearest + need as f64 * 20.0, e));
        }
        scored.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for (_, e) in scored {
            if attacks >= st.max_attacks {
                break;
            }
            let (epos, estock, elevel, eprog) = {
                let es = self.sun(e).unwrap();
                (es.pos, es.stock as i32, es.level as usize, es.progress.max(0))
            };
            let mut hp = eprog;
            for l in 1..=elevel {
                hp += UPGRADE_COST[l - 1] / 2;
            }
            let defenders = self.enemy_ships_near(f, epos, 1200.0) as i32;
            let need = estock + hp / 2 + CAPTURE_COST + 2 + defenders - self.inflight_to(f, e);
            if need <= 0 {
                continue;
            }
            // impatience: after 5 minutes the AI gradually accepts closer odds so mirror matches do not stall
            let patience = ((self.tick as f64 - 18_000.0) / 36_000.0).clamp(0.0, 1.0);
            let margin = st.attack_margin - (st.attack_margin - 0.9) * patience;
            let target_need = (need as f64 * margin).ceil() as i64;
            // gather sources, nearest first, that are not themselves threatened
            let mut cand: Vec<(f64, usize)> = mine
                .iter()
                .filter(|&&m| threat[m] == 0 && self.sun(m).map(|s| s.faction == f as i8 && s.stock >= 5).unwrap_or(false))
                .map(|&m| (self.sun(m).unwrap().pos.dist2(epos), m))
                .collect();
            cand.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let Some(&(_, nearest_src)) = cand.first() else { continue };
            let mines = if st.avoids_mines { self.mine_samples(self.sun(nearest_src).unwrap().pos, epos) } else { 0 };
            let target_need = (target_need as f64 * mine_factor(mines)).ceil() as i64;
            let mut chosen = Vec::new();
            let mut total: i64 = 0;
            for (_, m) in cand {
                if chosen.len() >= self.factions[f as usize].max_sources as usize {
                    break;
                }
                total += self.sun(m).unwrap().stock as i64;
                chosen.push(m);
                if total >= target_need {
                    break;
                }
            }
            if total < target_need || chosen.is_empty() {
                continue;
            }
            let pct = ((target_need * 100 + total - 1) / total).clamp(1, 100) as u8;
            if self.send(f, chosen, e, pct, &st) {
                attacks += 1;
            }
        }
    }
}
