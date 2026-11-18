//! Ship behaviour: flight, circling suns, hovering, arrival, ship-to-ship combat, self-defence and merging.

use crate::math::{dir_deg, sin_deg, Vec2};
use crate::rules::*;
use crate::{oval, Event, Mode, Obj, Ship, Sim, Target, NO_SUN};

/// Uniform grid over ship positions for neighbour queries: (cell key * 8 + faction, ship index), sorted. Keying by
/// faction lets a query skip a dense friendly cloud entirely.
pub(crate) struct Grid {
    cell: f64,
    cells: Vec<(i64, u32)>,
}

fn cell_of(p: Vec2, cell: f64) -> (i64, i64) {
    ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64)
}
fn key(c: (i64, i64)) -> i64 {
    (c.0 + 1_000_000) * 4_000_000 + (c.1 + 1_000_000)
}

impl Grid {
    pub(crate) fn build(ships: &[Ship], cell: f64, keep: impl Fn(&Ship) -> bool) -> Grid {
        let mut cells: Vec<(i64, u32)> = ships.iter().enumerate().filter(|(_, s)| keep(s)).map(|(i, s)| (key(cell_of(s.pos, cell)) * 8 + s.faction as i64, i as u32)).collect();
        cells.sort_unstable();
        Grid { cell, cells }
    }

    pub(crate) fn build_where(ships: &[Ship], cell: f64, keep: impl Fn(usize) -> bool) -> Grid {
        let mut cells: Vec<(i64, u32)> = ships.iter().enumerate().filter(|(i, _)| keep(*i)).map(|(i, s)| (key(cell_of(s.pos, cell)) * 8 + s.faction as i64, i as u32)).collect();
        cells.sort_unstable();
        Grid { cell, cells }
    }

    /// Nearest ship of one of `factions` within `range` of `p` (ties: lowest index) accepted by `ok`.
    pub(crate) fn nearest(&self, ships: &[Ship], p: Vec2, range: f64, factions: &[u8], ok: impl Fn(usize, &Ship) -> bool) -> Option<usize> {
        let (cx, cy) = cell_of(p, self.cell);
        let r2 = range * range;
        let reach = (range / self.cell).ceil() as i64;
        let mut best: Option<(f64, usize)> = None;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
              for &fa in factions {
                let k = key((cx + dx, cy + dy)) * 8 + fa as i64;
                let start = self.cells.partition_point(|c| c.0 < k);
                for c in &self.cells[start..] {
                    if c.0 != k {
                        break;
                    }
                    let j = c.1 as usize;
                    let b = &ships[j];
                    if !ok(j, b) {
                        continue;
                    }
                    let d = p.dist2(b.pos);
                    if d < r2 && best.map(|(bd, bj)| d < bd || (d == bd && j < bj)).unwrap_or(true) {
                        best = Some((d, j));
                    }
                }
              }
            }
        }
        best.map(|(_, j)| j)
    }
}

/// Coarse map of which factions have fighting ships where (cells of `PRESENCE_CELL`), so ships with no enemy in
/// their neighbourhood skip the fine neighbour search entirely (most circling ships, most of the time).
pub(crate) struct Presence {
    origin: Vec2,
    cols: i64,
    rows: i64,
    masks: Vec<u8>,
}
const PRESENCE_CELL: f64 = 200.0;

impl Presence {
    pub(crate) fn build(ships: &[Ship]) -> Presence {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for s in ships.iter().filter(|s| fights(s)) {
            x0 = x0.min(s.pos.x);
            y0 = y0.min(s.pos.y);
            x1 = x1.max(s.pos.x);
            y1 = y1.max(s.pos.y);
        }
        if x0 > x1 {
            return Presence { origin: Vec2::ZERO, cols: 0, rows: 0, masks: Vec::new() };
        }
        let origin = Vec2::new(x0 - PRESENCE_CELL, y0 - PRESENCE_CELL);
        let cols = (((x1 - origin.x) / PRESENCE_CELL) as i64 + 2).min(4000);
        let rows = (((y1 - origin.y) / PRESENCE_CELL) as i64 + 2).min(4000);
        let mut p = Presence { origin, cols, rows, masks: vec![0; (cols * rows) as usize] };
        for s in ships.iter().filter(|s| fights(s)) {
            if let Some(k) = p.index(s.pos, 0, 0) {
                p.masks[k] |= 1 << (s.faction & 7);
            }
        }
        p
    }
    fn index(&self, pos: Vec2, dx: i64, dy: i64) -> Option<usize> {
        let x = ((pos.x - self.origin.x) / PRESENCE_CELL).floor() as i64 + dx;
        let y = ((pos.y - self.origin.y) / PRESENCE_CELL).floor() as i64 + dy;
        (x >= 0 && y >= 0 && x < self.cols && y < self.rows).then(|| (y * self.cols + x) as usize)
    }
    /// Could a ship of `faction` at `pos` have an enemy within one cell?
    pub(crate) fn enemy_near(&self, pos: Vec2, faction: u8) -> bool {
        let own = 1u8 << (faction & 7);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(k) = self.index(pos, dx, dy) {
                    if self.masks[k] & !own != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn fights(s: &Ship) -> bool {
    s.alive && !s.neutral
}

impl Sim {
    /// For each faction, the list of other factions.
    fn enemy_lists(&self) -> Vec<Vec<u8>> {
        let n = self.factions.len() as u8;
        (0..n).map(|f| (0..n).filter(|&g| g != f).collect()).collect()
    }

    /// Make a ship idle: it joins a nearby friendly sun's cloud, or hovers where it is.
    pub(crate) fn make_idle(&mut self, s: &mut Ship) {
        s.target = Target::None;
        s.mode = Mode::Hover;
        s.home = NO_SUN;
        s.hold = false;
        s.guard = 0;
        if let Some(j) = self.join_candidate(s.faction, s.pos) {
            self.join_orbit(s, j, None);
        }
    }

    /// Where each ship's nearest visible enemy is (for swerving towards it), indexed like `ships`.
    fn seek_targets(&self) -> Vec<Option<Vec2>> {
        let presence = Presence::build(&self.ships);
        let near: Vec<bool> = self.ships.iter().map(|s| fights(s) && presence.enemy_near(s.pos, s.faction)).collect();
        let grid = Grid::build_where(&self.ships, SEEK_RANGE, |i| near[i]);
        let enemies = self.enemy_lists();
        self.ships
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if !near[i] {
                    return None;
                }
                grid.nearest(&self.ships, a.pos, SEEK_RANGE, &enemies[a.faction as usize], |_, b| fights(b)).map(|j| self.ships[j].pos)
            })
            .collect()
    }

    pub(crate) fn move_ships(&mut self) {
        let seek = self.seek_targets();
        self.refresh_strength_cache();
        let mut ships = std::mem::take(&mut self.ships);
        for (k, s) in ships.iter_mut().enumerate() {
            if s.alive {
                self.move_ship(s, seek[k]);
            }
        }
        // ships spawned while moving (none today) would be in self.ships; keep the id order
        let mut extra = std::mem::take(&mut self.ships);
        ships.append(&mut extra);
        self.ships = ships;
        // a capture / upgrade completed: friendly ships still on their way stop and join the cloud instead
        if !self.completed.is_empty() {
            for i in 0..self.ships.len() {
                let mut s = self.ships[i];
                if let Target::Obj(j) = s.target {
                    if s.alive && self.completed.contains(&(j, s.faction)) {
                        self.join_orbit(&mut s, j, None);
                        self.ships[i] = s;
                    }
                }
            }
        }
    }

    fn move_ship(&mut self, s: &mut Ship, seek: Option<Vec2>) {
        let fx = self.terrain_effects(s);
        if !s.alive {
            return;
        }
        // targets and homes that went away
        if s.target != Target::None && !self.valid_target(s.target) {
            self.make_idle(s);
        }
        if s.mode == Mode::Orbit {
            let ok = self.sun(s.home as usize).is_some_and(|h| h.present() && h.faction == s.faction as i8);
            if !ok {
                self.make_idle(s);
            }
        }
        if s.guard > 0 && !self.sun(s.guard as usize - 1).is_some_and(|h| h.faction == s.faction as i8) {
            s.guard = 0;
            s.hold = false;
        }
        if fx.accel != Vec2::ZERO && s.mode != Mode::Orbit {
            s.push += fx.accel;
            let l = s.push.len();
            if l > PUSH_MAX {
                s.push = s.push * (PUSH_MAX / l);
            }
            s.push = s.push * PUSH_DECAY_INSIDE;
        } else {
            s.push = s.push * PUSH_DECAY_OUTSIDE;
        }

        let speed = SHIP_SPEED * s.speed_mul * fx.speed;
        let old = s.pos;
        let mut newpos;
        if let Some(e) = seek {
            // an enemy is close: swerve towards it
            let desired = (e - s.pos).norm_or(s.heading);
            s.heading = (s.heading + desired * SEEK_STEER).norm_or(desired);
            newpos = s.pos + s.heading * speed + s.push;
            if s.mode == Mode::Orbit {
                s.orbit_ang += s.orbit_w * fx.speed;
            }
        } else {
            match s.mode {
                Mode::Fly => {
                    let tp = self.target_pos(s.target).unwrap_or(s.pos);
                    let mut desired = (tp - s.pos).norm_or(s.heading);
                    // wobble: a loose swarm rather than a line
                    let w = WOBBLE_DEG * sin_deg(s.wobble + self.tick as f64 * WOBBLE_RATE);
                    desired = desired.rotate(w);
                    // bend around suns that are in the way
                    let tgt = if let Target::Obj(j) = s.target { j } else { usize::MAX };
                    for (j, su) in self.suns() {
                        if j == tgt || !su.present() {
                            continue;
                        }
                        let rel = s.pos - su.pos;
                        let d = rel.len();
                        let edge = su.radius + AVOID_MARGIN;
                        if d >= edge || d < 1e-6 {
                            continue;
                        }
                        let away = rel * (1.0 / d);
                        if desired.dot(away) >= 0.3 {
                            continue; // already heading away
                        }
                        let k = 1.0 - (d - su.radius).max(0.0) / AVOID_MARGIN;
                        let side = if desired.cross(away) > 0.0 { -away.perp() } else { away.perp() };
                        desired = (desired + (away * 0.8 + side) * (k * 2.5)).norm_or(desired);
                    }
                    s.heading = (s.heading + desired * STEER).norm_or(desired);
                    newpos = s.pos + s.heading * speed + s.push;
                }
                Mode::Orbit => {
                    let c = self.sun(s.home as usize).map(|h| h.pos).unwrap_or(s.pos);
                    s.orbit_ang += s.orbit_w * fx.speed;
                    if s.orbit_ang >= 360.0 {
                        s.orbit_ang -= 360.0;
                    }
                    let want = oval(c, s.orbit_a, s.orbit_ang);
                    let d = want - s.pos;
                    let l = d.len();
                    // a ship that has settled sticks to its oval (which travels with a moving sun)
                    let catch_up = speed * 1.6 + self.sun(s.home as usize).and_then(|h| h.orbit.as_ref()).map(|_| 8.0).unwrap_or(0.0);
                    newpos = if l <= catch_up { want } else { s.pos + d * (speed * 1.3 / l) };
                }
                Mode::Hover => {
                    let h = hash2(s.id, self.tick as u32);
                    let jx = ((h & 0xffff) as f64 / 65535.0 - 0.5) * 2.0 * HOVER_JITTER;
                    let jy = (((h >> 16) & 0xffff) as f64 / 65535.0 - 0.5) * 2.0 * HOVER_JITTER;
                    newpos = s.pos + Vec2::new(jx, jy) + s.push;
                    if !s.hold && (self.tick + s.id as u64) % 30 == 0 {
                        if let Some(j) = self.join_candidate(s.faction, s.pos) {
                            self.join_orbit(s, j, None);
                        }
                    }
                }
            }
        }
        // reflect walls
        if newpos != s.pos && self.is_wall(newpos) && !self.is_wall(s.pos) {
            let nx = Vec2::new(newpos.x, s.pos.y);
            let ny = Vec2::new(s.pos.x, newpos.y);
            if self.is_wall(nx) {
                s.heading.x = -s.heading.x;
                s.push.x = -s.push.x;
                newpos.x = s.pos.x;
            }
            if self.is_wall(ny) {
                s.heading.y = -s.heading.y;
                s.push.y = -s.push.y;
                newpos.y = s.pos.y;
            }
            if self.is_wall(newpos) {
                s.heading = -s.heading;
                s.push = -s.push;
                newpos = s.pos;
            }
        }
        // nothing flies through a sun it is not aiming for
        if s.mode == Mode::Fly {
            let tgt = if let Target::Obj(j) = s.target { j } else { usize::MAX };
            for (j, su) in self.suns() {
                if j != tgt && su.present() {
                    let rel = newpos - su.pos;
                    let d2 = rel.len2();
                    let min = su.radius + 2.0;
                    if d2 < min * min {
                        newpos = su.pos + rel.norm_or(s.heading) * min;
                    }
                }
            }
        }
        s.pos = newpos;
        if s.mode != Mode::Fly && seek.is_none() {
            s.heading = (s.pos - old).norm_or(s.heading);
        }

        self.arrive(s);
    }

    /// Arrival at the ship's target (only ships ordered somewhere arrive).
    fn arrive(&mut self, s: &mut Ship) {
        match s.target {
            Target::Obj(j) => match self.objs.get(j) {
                Some(Obj::Sun(su)) if su.present() => {
                    if s.pos.dist(su.pos) >= su.radius + ARRIVE_MARGIN {
                        return;
                    }
                    let own = su.faction == s.faction as i8;
                    let left = if own && !su.wants_ships() { s.strength } else { self.arrive_at_sun(s.faction, s.strength, j) };
                    if left == 0 {
                        s.alive = false;
                        return;
                    }
                    s.strength = left;
                    if self.sun(j).is_some_and(|su| su.faction == s.faction as i8) {
                        self.join_orbit(s, j, None);
                    } else {
                        self.make_idle(s);
                    }
                }
                Some(Obj::Wormhole(w)) => {
                    if s.pos.dist2(w.pos) < w.size * w.size {
                        self.jump(s, j);
                    }
                }
                Some(Obj::WhiteHole(w)) => {
                    if s.pos.dist(w.pos) < w.radius().max(OBJECT_RADIUS) {
                        let left = self.invest(j, s.faction, s.strength);
                        if left == 0 {
                            s.alive = false;
                        } else {
                            s.strength = left;
                            self.make_idle(s);
                        }
                    }
                }
                Some(Obj::Supernova(n)) => {
                    if s.pos.dist(n.pos) < n.radius().max(OBJECT_RADIUS) {
                        let left = self.invest(j, s.faction, s.strength);
                        if left == 0 {
                            s.alive = false;
                        } else {
                            s.strength = left;
                            self.make_idle(s);
                        }
                    }
                }
                _ => {}
            },
            Target::Point(p) => {
                if s.pos.dist(p) < POINT_ARRIVE + SHIP_SPEED * s.speed_mul {
                    s.target = Target::None;
                    s.mode = Mode::Hover;
                    if !s.hold {
                        self.make_idle(s);
                    }
                }
            }
            Target::None => {}
        }
    }

    /// Ship-to-ship collisions: the stronger survives, weakened by the weaker; equal strengths cancel out.
    pub(crate) fn combat(&mut self) {
        if self.ships.len() < 2 {
            return;
        }
        let presence = Presence::build(&self.ships);
        let near: Vec<bool> = self.ships.iter().map(|s| fights(s) && presence.enemy_near(s.pos, s.faction)).collect();
        let grid = Grid::build_where(&self.ships, HIT_RANGE * 2.0, |i| near[i]);
        let enemies = self.enemy_lists();
        for i in 0..self.ships.len() {
            let a = self.ships[i];
            if !near[i] || !fights(&a) {
                continue;
            }
            let ships = &self.ships;
            let Some(j) = grid.nearest(ships, a.pos, HIT_RANGE, &enemies[a.faction as usize], |_, b| fights(b)) else { continue };
            let b = self.ships[j];
            let m = a.strength.min(b.strength);
            self.ships[i].strength -= m;
            self.ships[j].strength -= m;
            if self.ships[i].strength == 0 {
                self.ships[i].alive = false;
            }
            if self.ships[j].strength == 0 {
                self.ships[j].alive = false;
            }
            self.factions[a.faction as usize].combat_losses += m;
            self.factions[b.faction as usize].combat_losses += m;
            self.events.push(Event::Combat { pos: (a.pos + b.pos) * 0.5, a: a.faction, b: b.faction });
        }
    }

    /// Self-defence: when a serious attack heads for a (non-orbiting) sun, part of its circling cloud breaks off,
    /// about once a second, to meet the attackers on their side, until the defenders clearly outnumber them.
    pub(crate) fn self_defence(&mut self) {
        let due: Vec<usize> = self
            .suns()
            .filter(|(j, s)| (self.tick + *j as u64 * 7) % DEFENCE_TICKS == 0 && s.faction >= 0 && s.present() && !s.moving())
            .map(|(j, _)| j)
            .collect();
        for j in due {
            let (owner, pos, cloud) = {
                let s = self.sun(j).unwrap();
                (s.faction as u8, s.pos, s.cloud_radius())
            };
            let r2 = DEFENCE_RADIUS * DEFENCE_RADIUS;
            let (mut attack, mut defence) = (0u32, 0u32);
            let mut centroid = Vec2::ZERO;
            for s in &self.ships {
                if !s.alive {
                    continue;
                }
                if s.faction != owner && s.target == Target::Obj(j) && s.pos.dist2(pos) <= r2 {
                    attack += s.strength;
                    centroid += s.pos * s.strength as f64;
                } else if s.faction == owner && s.guard == j as u32 + 1 {
                    defence += s.strength;
                }
            }
            if attack == 0 {
                if defence > 0 {
                    // the threat is over: defenders go back to circling
                    for i in 0..self.ships.len() {
                        if self.ships[i].guard == j as u32 + 1 && self.ships[i].faction == owner {
                            let mut s = self.ships[i];
                            self.make_idle(&mut s);
                            self.ships[i] = s;
                        }
                    }
                }
                continue;
            }
            if attack < DEFENCE_MIN_ATTACK {
                continue;
            }
            let want = (attack as f64 * DEFENCE_RATIO).ceil() as u32;
            if defence >= want {
                continue;
            }
            let need = want - defence;
            let batch = ((need as f64 * DEFENCE_SHARE).ceil() as u32).max(need.min(8));
            let dir = (centroid * (1.0 / attack as f64) - pos).norm_or(Vec2::new(1.0, 0.0));
            let meet = pos + dir * (cloud + 150.0);
            // the circling ships on the attackers' side go first
            let mut cand: Vec<(f64, usize)> = self
                .ships
                .iter()
                .enumerate()
                .filter(|(_, s)| s.alive && s.faction == owner && s.mode == Mode::Orbit && s.home == j as u32 && s.idle())
                .map(|(i, s)| (-(s.pos - pos).dot(dir), i))
                .collect();
            cand.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mut sent = 0;
            let spread = (POINT_SPREAD * (batch as f64).sqrt()).min(POINT_SPREAD_MAX);
            for (_, i) in cand {
                if sent >= batch {
                    break;
                }
                let off = dir_deg(self.rng.unit() * 360.0) * (spread * self.rng.unit().sqrt());
                let s = &mut self.ships[i];
                s.target = Target::Point(meet + off);
                s.mode = Mode::Fly;
                s.home = NO_SUN;
                s.hold = true;
                s.guard = j as u32 + 1;
                sent += s.strength;
            }
        }
    }

    /// When the board is very crowded, nearby idle friendly ships fuse into fewer, stronger dots.
    pub(crate) fn merge_crowds(&mut self) {
        if self.ships.len() <= MERGE_THRESHOLD {
            return;
        }
        let mergeable = |s: &Ship| s.alive && s.target == Target::None && s.guard == 0 && s.strength < MERGE_MAX;
        let grid = Grid::build(&self.ships, MERGE_RANGE * 4.0, mergeable);
        for i in 0..self.ships.len() {
            let a = self.ships[i];
            if !mergeable(&a) {
                continue;
            }
            let ships = &self.ships;
            let Some(j) = grid.nearest(ships, a.pos, MERGE_RANGE, &[a.faction], |j, b| {
                j != i && mergeable(b) && b.faction == a.faction && b.home == a.home && b.hold == a.hold && a.strength + b.strength <= MERGE_MAX
            }) else {
                continue;
            };
            // the lower index absorbs the other
            let (keep, gone) = if i < j { (i, j) } else { (j, i) };
            self.ships[keep].strength += self.ships[gone].strength;
            self.ships[gone].alive = false;
        }
        self.ships.retain(|s| s.alive);
    }
}

pub(crate) fn hash2(a: u32, b: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}
