//! Deterministic headless simulation implementing `cleanroom/public/RULES.md`.
//!
//! * Fixed 60 Hz step: [`Sim::step`] advances exactly one tick and takes that tick's commands.
//! * No wall clock, no global RNG: all randomness comes from the seeded [`math::Rng`] stored in the sim.
//! * No dependence on libm: see [`math`].
//! * Object identity = index into the level's `objects` array (same as in the map crate).
//! * Every ship is an entity in [`Sim::ships`], kept sorted by `id`; there is no hidden stock inside a sun.
//!
//! Module layout: this file holds the types, construction, queries, commands and the tick; `ships.rs` movement,
//! orbits, arrival, combat, self-defence and merging; `specials.rs` the special objects; `terrain.rs` the zones;
//! `ai.rs` the computer players.

pub mod ai;
pub mod math;
pub mod rules;
mod ships;
mod specials;
mod terrain;

use au2_map::{FogDef, Map, Object, OrbitDef};
pub use math::Vec2;
use math::{cos_deg, dir_deg, sin_deg, Rng};
use rules::*;
pub use specials::{Gamma, NovaPhase, Shock, ShockShape, Supernova, WhiteHole, WhitePhase, Wormhole};
pub use terrain::Terrain;

// ------------------------------------------------------------------------------------------------ public types
/// Where a ship is going. `Obj(i)` is an index into [`Sim::objs`]; its kind decides what arriving does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// No order: the ship is idle (circling a sun or hovering).
    None,
    Obj(usize),
    Point(Vec2),
}

/// What a ship is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Flying towards its target.
    Fly,
    /// Idle, circling the sun `home` on an oval.
    Orbit,
    /// Idle, hovering in space.
    Hover,
}

pub const NO_SUN: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ship {
    pub id: u32,
    pub pos: Vec2,
    /// Unit heading (direction of the last movement).
    pub heading: Vec2,
    /// Extra velocity from force terrain and blasts (units/tick).
    pub push: Vec2,
    pub faction: u8,
    /// How many ships this dot counts as (more than 1 after merging, or for strong white-hole ships).
    pub strength: u32,
    pub target: Target,
    pub mode: Mode,
    /// Sun being circled (object index) while `mode == Orbit`, else [`NO_SUN`].
    pub home: u32,
    /// Orbit oval: semi-major axis, current parameter angle (degrees) and angular speed (degrees per tick).
    pub orbit_a: f64,
    pub orbit_ang: f64,
    pub orbit_w: f64,
    /// Ordered to hover in space: stays put instead of joining a nearby sun.
    pub hold: bool,
    /// Defending sun `guard - 1` (self-defence); 0 = not a defender.
    pub guard: u32,
    pub speed_mul: f64,
    /// Wobble phase (degrees).
    pub wobble: f64,
    pub alive: bool,
    /// Inside a neutral zone this tick.
    pub neutral: bool,
}

impl Ship {
    pub fn idle(&self) -> bool {
        self.target == Target::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FragilePhase {
    Intact,
    /// Broken: a small wreck that cannot be captured.
    Broken { until: u64 },
    /// Growing back; becomes an empty neutral sun at `until`.
    Rebuilding { from: u64, until: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fragile {
    pub phase: FragilePhase,
    pub broken_ticks: u32,
    pub rebuild_ticks: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sun {
    pub pos: Vec2,
    pub centre: Vec2,
    pub orbit: Option<OrbitDef>,
    /// -1 neutral, else faction index.
    pub faction: i8,
    /// 0 = empty (only neutral suns are empty), 1..=max_level while owned.
    pub level: u8,
    pub max_level: u8,
    pub health: i32,
    /// Growth towards the next level (owned suns).
    pub growth: i32,
    /// Faction currently filling this empty sun (-1 none) and how far along.
    pub cap_faction: i8,
    pub cap_progress: i32,
    ring_timer: u32,
    accum: u32,
    rings: u32,
    /// Tick of the last production ring (for the pulse).
    pub last_ring: u64,
    pub fragile: Option<Fragile>,
    pub radius: f64,
}

impl Sun {
    /// Exists in the world right now (not a broken / rebuilding fragile sun).
    pub fn present(&self) -> bool {
        match &self.fragile {
            None => true,
            Some(f) => matches!(f.phase, FragilePhase::Intact),
        }
    }
    pub fn max_health(&self) -> i32 {
        MAX_HEALTH[self.level.min(5) as usize]
    }
    /// Ships of growth for the next level (0 at maximum).
    pub fn growth_needed(&self) -> i32 {
        if self.faction < 0 {
            CAPTURE_COST
        } else if self.level >= self.max_level {
            0
        } else {
            UPGRADE_COST[self.level as usize]
        }
    }
    /// Would a friendly ship arriving now be used up (repair or growth)?
    pub fn wants_ships(&self) -> bool {
        self.faction >= 0 && (self.health < self.max_health() || self.level < self.max_level)
    }
    /// Extent of the circling cloud.
    pub fn cloud_radius(&self) -> f64 {
        cloud_radius(self.radius, self.level)
    }
    /// Orbits along a path (does not self-defend).
    pub fn moving(&self) -> bool {
        self.orbit.is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Sun(Sun),
    Wormhole(Wormhole),
    Supernova(Supernova),
    WhiteHole(WhiteHole),
    Gamma(Gamma),
    Shock(Shock),
    Fog,
    Terrain(Terrain),
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Won,
    Lost,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Order { faction: u8, count: u32 },
    Combat { pos: Vec2, a: u8, b: u8 },
    /// A production ring left the sun.
    Ring { sun: usize, ships: u32 },
    SunHit { sun: usize },
    SunUpgraded { sun: usize, level: u8 },
    SunMaxed { sun: usize },
    SunCaptured { sun: usize, faction: u8 },
    /// Health reached zero (or a gamma ray / supernova): the sun dropped to empty. `level` before death.
    SunDied { sun: usize, faction: i8, level: u8 },
    SunBroken { sun: usize },
    SunRebuilt { sun: usize },
    WormholeJump { obj: usize },
    SupernovaCollapse { obj: usize, faction: u8 },
    Supernova { obj: usize, faction: u8 },
    WhiteHole { obj: usize, faction: u8, ships: u32 },
    GammaWarning { obj: usize },
    GammaPulse { obj: usize },
    Eliminated { faction: u8 },
    OrderRejected { faction: u8, why: OrderError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderError {
    UnknownFaction,
    Eliminated,
    BadTarget,
    /// The target lies in fogged space.
    Fogged,
    NoShips,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Send these ships (by id; ids that are not the faction's live ships are ignored) to `target`.
    Order { faction: u8, ships: Vec<u32>, target: Target },
    Surrender { faction: u8 },
}

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub seed: u64,
    /// AI difficulty 0 (easy) .. 3 (hardest).
    pub ai_difficulty: u8,
    /// Let the AI also drive faction 0 (used by tests and demos).
    pub human_ai: bool,
    /// Master switch for the AI (off = every faction only acts through commands; used by tests and hot-seat play).
    pub enable_ai: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig { seed: 1, ai_difficulty: 1, human_ai: false, enable_ai: true }
    }
}

#[derive(Debug, Clone)]
pub struct FactionState {
    pub alive: bool,
    pub think_period: u32,
    /// Strength lost in ship-to-ship combat so far.
    pub combat_losses: u32,
    pub(crate) next_think: u64,
    pub(crate) plans: u32,
}

/// One sample of per-faction statistics (for the results screen).
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub tick: u64,
    /// Total ship strength.
    pub ships: Vec<u32>,
    /// Ships produced per second (by the current suns).
    pub production: Vec<u32>,
    /// Cumulative combat losses.
    pub losses: Vec<u32>,
    pub suns: Vec<u32>,
}

pub struct Sim {
    pub tick: u64,
    pub objs: Vec<Obj>,
    /// Every ship, sorted by `id`.
    pub ships: Vec<Ship>,
    pub factions: Vec<FactionState>,
    pub status: Status,
    pub fog: Option<FogDef>,
    pub cfg: SimConfig,
    pub stats: Vec<Sample>,
    pub(crate) rng: Rng,
    next_ship_id: u32,
    pub(crate) events: Vec<Event>,
    pub(crate) terrain_idx: Vec<usize>,
    pub(crate) wormholes: Vec<usize>,
    /// (sun, faction) whose capture / upgrade completed this tick: more arrivals join the cloud instead.
    pub(crate) completed: Vec<(usize, u8)>,
    pub(crate) strength_cache: Vec<u32>,
}

// ------------------------------------------------------------------------------------------------ helpers
pub(crate) fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

pub fn orbit_pos(centre: Vec2, o: &OrbitDef, tick: u64) -> Vec2 {
    if o.rx == 0.0 && o.ry == 0.0 {
        return centre;
    }
    let ang = o.initial_angle_deg + (tick as f64) * o.speed / (ORBIT_SPEED_DIV * TICKS_PER_SEC as f64);
    centre + Vec2::new(o.rx * cos_deg(ang), o.ry * sin_deg(ang)).rotate(o.rotation_deg)
}

pub(crate) fn has_orbit(o: &OrbitDef) -> bool {
    o.rx != 0.0 || o.ry != 0.0
}

/// Ships per second a sun at `level` produces (ring size; one ring per second).
pub fn production_per_sec(level: u8) -> f64 {
    RING_SHIPS[(level as usize).min(5)] as f64 * TICKS_PER_SEC as f64 / RING_TICKS as f64
}

/// Point on a sun's orbit oval (semi-major `a`, parameter angle `ang`).
pub fn oval(centre: Vec2, a: f64, ang: f64) -> Vec2 {
    centre + Vec2::new(a * cos_deg(ang), a * ORBIT_FLATTEN * sin_deg(ang))
}

/// What a faction can see (fog of war). With no fog everything is visible.
pub struct Vision {
    /// No fog on this level (or fog lifted): everything is visible.
    pub all: bool,
    /// Visible discs (centre, radius).
    pub circles: Vec<(Vec2, f64)>,
}
impl Vision {
    pub fn visible(&self, p: Vec2) -> bool {
        self.all || self.circles.iter().any(|(c, r)| c.dist2(p) <= r * r)
    }
}

// ------------------------------------------------------------------------------------------------ construction
impl Sim {
    pub fn new(map: &Map, cfg: SimConfig) -> Sim {
        let rng = Rng::new(cfg.seed);
        let factions = map
            .factions
            .iter()
            .map(|f| FactionState { alive: true, think_period: f.think_period, combat_losses: 0, next_think: 0, plans: 0 })
            .collect();
        let mut sim = Sim {
            tick: 0,
            objs: Vec::new(),
            ships: Vec::new(),
            factions,
            status: Status::Playing,
            fog: None,
            cfg,
            stats: Vec::new(),
            rng,
            next_ship_id: 0,
            events: Vec::new(),
            terrain_idx: Vec::new(),
            wormholes: Vec::new(),
            completed: Vec::new(),
            strength_cache: Vec::new(),
        };
        let nf = sim.factions.len() as i32;
        let mut garrisons = Vec::new();
        for (i, o) in map.objects.iter().enumerate() {
            let make_sun = |s: &au2_map::SunDef, orbit: Option<OrbitDef>, fragile: Option<Fragile>| {
                let max_level = s.max_power.clamp(1, 5) as u8;
                let owned = s.faction >= 0 && s.faction < nf;
                let centre = v2(s.pos);
                let pos = match &orbit {
                    Some(or) => orbit_pos(centre, or, 0),
                    None => centre,
                };
                let level = if owned { (s.power.max(1) as u8).min(max_level) } else { 0 };
                Sun {
                    pos,
                    centre,
                    orbit,
                    faction: if owned { s.faction as i8 } else { -1 },
                    level,
                    max_level,
                    health: MAX_HEALTH[level as usize],
                    growth: 0,
                    cap_faction: -1,
                    cap_progress: 0,
                    ring_timer: 0,
                    accum: 0,
                    rings: 0,
                    last_ring: 0,
                    fragile,
                    radius: sun_radius(level),
                }
            };
            let obj = match o {
                Object::Sun(s) => Obj::Sun(make_sun(s, None, None)),
                Object::OrbitingSun(s, or) => Obj::Sun(make_sun(s, has_orbit(or).then(|| or.clone()), None)),
                Object::FragileSun(f) => Obj::Sun(make_sun(
                    &f.sun,
                    has_orbit(&f.orbit).then(|| f.orbit.clone()),
                    Some(Fragile {
                        phase: FragilePhase::Intact,
                        broken_ticks: f.broken_seconds * FRAGILE_TICKS_PER_UNIT,
                        rebuild_ticks: f.rebuild_frames.max(1),
                    }),
                )),
                Object::Wormhole(w) => {
                    sim.wormholes.push(i);
                    Obj::Wormhole(Wormhole::new(w))
                }
                Object::Supernova(d) => Obj::Supernova(Supernova::new(d)),
                Object::WhiteHole(d) => Obj::WhiteHole(WhiteHole::new(d)),
                Object::GammaRay(d) => Obj::Gamma(Gamma::new(d, &mut sim.rng)),
                Object::Shockwave(d) => Obj::Shock(Shock { def: d.clone(), pos: v2(d.pos) }),
                Object::Fog(d) => {
                    sim.fog = Some(d.clone());
                    Obj::Fog
                }
                Object::Terrain(d) => {
                    sim.terrain_idx.push(i);
                    Obj::Terrain(Terrain::new(d))
                }
                Object::Unknown(_) => Obj::Other,
            };
            if let (Obj::Sun(s), Some(n)) = (&obj, sun_garrison(o)) {
                if s.faction >= 0 && n > 0 {
                    garrisons.push((i, n));
                }
            }
            sim.objs.push(obj);
        }
        // starting garrisons are laid out on their sun's orbits
        for (j, n) in garrisons {
            for _ in 0..n {
                let (pos, f) = {
                    let s = sim.sun(j).unwrap();
                    (s.pos, s.faction as u8)
                };
                let mut sh = sim.new_ship(f, pos, Target::None);
                sim.join_orbit(&mut sh, j, None);
                let s = sim.sun(j).unwrap();
                sh.pos = oval(s.pos, sh.orbit_a, sh.orbit_ang);
                sim.spawn(sh);
            }
        }
        // the AI waits a few seconds before its first plan (staggered per faction)
        for f in 0..sim.factions.len() {
            sim.factions[f].next_think = AI_START_TICKS + 23 * f as u64;
        }
        sim.sample_stats();
        sim
    }

    // -------------------------------------------------------------------------------------------- queries
    pub fn faction_count(&self) -> usize {
        self.factions.len()
    }

    pub fn sun(&self, i: usize) -> Option<&Sun> {
        match self.objs.get(i) {
            Some(Obj::Sun(s)) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn sun_mut(&mut self, i: usize) -> Option<&mut Sun> {
        match self.objs.get_mut(i) {
            Some(Obj::Sun(s)) => Some(s),
            _ => None,
        }
    }
    pub fn suns(&self) -> impl Iterator<Item = (usize, &Sun)> {
        self.objs.iter().enumerate().filter_map(|(i, o)| if let Obj::Sun(s) = o { Some((i, s)) } else { None })
    }

    /// Current world position of an object that has one.
    pub fn obj_pos(&self, i: usize) -> Option<Vec2> {
        Some(match self.objs.get(i)? {
            Obj::Sun(s) => s.pos,
            Obj::Wormhole(w) => w.pos,
            Obj::Supernova(s) => s.pos,
            Obj::WhiteHole(w) => w.pos,
            Obj::Gamma(g) => g.pos,
            Obj::Shock(s) => s.pos,
            _ => return None,
        })
    }

    pub fn target_pos(&self, t: Target) -> Option<Vec2> {
        match t {
            Target::None => None,
            Target::Point(p) => Some(p),
            Target::Obj(i) => self.obj_pos(i),
        }
    }

    /// Can orders be aimed at this?
    pub fn valid_target(&self, t: Target) -> bool {
        match t {
            Target::None => false,
            Target::Point(p) => p.x.is_finite() && p.y.is_finite() && p.x.abs() < 50_000.0 && p.y.abs() < 50_000.0,
            Target::Obj(i) => match self.objs.get(i) {
                Some(Obj::Sun(s)) => s.present(),
                Some(Obj::Wormhole(_)) | Some(Obj::WhiteHole(_)) | Some(Obj::Supernova(_)) => true,
                _ => false,
            },
        }
    }

    /// Order-target under a world point (for picking): suns, wormholes, white holes, supernovas.
    pub fn pick(&self, p: Vec2, slack: f64) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, o) in self.objs.iter().enumerate() {
            let (pos, r) = match o {
                Obj::Sun(s) if s.present() => (s.pos, s.radius),
                Obj::Wormhole(w) => (w.pos, w.size),
                Obj::WhiteHole(w) => (w.pos, w.radius()),
                Obj::Supernova(s) => (s.pos, s.radius()),
                _ => continue,
            };
            let d = pos.dist(p);
            if d <= r + slack && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Radius of the area selected by tapping sun `i` (its whole circling cloud).
    pub fn sun_select_radius(&self, i: usize) -> f64 {
        self.sun(i).map(|s| s.cloud_radius() + 60.0).unwrap_or(0.0)
    }

    /// Ids of `faction`'s live ships inside the circle (orbiting, hovering and flying alike).
    pub fn ships_in_circle(&self, faction: u8, c: Vec2, r: f64) -> Vec<u32> {
        let r2 = r * r;
        self.ships.iter().filter(|s| s.alive && s.faction == faction && s.pos.dist2(c) <= r2).map(|s| s.id).collect()
    }

    /// Ids of all of `faction`'s live ships.
    pub fn all_ships(&self, faction: u8) -> Vec<u32> {
        self.ships.iter().filter(|s| s.alive && s.faction == faction).map(|s| s.id).collect()
    }

    /// Index into `ships` of the ship with this id.
    pub fn ship_index(&self, id: u32) -> Option<usize> {
        self.ships.binary_search_by_key(&id, |s| s.id).ok()
    }

    /// Number of ship dots of a faction.
    pub fn ship_count(&self, faction: u8) -> usize {
        self.ships.iter().filter(|s| s.alive && s.faction == faction).count()
    }
    /// Total strength (ships, counting fused ships as what they were made of) of a faction.
    pub fn strength(&self, faction: u8) -> u32 {
        self.ships.iter().filter(|s| s.alive && s.faction == faction).map(|s| s.strength).sum()
    }
    pub fn sun_count(&self, faction: u8) -> usize {
        self.suns().filter(|(_, s)| s.faction == faction as i8 && s.present()).count()
    }
    /// Ships per second the faction's suns make right now (before any AI handicap).
    pub fn production(&self, faction: u8) -> u32 {
        self.suns().filter(|(_, s)| s.faction == faction as i8 && s.present()).map(|(_, s)| production_per_sec(s.level) as u32).sum()
    }
    /// Strength of `faction`'s idle ships circling sun `j`.
    pub fn orbiting_strength(&self, faction: u8, j: usize) -> u32 {
        self.ships
            .iter()
            .filter(|s| s.alive && s.faction == faction && s.mode == Mode::Orbit && s.home == j as u32 && s.idle())
            .map(|s| s.strength)
            .sum()
    }

    pub fn vision(&self, faction: u8) -> Vision {
        let Some(f) = &self.fog else { return Vision { all: true, circles: Vec::new() } };
        if !self.factions[0].alive || self.status != Status::Playing {
            return Vision { all: true, circles: Vec::new() };
        }
        let mut circles = Vec::new();
        for (_, s) in self.suns() {
            if !s.present() {
                continue;
            }
            if s.faction == faction as i8 {
                circles.push((s.pos, f.sun_radius + f.additional_per_ring * s.level.saturating_sub(1) as f64));
            } else if s.faction < 0 {
                circles.push((s.pos, if f.unowned_radius > 0.0 { f.unowned_radius } else { s.radius * 2.5 }));
            } else {
                circles.push((s.pos, if f.enemy_radius > 0.0 { f.enemy_radius } else { s.radius * 2.5 }));
            }
        }
        Vision { all: false, circles }
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Hash of the entire simulation state; equal hashes mean equal states (used by determinism tests / replays).
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        };
        mix(self.tick);
        mix(self.rng.state());
        for s in &self.ships {
            mix(s.id as u64);
            mix(s.pos.x.to_bits());
            mix(s.pos.y.to_bits());
            mix(s.faction as u64);
            mix(s.strength as u64);
        }
        for o in &self.objs {
            match o {
                Obj::Sun(s) => {
                    mix(s.faction as i64 as u64);
                    mix(s.level as u64);
                    mix(s.health as i64 as u64);
                    mix(s.growth as i64 as u64);
                    mix(s.cap_progress as i64 as u64);
                    mix(s.pos.x.to_bits());
                }
                Obj::Supernova(s) => mix(s.invest as u64 ^ ((s.investor as i64 as u64) << 32)),
                Obj::WhiteHole(s) => mix(s.invest as u64 ^ ((s.investor as i64 as u64) << 32)),
                Obj::Gamma(g) => mix(g.next_pulse),
                Obj::Terrain(t) => t.mines.iter().for_each(|m| mix(*m as u64)),
                _ => {}
            }
        }
        h
    }

    // -------------------------------------------------------------------------------------------- commands
    pub fn is_ai(&self, faction: usize) -> bool {
        self.cfg.enable_ai && faction < self.factions.len() && (faction != 0 || self.cfg.human_ai)
    }

    /// Validate and execute one command now.
    pub fn issue(&mut self, cmd: Command) -> Result<(), OrderError> {
        match cmd {
            Command::Surrender { faction } => {
                if faction as usize >= self.factions.len() {
                    return Err(OrderError::UnknownFaction);
                }
                self.surrender(faction);
                Ok(())
            }
            Command::Order { faction, ships, target } => self.order(faction, &ships, target),
        }
    }

    fn order(&mut self, faction: u8, ids: &[u32], target: Target) -> Result<(), OrderError> {
        match self.factions.get(faction as usize) {
            None => return Err(OrderError::UnknownFaction),
            Some(f) if !f.alive => return Err(OrderError::Eliminated),
            Some(_) => {}
        }
        if !self.valid_target(target) {
            return Err(OrderError::BadTarget);
        }
        let tp = self.target_pos(target).unwrap();
        if self.fog.is_some() && !self.vision(faction).visible(tp) {
            return Err(OrderError::Fogged);
        }
        let idx: Vec<usize> = ids.iter().filter_map(|&id| self.ship_index(id)).filter(|&i| self.ships[i].alive && self.ships[i].faction == faction).collect();
        if idx.is_empty() {
            return Err(OrderError::NoShips);
        }
        // ships sent to empty space spread over a disc so the swarm hovers as a loose cloud
        let spread = if let Target::Point(_) = target { (POINT_SPREAD * (idx.len() as f64).sqrt()).min(POINT_SPREAD_MAX) } else { 0.0 };
        let mut count = 0;
        for i in idx {
            let t = match target {
                Target::Point(p) => {
                    let off = dir_deg(self.rng.unit() * 360.0) * (spread * self.rng.unit().sqrt());
                    Target::Point(p + off)
                }
                t => t,
            };
            let s = &mut self.ships[i];
            s.target = t;
            s.mode = Mode::Fly;
            s.home = NO_SUN;
            s.hold = matches!(target, Target::Point(_));
            s.guard = 0;
            count += s.strength;
        }
        self.events.push(Event::Order { faction, count });
        Ok(())
    }

    fn surrender(&mut self, faction: u8) {
        for j in 0..self.objs.len() {
            if matches!(&self.objs[j], Obj::Sun(s) if s.faction == faction as i8) {
                self.kill_sun(j);
            }
        }
        for s in self.ships.iter_mut() {
            if s.faction == faction {
                s.alive = false;
            }
        }
        self.ships.retain(|s| s.alive);
        self.update_status();
    }

    // -------------------------------------------------------------------------------------------- ships
    pub(crate) fn new_ship(&mut self, faction: u8, pos: Vec2, target: Target) -> Ship {
        let speed_mul = 1.0 + self.rng.signed_unit() * SPEED_JITTER;
        let wobble = self.rng.unit() * 360.0;
        Ship {
            id: 0,
            pos,
            heading: Vec2::new(1.0, 0.0),
            push: Vec2::ZERO,
            faction,
            strength: 1,
            target,
            mode: if target == Target::None { Mode::Hover } else { Mode::Fly },
            home: NO_SUN,
            orbit_a: 0.0,
            orbit_ang: 0.0,
            orbit_w: 0.0,
            hold: false,
            guard: 0,
            speed_mul,
            wobble,
            alive: true,
            neutral: false,
        }
    }

    /// Add a ship (scenario setup, tests, tools) and return its id. With `Target::None` it is idle: it joins a nearby
    /// friendly sun's cloud or hovers. With `Target::Point(p)` at its own position it hovers there, held.
    pub fn add_ship(&mut self, faction: u8, pos: Vec2, target: Target) -> u32 {
        let mut s = self.new_ship(faction, pos, target);
        if let Some(tp) = self.target_pos(target) {
            s.heading = (tp - pos).norm_or(s.heading);
        }
        if let Target::Point(_) = target {
            s.hold = true;
        }
        if target == Target::None {
            self.make_idle(&mut s);
        }
        let id = self.next_ship_id;
        self.spawn(s);
        id
    }

    pub(crate) fn spawn(&mut self, mut s: Ship) {
        s.id = self.next_ship_id;
        self.next_ship_id = self.next_ship_id.wrapping_add(1);
        self.ships.push(s);
    }

    /// Put `s` on an orbit around sun `j`. `at`: start at this parameter angle (else from the ship's position).
    pub(crate) fn join_orbit(&mut self, s: &mut Ship, j: usize, at: Option<f64>) {
        let Some(sun) = self.sun(j) else { return };
        let (pos, r, level) = (sun.pos, sun.radius, sun.level);
        let band = self.rng.below(orbit_bands(level)) as f64;
        let a = r * (ORBIT_MIN + (band + self.rng.unit()) * ORBIT_BAND_STEP);
        let v = ORBIT_SPEED * (1.0 + self.rng.signed_unit() * ORBIT_SPEED_JITTER);
        let rel = s.pos - pos;
        s.orbit_ang = match at {
            Some(a) => a,
            None if rel.len2() > 1.0 => math::atan2_deg(rel.y / ORBIT_FLATTEN, rel.x),
            None => self.rng.unit() * 360.0,
        };
        s.orbit_a = a;
        s.orbit_w = v / a * (180.0 / std::f64::consts::PI);
        s.mode = Mode::Orbit;
        s.home = j as u32;
        s.target = Target::None;
        s.hold = false;
        s.guard = 0;
    }

    /// Nearest present sun owned by `faction` to `p`.
    pub fn nearest_own_sun(&self, faction: u8, p: Vec2) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, s) in self.suns() {
            if s.faction == faction as i8 && s.present() {
                let d = s.pos.dist2(p);
                if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// The friendly sun an idle ship at `p` would join (nearest, rim within [`JOIN_DISTANCE`]).
    pub fn join_candidate(&self, faction: u8, p: Vec2) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, s) in self.suns() {
            if s.faction == faction as i8 && s.present() {
                let d = s.pos.dist(p) - s.radius;
                if d <= JOIN_DISTANCE && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    // -------------------------------------------------------------------------------------------- sun state changes
    /// The sun dies: it drops to an empty, ownerless sun whatever its level (fragile suns break).
    pub(crate) fn kill_sun(&mut self, j: usize) {
        let tick = self.tick;
        let (faction, level, broke) = {
            let Some(s) = self.sun_mut(j) else { return };
            if !s.present() {
                return;
            }
            let (f, l) = (s.faction, s.level);
            s.faction = -1;
            s.level = 0;
            s.health = MAX_HEALTH[0];
            s.growth = 0;
            s.cap_faction = -1;
            s.cap_progress = 0;
            s.ring_timer = 0;
            s.accum = 0;
            s.radius = sun_radius(0);
            let mut broke = false;
            if let Some(fr) = &mut s.fragile {
                fr.phase = FragilePhase::Broken { until: tick + fr.broken_ticks as u64 };
                broke = true;
            }
            (f, l, broke)
        };
        if faction >= 0 || broke {
            self.events.push(Event::SunDied { sun: j, faction, level });
        }
        if broke {
            self.events.push(Event::SunBroken { sun: j });
        }
    }

    /// A ship of `faction` with `strength` reaches sun `j`. Returns the strength left over (the ship survives with
    /// it): growth beyond a completed capture/upgrade, damage beyond a kill, or everything if the sun needs nothing.
    pub(crate) fn arrive_at_sun(&mut self, faction: u8, strength: u32, j: usize) -> u32 {
        let f = faction as i8;
        if self.completed.contains(&(j, faction)) {
            return strength;
        }
        let mut left = strength as i32;
        enum Done {
            Nothing,
            Hit,
            Died,
            Captured,
            Upgraded(u8, bool),
        }
        let done = {
            let s = self.sun_mut(j).unwrap();
            if s.faction == f {
                let max_h = s.max_health();
                if s.health < max_h {
                    let k = left.min(max_h - s.health);
                    s.health += k;
                    left -= k;
                }
                if left > 0 && s.level < s.max_level {
                    let need = UPGRADE_COST[s.level as usize] - s.growth;
                    let k = left.min(need);
                    s.growth += k;
                    left -= k;
                    if s.growth >= UPGRADE_COST[s.level as usize] {
                        s.level += 1;
                        s.growth = 0;
                        s.health = s.max_health();
                        s.radius = sun_radius(s.level);
                        Done::Upgraded(s.level, s.level == s.max_level)
                    } else {
                        Done::Nothing
                    }
                } else {
                    Done::Nothing
                }
            } else if s.faction >= 0 {
                let k = left.min(s.health);
                s.health -= k;
                left -= k;
                if s.health <= 0 {
                    Done::Died
                } else {
                    Done::Hit
                }
            } else {
                // empty sun: a tug of war over its growth
                if s.cap_faction != f && s.cap_progress > 0 {
                    let k = left.min(s.cap_progress);
                    s.cap_progress -= k;
                    left -= k;
                    if s.cap_progress == 0 {
                        s.cap_faction = -1;
                    }
                }
                if left > 0 {
                    s.cap_faction = f;
                    let k = left.min(CAPTURE_COST - s.cap_progress);
                    s.cap_progress += k;
                    left -= k;
                }
                if s.cap_progress >= CAPTURE_COST {
                    s.faction = f;
                    s.level = 1;
                    s.health = s.max_health();
                    s.growth = 0;
                    s.cap_faction = -1;
                    s.cap_progress = 0;
                    s.ring_timer = 0;
                    s.accum = 0;
                    s.radius = sun_radius(1);
                    Done::Captured
                } else {
                    Done::Nothing
                }
            }
        };
        match done {
            Done::Nothing => {}
            Done::Hit => self.events.push(Event::SunHit { sun: j }),
            Done::Died => {
                self.events.push(Event::SunHit { sun: j });
                self.kill_sun(j);
            }
            Done::Captured => {
                self.events.push(Event::SunCaptured { sun: j, faction });
                self.completed.push((j, faction));
            }
            Done::Upgraded(level, maxed) => {
                self.events.push(Event::SunUpgraded { sun: j, level });
                if maxed {
                    self.events.push(Event::SunMaxed { sun: j });
                }
                self.completed.push((j, faction));
            }
        }
        left.max(0) as u32
    }

    // -------------------------------------------------------------------------------------------- the tick
    /// Advance one fixed step, applying `cmds` first. Does nothing once the game is decided.
    pub fn step(&mut self, cmds: &[Command]) {
        if self.status != Status::Playing {
            return;
        }
        for c in cmds {
            let faction = match c {
                Command::Order { faction, .. } | Command::Surrender { faction } => *faction,
            };
            if let Err(why) = self.issue(c.clone()) {
                self.events.push(Event::OrderRejected { faction, why });
            }
        }
        if self.status != Status::Playing {
            return;
        }
        self.tick += 1;
        self.completed.clear();
        self.update_objects();
        self.produce();
        self.run_ai();
        self.self_defence();
        self.move_ships();
        self.combat();
        self.hazards();
        self.ships.retain(|s| s.alive);
        if self.tick % MERGE_TICKS == 0 {
            self.merge_crowds();
        }
        self.update_status();
        if self.tick % STATS_TICKS == 0 {
            self.sample_stats();
        }
    }

    fn produce(&mut self) {
        let tick = self.tick;
        for j in 0..self.objs.len() {
            let (n, pos, r, faction, rings) = {
                let ai_pct = match &self.objs[j] {
                    Obj::Sun(s) if s.faction >= 0 && self.is_ai(s.faction as usize) => AI_PRODUCTION_PCT[self.cfg.ai_difficulty.min(3) as usize],
                    _ => 100,
                };
                let Obj::Sun(s) = &mut self.objs[j] else { continue };
                if s.faction < 0 || !s.present() || s.level == 0 {
                    continue;
                }
                s.ring_timer += 1;
                if s.ring_timer < RING_TICKS {
                    continue;
                }
                s.ring_timer = 0;
                s.accum += RING_SHIPS[s.level as usize] * ai_pct;
                let n = s.accum / 100;
                s.accum %= 100;
                if n == 0 {
                    continue;
                }
                s.rings += 1;
                s.last_ring = tick;
                (n, s.pos, s.radius, s.faction as u8, s.rings)
            };
            // a ring of new ships, evenly spaced around the rim
            let start = rings as f64 * 37.0;
            for k in 0..n {
                let ang = start + 360.0 * k as f64 / n as f64;
                let d = dir_deg(ang);
                let mut sh = self.new_ship(faction, pos + d * (r + 4.0), Target::None);
                sh.heading = d;
                self.join_orbit(&mut sh, j, None);
                self.spawn(sh);
            }
            self.events.push(Event::Ring { sun: j, ships: n });
        }
    }

    /// Elimination: a faction is out when it owns no living sun and cannot capture even one empty sun, or when one
    /// rival owns every sun and is several times stronger. An out faction's ships vanish.
    pub(crate) fn update_status(&mut self) {
        let nf = self.factions.len();
        let mut has_sun = vec![false; nf];
        let mut strength = vec![0u32; nf];
        let mut owners = Vec::new();
        let mut any_unowned = false;
        for (_, s) in self.suns() {
            if !s.present() {
                continue;
            }
            if s.faction >= 0 {
                has_sun[s.faction as usize] = true;
                if !owners.contains(&s.faction) {
                    owners.push(s.faction);
                }
            } else {
                any_unowned = true;
            }
        }
        for s in &self.ships {
            if s.alive {
                strength[s.faction as usize] += s.strength;
            }
        }
        let monopoly = if !any_unowned && owners.len() == 1 { Some(owners[0] as usize) } else { None };
        let mut out = Vec::new();
        for f in 0..nf {
            if !self.factions[f].alive {
                continue;
            }
            let broke = !has_sun[f] && strength[f] < CAPTURE_COST as u32;
            let crushed = monopoly.is_some_and(|g| g != f && strength[g] >= strength[f].saturating_mul(DOMINANCE_RATIO));
            if broke || crushed {
                out.push(f);
            }
        }
        for f in out {
            self.factions[f].alive = false;
            for s in self.ships.iter_mut() {
                if s.faction as usize == f {
                    s.alive = false;
                }
            }
            self.events.push(Event::Eliminated { faction: f as u8 });
        }
        self.ships.retain(|s| s.alive);
        let before = self.status;
        if !self.factions[0].alive {
            self.status = Status::Lost;
        } else if self.factions.iter().skip(1).all(|f| !f.alive) {
            self.status = Status::Won;
        }
        if before == Status::Playing && self.status != Status::Playing {
            self.sample_stats();
        }
    }

    pub(crate) fn sample_stats(&mut self) {
        let nf = self.factions.len();
        let mut ships = vec![0; nf];
        for s in &self.ships {
            if s.alive {
                ships[s.faction as usize] += s.strength;
            }
        }
        let production = (0..nf).map(|f| self.production(f as u8)).collect();
        let suns = (0..nf).map(|f| self.sun_count(f as u8) as u32).collect();
        let losses = self.factions.iter().map(|f| f.combat_losses).collect();
        if self.stats.last().is_some_and(|s| s.tick == self.tick) {
            self.stats.pop();
        }
        self.stats.push(Sample { tick: self.tick, ships, production, losses, suns });
    }
}

fn sun_garrison(o: &Object) -> Option<u32> {
    match o {
        Object::Sun(s) | Object::OrbitingSun(s, _) => Some(s.starting_units),
        Object::FragileSun(f) => Some(f.sun.starting_units),
        _ => None,
    }
}
