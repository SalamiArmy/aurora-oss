//! Deterministic AI. It only ever acts through [`Sim::issue`], exactly like a human, and draws randomness only from
//! the sim's seeded RNG. It plans every several seconds and makes only a few, large moves per plan; difficulty 0..3
//! changes how often it plans, how many moves it makes, how many suns it gathers ships from and how much it wastes.

use crate::math::Vec2;
use crate::rules::*;
use crate::{production_per_sec, Command, Mode, NovaPhase, Obj, Sim, Target};
use au2_map::TerrainKind;

struct Style {
    /// Moves per plan.
    moves: usize,
    /// Suns a single move may gather ships from.
    sources: usize,
    /// Over-commit multiplier on what a move needs (easy AIs waste ships).
    waste: f64,
    /// Required superiority before attacking an enemy sun (scaled by relative strength).
    margin: f64,
    avoids_hazards: bool,
    uses_wormholes: bool,
    evacuates: bool,
}

fn style(d: u8) -> Style {
    match d.min(3) {
        0 => Style { moves: 1, sources: 1, waste: 1.5, margin: 1.7, avoids_hazards: false, uses_wormholes: false, evacuates: false },
        1 => Style { moves: 2, sources: 2, waste: 1.3, margin: 1.45, avoids_hazards: true, uses_wormholes: false, evacuates: true },
        2 => Style { moves: 2, sources: 3, waste: 1.15, margin: 1.3, avoids_hazards: true, uses_wormholes: true, evacuates: true },
        _ => Style { moves: 3, sources: 4, waste: 1.1, margin: 1.2, avoids_hazards: true, uses_wormholes: true, evacuates: true },
    }
}

/// A candidate move: send `amount` strength from `sources` (own suns, or loose ships) to `target`.
struct Move {
    score: f64,
    sources: Vec<usize>,
    loose: bool,
    target: usize,
    /// Aim at this point instead of `target` (hunting stray stacks).
    point: Option<Vec2>,
    amount: u32,
}

/// What the AI knows about the board at the start of a plan.
struct View {
    mine: Vec<usize>,
    /// Ship indices of idle, non-defending own ships circling each sun (by object index).
    pool: Vec<Vec<usize>>,
    avail: Vec<u32>,
    /// Own idle ships hovering in space.
    loose: Vec<usize>,
    loose_strength: u32,
    /// Enemy strength attacking (in flight to) or sitting around each sun.
    threat: Vec<u32>,
    /// Own strength already flying to each object.
    inflight: Vec<u32>,
    power: Vec<u32>,
}

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
            self.factions[f].plans += 1;
            self.ai_plan(f as u8, style(d));
        }
    }

    fn ai_view(&self, f: u8) -> View {
        let n = self.objs.len();
        let mine: Vec<usize> = self.suns().filter(|(_, s)| s.faction == f as i8 && s.present()).map(|(i, _)| i).collect();
        let mut pool = vec![Vec::new(); n];
        let mut avail = vec![0u32; n];
        let mut loose = Vec::new();
        let mut loose_strength = 0;
        let mut threat = vec![0u32; n];
        let mut inflight = vec![0u32; n];
        let mut power = vec![0u32; self.factions.len()];
        // "around a sun" = within its cloud plus a margin
        let near: Vec<(usize, Vec2, f64)> = self.suns().filter(|(_, s)| s.present()).map(|(i, s)| (i, s.pos, s.cloud_radius() + 300.0)).collect();
        for (k, s) in self.ships.iter().enumerate() {
            if !s.alive {
                continue;
            }
            power[s.faction as usize] += s.strength;
            if s.faction == f {
                if s.idle() && s.guard == 0 {
                    if s.mode == Mode::Orbit {
                        pool[s.home as usize].push(k);
                        avail[s.home as usize] += s.strength;
                    } else {
                        loose.push(k);
                        loose_strength += s.strength;
                    }
                } else if let Target::Obj(t) = s.target {
                    inflight[t] += s.strength;
                }
            } else if !s.neutral {
                if let Target::Obj(t) = s.target {
                    if self.sun(t).is_some_and(|x| x.faction == f as i8) {
                        threat[t] += s.strength;
                        continue;
                    }
                }
                for &(i, p, r) in &near {
                    if s.pos.dist2(p) <= r * r {
                        threat[i] += s.strength;
                        break;
                    }
                }
            }
        }
        View { mine, pool, avail, loose, loose_strength, threat, inflight, power }
    }

    /// Number of minefield samples (with most mines left) on the straight line a -> b.
    fn mine_samples(&self, a: Vec2, b: Vec2) -> usize {
        let steps = ((a.dist(b) / 250.0) as usize).max(1);
        let mut n = 0;
        for k in 0..=steps {
            let p = a + (b - a) * (k as f64 / steps as f64);
            for &i in &self.terrain_idx {
                if let Obj::Terrain(t) = &self.objs[i] {
                    if t.def.kind == TerrainKind::Minefield && t.contains(p) && t.mines_left(p) > 0.3 {
                        n += 1;
                        break;
                    }
                }
            }
        }
        n
    }

    /// Is a flight a -> b (taking `eta` ticks) dangerous: through a gamma lane about to pulse, or into a shockwave?
    fn dangerous(&self, a: Vec2, b: Vec2, eta: u64) -> bool {
        let arrive = self.tick + eta;
        for o in &self.objs {
            match o {
                Obj::Gamma(g) => {
                    let pulse_soon = g.next_pulse <= arrive + 10 * TICKS_PER_SEC as u64;
                    if pulse_soon && (g.in_lane(b, 250.0) || (g.warning(self.tick) && (g.in_lane(a, 0.0) || g.in_lane((a + b) * 0.5, 0.0)))) {
                        return true;
                    }
                }
                Obj::Shock(sh) => {
                    if sh.hits(b, arrive) || sh.hits((a + b) * 0.5, self.tick + eta / 2) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
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

    /// Own safe suns nearest to `p` with ships to spare, up to `max`, until `amount` is covered.
    fn gather(&self, v: &View, p: Vec2, amount: u32, max: usize, exclude: usize) -> Option<(Vec<usize>, u32)> {
        let mut cand: Vec<(f64, usize)> = v
            .mine
            .iter()
            .filter(|&&m| m != exclude && v.threat[m] == 0 && v.avail[m] >= 5)
            .map(|&m| (self.sun(m).unwrap().pos.dist2(p), m))
            .collect();
        cand.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // nearest suns first; if they cannot cover it, the biggest stacks
        let near: u32 = cand.iter().take(max).map(|&(_, m)| v.avail[m]).sum();
        if near < amount {
            cand.sort_by(|a, b| v.avail[b.1].cmp(&v.avail[a.1]).then(a.partial_cmp(b).unwrap()));
        }
        let mut chosen = Vec::new();
        let mut total = 0;
        for (_, m) in cand.into_iter().take(max) {
            chosen.push(m);
            total += v.avail[m];
            if total >= amount {
                return Some((chosen, total));
            }
        }
        None
    }

    fn ai_plan(&mut self, f: u8, st: Style) {
        let mut v = self.ai_view(f);
        if v.mine.is_empty() && v.loose.is_empty() {
            return;
        }
        let my_power = v.power[f as usize];
        let strongest_rival = v.power.iter().enumerate().filter(|(g, _)| *g != f as usize && self.factions[*g].alive).map(|(_, p)| *p).max().unwrap_or(0);
        let bold = (my_power as f64 / strongest_rival.max(1) as f64).clamp(0.6, 2.0);
        // impatience: after 6 minutes the AI gradually accepts closer odds so games do not stall
        let patience = ((self.tick as f64 - 21_600.0) / 36_000.0).clamp(0.0, 1.0);
        let mut moves: Vec<Move> = Vec::new();
        let eta = |a: Vec2, b: Vec2| (a.dist(b) / SHIP_SPEED) as u64;

        // --- evacuate suns that cannot be held
        if st.evacuates {
            for &m in &v.mine {
                let s = self.sun(m).unwrap();
                let hold = v.avail[m] as i64 + s.health as i64;
                if v.threat[m] >= 40 && v.threat[m] as f64 > hold as f64 * 1.4 && v.avail[m] > 0 {
                    let safe = v.mine.iter().filter(|&&o| o != m && v.threat[o] == 0).min_by(|&&a, &&b| {
                        let (pa, pb) = (self.sun(a).unwrap().pos.dist2(s.pos), self.sun(b).unwrap().pos.dist2(s.pos));
                        pa.partial_cmp(&pb).unwrap().then(a.cmp(&b))
                    });
                    if let Some(&o) = safe {
                        moves.push(Move { score: 1000.0, sources: vec![m], loose: false, target: o, amount: v.avail[m], point: None });
                    }
                }
            }
        }

        // --- upgrade / repair own suns that are safe
        for &m in &v.mine {
            let s = self.sun(m).unwrap();
            if v.threat[m] > 0 || !s.wants_ships() {
                continue;
            }
            let need = (s.max_health() - s.health) + if s.level < s.max_level { UPGRADE_COST[s.level as usize] - s.growth } else { 0 };
            let need = need as i64 - v.inflight[m] as i64;
            if need <= 0 {
                continue;
            }
            if st.avoids_hazards && self.dangerous(s.pos, s.pos, 0) {
                continue;
            }
            let amount = (need as f64 * st.waste).ceil() as u32;
            if v.avail[m] >= amount {
                moves.push(Move { score: 600.0 - need as f64 * 0.5 + s.level as f64 * 5.0, sources: vec![m], loose: false, target: m, amount, point: None });
            } else if let Some((srcs, _)) = self.gather(&v, s.pos, amount, st.sources, usize::MAX) {
                moves.push(Move { score: 420.0 - need as f64 * 0.5, sources: srcs, loose: false, target: m, amount, point: None });
            }
        }

        // --- capture empty suns; attack enemy suns it can beat
        let mut unaffordable: Option<(f64, usize, u32)> = None;
        let others: Vec<usize> = self.suns().filter(|(_, s)| s.present() && s.faction != f as i8).map(|(i, _)| i).collect();
        for t in others {
            let s = self.sun(t).unwrap();
            let tp = s.pos;
            let Some(&nearest) = v.mine.iter().min_by(|&&a, &&b| self.sun(a).unwrap().pos.dist2(tp).partial_cmp(&self.sun(b).unwrap().pos.dist2(tp)).unwrap().then(a.cmp(&b))) else {
                continue;
            };
            let from = self.sun(nearest).unwrap().pos;
            let dist = from.dist(tp);
            let guard = v.threat[t] as f64; // enemy ships around the target
            // what the whole wave must be (ships already on their way count towards it)
            let (wave, base) = if s.faction < 0 {
                let mine_progress = if s.cap_faction == f as i8 { s.cap_progress } else { -s.cap_progress };
                (((CAPTURE_COST - mine_progress) as f64 + guard * 1.2) * st.waste, 400.0)
            } else {
                // a decisive wave: full health (it may be repaired meanwhile), the defenders, and what the sun makes
                // while we fly; a stronger AI attacks more readily (lower margin), an impatient one too
                let m = (st.margin / bold.sqrt() * (1.0 - 0.2 * patience)).max(1.05);
                let prod = production_per_sec(s.level) * eta(from, tp) as f64 / TICKS_PER_SEC as f64;
                ((s.max_health() as f64 + guard * 1.25 + prod * 0.5 + 10.0) * m, 300.0 + s.level as f64 * 15.0)
            };
            let need = wave - v.inflight[t] as f64;
            if need < wave * 0.25 {
                continue;
            }
            let mut amount = need.ceil() as u32;
            let mut score = base - dist / 40.0 - amount as f64 / 6.0;
            if st.avoids_hazards {
                let mines = self.mine_samples(from, tp);
                if mines > 0 {
                    amount = (amount as f64 * (1.0 + 0.04 * mines as f64)).ceil() as u32;
                    score -= 12.0 * mines as f64;
                }
                if self.dangerous(from, tp, eta(from, tp)) {
                    continue;
                }
            }
            if let Some((srcs, _)) = self.gather(&v, tp, amount, st.sources, usize::MAX) {
                moves.push(Move { score, sources: srcs, loose: false, target: t, amount, point: None });
            } else if s.faction >= 0 && unaffordable.as_ref().map(|u: &(f64, usize, u32)| score > u.0).unwrap_or(true) {
                unaffordable = Some((score, t, amount));
            }
        }

        // --- stage: the best attack needs more ships than its nearest suns hold, but the whole empire has them:
        // bring idle stacks together at the own sun nearest the target
        if let Some((_, t, amount)) = unaffordable {
            let total: u32 = v.mine.iter().filter(|&&m| v.threat[m] == 0).map(|&m| v.avail[m]).sum::<u32>() + v.loose_strength;
            if total >= amount {
                let tp = self.sun(t).unwrap().pos;
                let rally = v
                    .mine
                    .iter()
                    .copied()
                    .min_by(|&a, &b| {
                        let key = |m: usize| self.sun(m).unwrap().pos.dist(tp) + if self.sun(m).unwrap().wants_ships() { 2500.0 } else { 0.0 };
                        key(a).partial_cmp(&key(b)).unwrap().then(a.cmp(&b))
                    })
                    .unwrap();
                for &m in &v.mine {
                    if m != rally && v.threat[m] == 0 && v.avail[m] >= 20 {
                        moves.push(Move { score: 270.0 - self.sun(m).unwrap().pos.dist(tp) / 200.0, sources: vec![m], loose: false, target: rally, amount: v.avail[m], point: None });
                    }
                }
            }
        }

        // --- white holes and supernovas
        for i in 0..self.objs.len() {
            let (pos, need, prize) = match &self.objs[i] {
                Obj::WhiteHole(w) if w.accepting() => {
                    let stake = if w.investor == f as i8 { w.invest as i64 } else { -(w.invest as i64) };
                    (w.pos, w.required() as i64 - stake, w.prize(self.tick) as f64)
                }
                Obj::Supernova(n) if n.phase == NovaPhase::Ready => {
                    let stake = if n.investor == f as i8 { n.invest as i64 } else { -(n.invest as i64) };
                    (n.pos, n.required() as i64 - stake, n.def.unit_residuals as f64 + 200.0)
                }
                _ => continue,
            };
            let need = need - v.inflight[i] as i64;
            if need <= 0 || (need as u32) * 2 > my_power {
                continue;
            }
            let amount = (need as f64 * 1.05).ceil() as u32;
            if let Some((srcs, _)) = self.gather(&v, pos, amount, st.sources, usize::MAX) {
                moves.push(Move { score: 200.0 + prize / 5.0 - amount as f64 / 8.0, sources: srcs, loose: false, target: i, amount, point: None });
            }
        }

        // --- hunt big stray stacks of factions that have lost their suns (games end decisively)
        for g in 0..self.factions.len() as u8 {
            if g == f || !self.factions[g as usize].alive || self.sun_count(g) > 0 {
                continue;
            }
            let (mut c, mut n) = (Vec2::ZERO, 0u32);
            for x in self.ships.iter().filter(|x| x.alive && x.faction == g) {
                c += x.pos * x.strength as f64;
                n += x.strength;
            }
            if n < 20 {
                continue;
            }
            let c = c * (1.0 / n as f64);
            let amount = (n as f64 * st.margin).ceil() as u32;
            if let Some((srcs, _)) = self.gather(&v, c, amount, st.sources + 1, usize::MAX) {
                moves.push(Move { score: 320.0, sources: srcs, loose: false, target: usize::MAX, point: Some(c), amount });
            }
        }

        // --- gather loose ships into the nearest own sun's cloud
        if v.loose_strength >= 10 {
            let c = v.loose.iter().fold(Vec2::ZERO, |a, &k| a + self.ships[k].pos) * (1.0 / v.loose.len() as f64);
            if let Some(t) = self.nearest_own_sun(f, c) {
                moves.push(Move { score: 250.0, sources: Vec::new(), loose: true, target: t, amount: v.loose_strength, point: None });
            }
        }

        // --- gather idle stacks from rear suns that need nothing to the front
        if v.mine.len() >= 2 {
            let enemy_suns: Vec<Vec2> = self.suns().filter(|(_, s)| s.present() && s.faction != f as i8).map(|(_, s)| s.pos).collect();
            let front_dist = |p: Vec2| enemy_suns.iter().map(|e| e.dist(p)).fold(f64::MAX, f64::min);
            if let Some(&front) = v.mine.iter().min_by(|&&a, &&b| front_dist(self.sun(a).unwrap().pos).partial_cmp(&front_dist(self.sun(b).unwrap().pos)).unwrap().then(a.cmp(&b))) {
                for &m in &v.mine {
                    let s = self.sun(m).unwrap();
                    if m != front && !s.wants_ships() && v.threat[m] == 0 && v.avail[m] >= 80 && front_dist(s.pos) > front_dist(self.sun(front).unwrap().pos) + 1500.0 {
                        moves.push(Move { score: 150.0, sources: vec![m], loose: false, target: front, amount: v.avail[m] * 3 / 4, point: None });
                    }
                }
            }
        }

        // --- pick the best few, without spending the same ships twice
        moves.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap().then(a.target.cmp(&b.target)));
        let gentle = 1 + self.factions[f as usize].plans as usize / 4;
        let max_moves = st.moves.min(gentle);
        let mut done = 0;
        let mut targeted = Vec::new();
        for mv in moves {
            if done >= max_moves {
                break;
            }
            if targeted.contains(&mv.target) {
                continue;
            }
            let mut ids = Vec::new();
            let mut total = 0;
            if mv.loose {
                ids.extend(v.loose.iter().map(|&k| self.ships[k].id));
                v.loose.clear();
            } else {
                let have: u32 = mv.sources.iter().map(|&m| v.avail[m]).sum();
                if have < mv.amount.min(have.max(1)) || have == 0 {
                    continue;
                }
                'outer: for &m in &mv.sources {
                    let pool = std::mem::take(&mut v.pool[m]);
                    let mut rest = Vec::new();
                    for k in pool {
                        if total >= mv.amount {
                            rest.push(k);
                            continue;
                        }
                        ids.push(self.ships[k].id);
                        total += self.ships[k].strength;
                        v.avail[m] -= self.ships[k].strength;
                    }
                    v.pool[m] = rest;
                    if total >= mv.amount {
                        break 'outer;
                    }
                }
                if total < mv.amount {
                    continue;
                }
            }
            if ids.is_empty() {
                continue;
            }
            ids.sort_unstable();
            let mut target = match mv.point {
                Some(p) => Target::Point(p),
                None => Target::Obj(mv.target),
            };
            if st.uses_wormholes && mv.point.is_none() {
                let from = self.ships[self.ship_index(ids[0]).unwrap()].pos;
                if let (Some(tp), true) = (self.obj_pos(mv.target), matches!(self.objs[mv.target], Obj::Sun(_))) {
                    if let Some(w) = self.wormhole_route(from, tp) {
                        target = Target::Obj(w);
                    }
                }
            }
            if self.issue(Command::Order { faction: f, ships: ids, target }).is_ok() {
                done += 1;
                targeted.push(mv.target);
            }
        }
    }
}
