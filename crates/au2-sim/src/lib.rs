//! Deterministic headless simulation implementing `cleanroom/public/RULES.md`.
//!
//! * Fixed 60 Hz step: [`Sim::step`] advances exactly one tick and takes that tick's commands.
//! * No wall clock, no global RNG: all randomness comes from the seeded [`math::Rng`] stored in the sim.
//! * No dependence on libm: see [`math`].
//! * Object identity = index into the level's `objects` array (same as in the map crate).

pub mod ai;
pub mod math;
pub mod rules;

use au2_map::{FogDef, GammaRayDef, Map, Object, OrbitDef, ShockwaveDef, SupernovaDef, TerrainDef, TerrainKind, WhiteHoleDef};
pub use math::Vec2;
use math::{cos_deg, dir_deg, sin_deg, Rng};
use rules::*;

// ------------------------------------------------------------------------------------------------ public types
/// Where ships are going. `Obj(i)` is an index into [`Sim::objs`]; its kind decides what arriving does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// Idle: the ship stays where it is.
    None,
    Obj(usize),
    Point(Vec2),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ship {
    pub id: u32,
    pub pos: Vec2,
    /// Unit heading.
    pub heading: Vec2,
    /// Extra velocity from force terrain (units/tick).
    pub push: Vec2,
    pub faction: u8,
    pub target: Target,
    /// Where to go after passing through the wormhole this ship is currently targeting.
    pub then: Option<Target>,
    pub speed_mul: f64,
    pub cooldown: u16,
    /// Last wormhole (object index) this ship entered, as `index + 1`; 0 = none. A ship never re-enters the
    /// wormhole it last went through, which breaks endless loops when a wormhole sits on the line to a target.
    pub last_wormhole: u32,
    pub alive: bool,
    /// Inside a neutral zone this tick.
    pub neutral: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Repeat {
    pub target: Target,
    pub then: Option<Target>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FragilePhase {
    Intact,
    /// Destroyed; ships pass through.
    Gone { until: u64 },
    /// Growing back; becomes an empty neutral sun at `until`.
    Rebuilding { from: u64, until: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fragile {
    pub phase: FragilePhase,
    pub gone_ticks: u32,
    pub rebuild_ticks: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sun {
    pub pos: Vec2,
    pub centre: Vec2,
    pub orbit: Option<OrbitDef>,
    /// -1 neutral, else faction index.
    pub faction: i8,
    pub level: u8,
    pub max_level: u8,
    /// Progress towards the next level (>= 0 while owned).
    pub progress: i32,
    /// Ships stationed here.
    pub stock: u32,
    /// Faction currently capturing this neutral sun (-1 none) and how far along.
    pub cap_faction: i8,
    pub cap_progress: i32,
    pub accum: i32,
    pub repeat: Option<Repeat>,
    pub fragile: Option<Fragile>,
    pub radius: f64,
}

impl Sun {
    /// Exists in the world right now (not a destroyed / rebuilding fragile sun).
    pub fn present(&self) -> bool {
        match &self.fragile {
            None => true,
            Some(f) => matches!(f.phase, FragilePhase::Intact),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wormhole {
    pub pos: Vec2,
    pub centre: Vec2,
    pub orbit: Option<OrbitDef>,
    pub size: f64,
    pub dest: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Supernova {
    pub def: SupernovaDef,
    pub pos: Vec2,
    pub invest: [u32; 4],
    pub dormant_until: u64,
    pub last_blast: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhiteHole {
    pub def: WhiteHoleDef,
    pub pos: Vec2,
    pub invest: [u32; 4],
    pub dormant_until: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gamma {
    pub def: GammaRayDef,
    pub pos: Vec2,
    /// Current beam direction in degrees.
    pub angle: f64,
    phase: f64,
    offset: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shock {
    pub def: ShockwaveDef,
    pub pos: Vec2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    pub def: TerrainDef,
    pub enabled: bool,
    pub accel: Vec2,
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
    Launch { faction: u8, count: u32 },
    Combat { pos: Vec2, a: u8, b: u8 },
    SunHit { sun: usize },
    SunUpgraded { sun: usize, level: u8 },
    SunMaxed { sun: usize },
    SunCaptured { sun: usize, faction: u8 },
    SunLost { sun: usize, faction: u8 },
    SunDestroyed { sun: usize },
    SunRebuilt { sun: usize },
    WormholeJump { obj: usize },
    Supernova { obj: usize, faction: u8 },
    WhiteHole { obj: usize, faction: u8, ships: u32 },
    Eliminated { faction: u8 },
    OrderRejected { faction: u8, why: OrderError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderError {
    UnknownFaction,
    Eliminated,
    NoSources,
    TooManySources,
    NotYourSun,
    BadTarget,
    BadPercent,
    TooManyOrders,
    TooManyRepeats,
    NoShips,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Order {
        faction: u8,
        sources: Vec<usize>,
        target: Target,
        /// Optional follow-up after a wormhole target.
        then: Option<Target>,
        /// Fraction of each source's stationed ships to send (1..=100).
        percent: u8,
        /// Keep sending newly produced ships to the same target.
        repeat: bool,
    },
    /// Cancel standing (repeat) orders at these suns.
    Cancel { faction: u8, sources: Vec<usize> },
    Surrender { faction: u8 },
}

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub seed: u64,
    /// AI difficulty 0 (easy) .. 3 (insane).
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
    pub max_sources: u32,
    pub max_orders: u32,
    pub think_period: u32,
    order_log: Vec<u64>,
    pub(crate) next_think: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Pending {
    Invest { obj: usize, faction: u8, n: u32 },
}

pub struct Sim {
    pub tick: u64,
    pub objs: Vec<Obj>,
    pub ships: Vec<Ship>,
    pub factions: Vec<FactionState>,
    pub status: Status,
    pub fog: Option<FogDef>,
    pub cfg: SimConfig,
    pub(crate) rng: Rng,
    next_ship_id: u32,
    events: Vec<Event>,
    terrain_idx: Vec<usize>,
    wormholes: Vec<usize>,
}

// ------------------------------------------------------------------------------------------------ helpers
fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

pub fn orbit_pos(centre: Vec2, o: &OrbitDef, tick: u64) -> Vec2 {
    if o.rx == 0.0 && o.ry == 0.0 {
        return centre;
    }
    let ang = o.initial_angle_deg + (tick as f64) * o.speed / (ORBIT_SPEED_DIV * TICKS_PER_SEC as f64);
    centre + Vec2::new(o.rx * cos_deg(ang), o.ry * sin_deg(ang)).rotate(o.rotation_deg)
}

fn has_orbit(o: &OrbitDef) -> bool {
    o.rx != 0.0 || o.ry != 0.0
}

pub fn production_per_sec(level: u8) -> f64 {
    PRODUCTION_TENTHS[(level as usize).min(5)] as f64 / 10.0
}

#[derive(Default)]
struct TerrainFx {
    speed: f64,
    accel: Vec2,
    mine_ppm: u32,
    confuse_faction_ppm: u32,
    confuse_order_ppm: u32,
    neutral: bool,
}

fn rate_ppm(percent_per_sec: f64) -> u32 {
    // percent per second -> parts per million per tick
    (percent_per_sec * 10000.0 / TICKS_PER_SEC as f64).clamp(0.0, 1_000_000.0) as u32
}

impl Terrain {
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
}

/// What a faction can see (fog of war). With no fog everything is visible.
pub struct Vision {
    all: bool,
    circles: Vec<(Vec2, f64)>,
}
impl Vision {
    pub fn visible(&self, p: Vec2) -> bool {
        self.all || self.circles.iter().any(|(c, r)| c.dist2(p) <= r * r)
    }
}

// ------------------------------------------------------------------------------------------------ construction
impl Sim {
    pub fn new(map: &Map, cfg: SimConfig) -> Sim {
        let mut rng = Rng::new(cfg.seed);
        let mut objs = Vec::new();
        let mut fog = None;
        let mut terrain_idx = Vec::new();
        let mut wormholes = Vec::new();
        for (i, o) in map.objects.iter().enumerate() {
            let make_sun = |s: &au2_map::SunDef, orbit: Option<OrbitDef>, fragile: Option<Fragile>| {
                let max_level = s.max_power.clamp(1, 5) as u8;
                let owned = s.faction >= 0;
                let centre = v2(s.pos);
                let pos = match &orbit {
                    Some(or) => orbit_pos(centre, or, 0),
                    None => centre,
                };
                Sun {
                    pos,
                    centre,
                    orbit,
                    faction: s.faction as i8,
                    level: if owned { (s.power.max(0) as u8).min(max_level) } else { 0 },
                    max_level,
                    progress: 0,
                    stock: if owned { s.starting_units } else { 0 },
                    cap_faction: -1,
                    cap_progress: 0,
                    accum: 0,
                    repeat: None,
                    fragile,
                    radius: sun_radius(max_level),
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
                        gone_ticks: f.broken_seconds * FRAGILE_TICKS_PER_UNIT,
                        rebuild_ticks: f.rebuild_frames.max(1),
                    }),
                )),
                Object::Wormhole(w) => {
                    wormholes.push(i);
                    let centre = v2(w.pos);
                    let orbit = has_orbit(&w.orbit).then(|| w.orbit.clone());
                    let pos = orbit.as_ref().map(|or| orbit_pos(centre, or, 0)).unwrap_or(centre);
                    Obj::Wormhole(Wormhole { pos, centre, orbit, size: w.size, dest: w.destination })
                }
                Object::Supernova(d) => Obj::Supernova(Supernova { def: d.clone(), pos: v2(d.pos), invest: [0; 4], dormant_until: 0, last_blast: None }),
                Object::WhiteHole(d) => Obj::WhiteHole(WhiteHole { def: d.clone(), pos: v2(d.pos), invest: [0; 4], dormant_until: 0 }),
                Object::GammaRay(d) => {
                    let off = if d.random_start { rng.unit() * 360.0 } else { 0.0 };
                    Obj::Gamma(Gamma { def: d.clone(), pos: v2(d.pos), angle: d.rotation_deg + off, phase: 0.0, offset: off })
                }
                Object::Shockwave(d) => Obj::Shock(Shock { def: d.clone(), pos: v2(d.pos) }),
                Object::Fog(d) => {
                    fog = Some(d.clone());
                    Obj::Fog
                }
                Object::Terrain(d) => {
                    terrain_idx.push(i);
                    Obj::Terrain(Terrain { def: d.clone(), enabled: d.enabled, accel: v2(d.acceleration) })
                }
                Object::Unknown(_) => Obj::Other,
            };
            objs.push(obj);
        }
        let factions = map
            .factions
            .iter()
            .map(|f| FactionState {
                alive: true,
                max_sources: f.max_sources.max(1),
                max_orders: f.max_orders.max(1),
                think_period: f.think_period,
                order_log: Vec::new(),
                next_think: 0,
            })
            .collect();
        let mut sim = Sim {
            tick: 0,
            objs,
            ships: Vec::new(),
            factions,
            status: Status::Playing,
            fog,
            cfg,
            rng,
            next_ship_id: 0,
            events: Vec::new(),
            terrain_idx,
            wormholes,
        };
        // stagger AI start deterministically
        for f in 0..sim.factions.len() {
            sim.factions[f].next_think = 10 + 7 * f as u64;
        }
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
    fn sun_mut(&mut self, i: usize) -> Option<&mut Sun> {
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

    /// Order-target under a world point (for mouse picking): suns, wormholes, holes, supernovas.
    pub fn pick(&self, p: Vec2, slack: f64) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, o) in self.objs.iter().enumerate() {
            let (pos, r) = match o {
                Obj::Sun(s) if s.present() => (s.pos, s.radius),
                Obj::Wormhole(w) => (w.pos, w.size),
                Obj::WhiteHole(w) => (w.pos, w.def.game_size.max(150.0)),
                Obj::Supernova(s) => (s.pos, s.def.start_scale.max(150.0)),
                _ => continue,
            };
            let d = pos.dist(p);
            if d <= r + slack && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    pub fn ship_count(&self, faction: u8) -> usize {
        self.ships.iter().filter(|s| s.faction == faction).count()
    }
    pub fn stock_total(&self, faction: u8) -> u32 {
        self.suns().filter(|(_, s)| s.faction == faction as i8).map(|(_, s)| s.stock).sum()
    }
    pub fn sun_count(&self, faction: u8) -> usize {
        self.suns().filter(|(_, s)| s.faction == faction as i8).count()
    }

    pub fn vision(&self, faction: u8) -> Vision {
        let Some(f) = &self.fog else { return Vision { all: true, circles: Vec::new() } };
        let mut circles = Vec::new();
        for (_, s) in self.suns() {
            if s.faction == faction as i8 && s.present() {
                circles.push((s.pos, f.sun_radius + f.additional_per_ring * s.level.saturating_sub(1) as f64));
            } else if s.faction < 0 && f.unowned_radius > 0.0 && s.present() {
                circles.push((s.pos, f.unowned_radius));
            } else if s.faction >= 0 && s.faction != faction as i8 && f.enemy_radius > 0.0 && s.present() {
                circles.push((s.pos, f.enemy_radius));
            }
        }
        for sh in &self.ships {
            if sh.faction == faction {
                circles.push((sh.pos, f.non_sun_radius));
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
        }
        for o in &self.objs {
            match o {
                Obj::Sun(s) => {
                    mix(s.faction as i64 as u64);
                    mix(s.level as u64);
                    mix(s.progress as i64 as u64);
                    mix(s.stock as u64);
                    mix(s.cap_progress as i64 as u64);
                    mix(s.pos.x.to_bits());
                }
                Obj::Supernova(s) => s.invest.iter().for_each(|v| mix(*v as u64)),
                Obj::WhiteHole(s) => s.invest.iter().for_each(|v| mix(*v as u64)),
                Obj::Gamma(g) => mix(g.angle.to_bits()),
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
            Command::Cancel { faction, sources } => {
                self.check_faction(faction)?;
                for s in sources {
                    if let Some(su) = self.sun_mut(s) {
                        if su.faction == faction as i8 {
                            su.repeat = None;
                        }
                    }
                }
                Ok(())
            }
            Command::Order { faction, sources, target, then, percent, repeat } => {
                self.order(faction, sources, target, then, percent, repeat)
            }
        }
    }

    fn check_faction(&self, faction: u8) -> Result<(), OrderError> {
        match self.factions.get(faction as usize) {
            None => Err(OrderError::UnknownFaction),
            Some(f) if !f.alive => Err(OrderError::Eliminated),
            Some(_) => Ok(()),
        }
    }

    fn order(&mut self, faction: u8, sources: Vec<usize>, target: Target, then: Option<Target>, percent: u8, repeat: bool) -> Result<(), OrderError> {
        self.check_faction(faction)?;
        if sources.is_empty() {
            return Err(OrderError::NoSources);
        }
        let mut srcs = sources;
        srcs.sort_unstable();
        srcs.dedup();
        if srcs.len() > self.factions[faction as usize].max_sources as usize {
            return Err(OrderError::TooManySources);
        }
        if !(1..=100).contains(&percent) {
            return Err(OrderError::BadPercent);
        }
        for &s in &srcs {
            match self.sun(s) {
                Some(su) if su.faction == faction as i8 && su.present() => {}
                _ => return Err(OrderError::NotYourSun),
            }
        }
        if !self.valid_target(target) || then.map(|t| !self.valid_target(t)).unwrap_or(false) {
            return Err(OrderError::BadTarget);
        }
        // rate cap: at most `maxorders` orders per rolling second
        let tick = self.tick;
        let max_orders = self.factions[faction as usize].max_orders as usize;
        {
            let log = &mut self.factions[faction as usize].order_log;
            log.retain(|&t| t + ORDER_WINDOW_TICKS > tick);
            if log.len() >= max_orders {
                return Err(OrderError::TooManyOrders);
            }
        }
        if repeat {
            let standing = self.suns().filter(|(i, s)| s.faction == faction as i8 && s.repeat.is_some() && !srcs.contains(i)).count();
            if standing + srcs.len() > max_orders {
                return Err(OrderError::TooManyRepeats);
            }
        }
        let any_stock = srcs.iter().any(|&s| self.sun(s).map(|x| x.stock > 0).unwrap_or(false));
        if !any_stock && !repeat {
            return Err(OrderError::NoShips);
        }
        self.factions[faction as usize].order_log.push(tick);
        let mut total = 0;
        for &s in &srcs {
            let stock = self.sun(s).unwrap().stock;
            let n = ((stock as u64 * percent as u64 + 99) / 100) as u32;
            let n = n.min(stock);
            if n > 0 {
                self.sun_mut(s).unwrap().stock -= n;
                self.launch(s, n, target, then);
                total += n;
            }
            self.sun_mut(s).unwrap().repeat = if repeat { Some(Repeat { target, then }) } else { None };
        }
        if total > 0 {
            self.events.push(Event::Launch { faction, count: total });
        }
        Ok(())
    }

    fn surrender(&mut self, faction: u8) {
        for i in 0..self.objs.len() {
            let owned = matches!(&self.objs[i], Obj::Sun(s) if s.faction == faction as i8);
            if owned {
                self.neutralize(i, false);
            }
        }
        self.ships.retain(|s| s.faction != faction);
        self.update_status();
    }

    /// Spawn `n` ships of the sun's owner at the sun, heading for `target`.
    fn launch(&mut self, sun: usize, n: u32, target: Target, then: Option<Target>) {
        let (centre, radius, faction) = {
            let s = self.sun(sun).unwrap();
            (s.pos, s.radius, s.faction as u8)
        };
        let tp = self.target_pos(target).unwrap_or(centre);
        let base = (tp - centre).norm_or(Vec2::new(1.0, 0.0));
        for _ in 0..n {
            let ang = self.rng.unit() * 360.0;
            let r = radius * 0.8 * self.rng.unit().sqrt();
            let pos = centre + dir_deg(ang) * r;
            let heading = base.rotate(self.rng.signed_unit() * LAUNCH_SPREAD_DEG);
            let speed_mul = 1.0 + self.rng.signed_unit() * SPEED_JITTER;
            self.spawn(Ship {
                id: 0,
                pos,
                heading,
                push: Vec2::ZERO,
                faction,
                target,
                then,
                speed_mul,
                cooldown: 0,
                last_wormhole: 0,
                alive: true,
                neutral: false,
            });
        }
    }

    fn spawn(&mut self, mut s: Ship) {
        s.id = self.next_ship_id;
        self.next_ship_id = self.next_ship_id.wrapping_add(1);
        self.ships.push(s);
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

    // -------------------------------------------------------------------------------------------- sun state changes
    fn level_up_check(&mut self, j: usize) {
        let mut ev = None;
        {
            let Some(s) = self.sun_mut(j) else { return };
            if s.level < s.max_level && s.progress >= UPGRADE_COST[s.level as usize] {
                s.level += 1;
                s.progress = 0;
                ev = Some((s.level, s.level == s.max_level));
            }
        }
        if let Some((lvl, maxed)) = ev {
            self.events.push(Event::SunUpgraded { sun: j, level: lvl });
            if maxed {
                self.events.push(Event::SunMaxed { sun: j });
            }
        }
    }

    /// Owner loses the sun: it becomes neutral (or is destroyed if fragile and `destroy_fragile`).
    fn neutralize(&mut self, j: usize, destroy_fragile: bool) {
        let tick = self.tick;
        let mut lost = None;
        let mut destroyed = false;
        {
            let Some(s) = self.sun_mut(j) else { return };
            if !s.present() {
                return;
            }
            if s.faction >= 0 {
                lost = Some(s.faction as u8);
            }
            s.faction = -1;
            s.level = 0;
            s.progress = 0;
            s.stock = 0;
            s.cap_faction = -1;
            s.cap_progress = 0;
            s.accum = 0;
            s.repeat = None;
            if destroy_fragile {
                if let Some(f) = &mut s.fragile {
                    f.phase = FragilePhase::Gone { until: tick + f.gone_ticks as u64 };
                    destroyed = true;
                }
            }
        }
        if let Some(f) = lost {
            self.events.push(Event::SunLost { sun: j, faction: f });
        }
        if destroyed {
            self.events.push(Event::SunDestroyed { sun: j });
        }
    }

    /// A ship of `faction` reaches sun `j` and is absorbed.
    fn arrive_at_sun(&mut self, faction: u8, j: usize) {
        let f = faction as i8;
        enum Next {
            Nothing,
            LevelUp,
            Neutralize,
            Captured,
            Hit,
        }
        let next = {
            let s = self.sun_mut(j).unwrap();
            if s.faction == f {
                if s.level < s.max_level {
                    s.progress += 1;
                    Next::LevelUp
                } else {
                    s.stock += 1; // reserve at maximum level
                    Next::Nothing
                }
            } else if s.faction >= 0 {
                let dmg = if s.stock > 0 {
                    s.stock -= 1;
                    (HIT_DAMAGE + 1) / 2
                } else {
                    HIT_DAMAGE
                };
                s.progress -= dmg;
                if s.progress < 0 {
                    if s.level > 0 {
                        s.level -= 1;
                        s.progress = UPGRADE_COST[s.level as usize] / 2;
                        Next::Hit
                    } else {
                        Next::Neutralize
                    }
                } else {
                    Next::Hit
                }
            } else {
                if s.cap_faction == f || s.cap_progress <= 0 {
                    s.cap_faction = f;
                    s.cap_progress += 1;
                } else {
                    s.cap_progress -= 1;
                }
                if s.cap_progress >= CAPTURE_COST {
                    if s.fragile.is_some() {
                        Next::Neutralize // handled below as a destruction
                    } else {
                        s.faction = f;
                        s.level = 0;
                        s.progress = 0;
                        s.stock = 0;
                        s.cap_faction = -1;
                        s.cap_progress = 0;
                        s.accum = 0;
                        Next::Captured
                    }
                } else {
                    Next::Nothing
                }
            }
        };
        match next {
            Next::Nothing => {}
            Next::LevelUp => self.level_up_check(j),
            Next::Hit => self.events.push(Event::SunHit { sun: j }),
            Next::Neutralize => self.neutralize(j, true),
            Next::Captured => self.events.push(Event::SunCaptured { sun: j, faction }),
        }
    }

    // -------------------------------------------------------------------------------------------- terrain
    fn terrain_fx(&self, p: Vec2) -> TerrainFx {
        let mut fx = TerrainFx { speed: 1.0, ..Default::default() };
        for &i in &self.terrain_idx {
            let Obj::Terrain(t) = &self.objs[i] else { continue };
            if !t.contains(p) {
                continue;
            }
            match t.def.kind {
                TerrainKind::Force => fx.accel += t.accel,
                TerrainKind::Speed => fx.speed *= (1.0 + t.def.speed_change / SPEED_CHANGE_UNIT).clamp(0.25, 4.0),
                TerrainKind::Reflect => {}
                TerrainKind::Minefield => {
                    let p = rate_ppm(t.def.death_rate * t.def.density / 100.0);
                    fx.mine_ppm = fx.mine_ppm.saturating_add(p);
                }
                TerrainKind::Confusion => {
                    fx.confuse_faction_ppm = fx.confuse_faction_ppm.saturating_add(rate_ppm(t.def.faction_change_rate));
                    fx.confuse_order_ppm = fx.confuse_order_ppm.saturating_add(rate_ppm(t.def.lose_order_rate));
                }
                TerrainKind::Neutral => fx.neutral = true,
            }
        }
        fx
    }

    fn is_wall(&self, p: Vec2) -> bool {
        self.terrain_idx.iter().any(|&i| matches!(&self.objs[i], Obj::Terrain(t) if t.def.kind == TerrainKind::Reflect && t.contains(p)))
    }

    // -------------------------------------------------------------------------------------------- the tick
    /// Advance one fixed step, applying `cmds` first. Does nothing once the game is decided.
    pub fn step(&mut self, cmds: &[Command]) {
        if self.status != Status::Playing {
            return;
        }
        for c in cmds {
            let faction = match c {
                Command::Order { faction, .. } | Command::Cancel { faction, .. } | Command::Surrender { faction } => *faction,
            };
            if let Err(why) = self.issue(c.clone()) {
                self.events.push(Event::OrderRejected { faction, why });
            }
        }
        self.tick += 1;
        self.update_objects();
        self.produce();
        self.run_ai();
        self.move_ships();
        self.combat();
        self.hazards();
        self.ships.retain(|s| s.alive);
        if self.tick % ORPHAN_RECALL_TICKS == 0 {
            self.recall_orphans();
        }
        self.update_status();
    }

    /// Ships of a faction that owns no sun and sits idle advance on the nearest sun (capturing or hitting it),
    /// so a faction cannot linger forever as a handful of stranded ships.
    fn recall_orphans(&mut self) {
        for f in 0..self.factions.len() as u8 {
            if !self.factions[f as usize].alive || self.sun_count(f) > 0 {
                continue;
            }
            for i in 0..self.ships.len() {
                let sh = self.ships[i];
                if sh.faction != f || sh.target != Target::None {
                    continue;
                }
                let mut best: Option<(f64, usize)> = None;
                for (j, su) in self.suns() {
                    if su.present() {
                        let d = su.pos.dist2(sh.pos);
                        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                            best = Some((d, j));
                        }
                    }
                }
                if let Some((_, j)) = best {
                    self.ships[i].target = Target::Obj(j);
                }
            }
        }
    }

    fn update_objects(&mut self) {
        let tick = self.tick;
        for i in 0..self.objs.len() {
            match &mut self.objs[i] {
                Obj::Sun(s) => {
                    if let Some(or) = &s.orbit {
                        s.pos = orbit_pos(s.centre, or, tick);
                    }
                    if let Some(f) = &mut s.fragile {
                        match f.phase {
                            FragilePhase::Gone { until } if tick >= until => {
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
                Obj::Gamma(g) => {
                    let per_rev = (g.def.period.max(1) * GAMMA_TICKS_PER_PERIOD) as f64;
                    g.phase += 360.0 / per_rev;
                    if g.phase >= 360.0 {
                        g.phase -= 360.0;
                        if g.def.randomization > 0.0 {
                            let j = self.rng.signed_unit() * g.def.randomization;
                            g.offset += j;
                        }
                    }
                    g.angle = g.def.rotation_deg + g.offset + g.phase;
                }
                Obj::Terrain(t) => {
                    if t.def.switch_time > 0 && tick % t.def.switch_time as u64 == 0 {
                        if t.def.kind == TerrainKind::Force && t.def.dir_switch != 0 {
                            t.accel = -t.accel;
                        } else {
                            t.enabled = !t.enabled;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn produce(&mut self) {
        for j in 0..self.objs.len() {
            let mut spawn: Option<(u32, Repeat)> = None;
            let mut cancel = false;
            {
                let Obj::Sun(s) = &mut self.objs[j] else { continue };
                if s.faction < 0 || !s.present() || s.level == 0 {
                    continue;
                }
                s.accum += PRODUCTION_TENTHS[s.level as usize];
                let mut n = 0;
                while s.accum >= PRODUCTION_UNIT {
                    s.accum -= PRODUCTION_UNIT;
                    n += 1;
                }
                if n > 0 {
                    match s.repeat {
                        Some(r) => spawn = Some((n, r)),
                        None => s.stock += n,
                    }
                }
            }
            if let Some((n, r)) = spawn {
                let target_ok = match r.target {
                    Target::Obj(t) => self.sun(t).map(|x| x.present()).unwrap_or(true) && self.valid_target(r.target),
                    other => self.valid_target(other),
                };
                if target_ok {
                    self.launch(j, n, r.target, r.then);
                } else {
                    cancel = true;
                    self.sun_mut(j).unwrap().stock += n;
                }
            }
            if cancel {
                self.sun_mut(j).unwrap().repeat = None;
            }
        }
    }

    fn move_ships(&mut self) {
        let mut ships = std::mem::take(&mut self.ships);
        let mut pending: Vec<Pending> = Vec::new();
        for s in ships.iter_mut() {
            if s.alive {
                self.move_ship(s, &mut pending);
            }
        }
        // ships spawned while moving (none today) would be in self.ships; keep order stable
        let mut extra = std::mem::take(&mut self.ships);
        ships.append(&mut extra);
        self.ships = ships;
        for p in pending {
            match p {
                Pending::Invest { obj, faction, n } => self.invest(obj, faction, n),
            }
        }
    }

    fn move_ship(&mut self, s: &mut Ship, pending: &mut Vec<Pending>) {
        if s.cooldown > 0 {
            s.cooldown -= 1;
        }
        // terrain at the current position
        let fx = self.terrain_fx(s.pos);
        s.neutral = fx.neutral;
        if fx.mine_ppm > 0 && self.rng.chance_ppm(fx.mine_ppm) {
            s.alive = false;
            return;
        }
        if fx.confuse_faction_ppm > 0 && self.rng.chance_ppm(fx.confuse_faction_ppm) {
            let n = self.factions.len() as u32;
            let others = self.rng.below(n - 1) as u8;
            s.faction = if others >= s.faction { others + 1 } else { others };
        }
        if fx.confuse_order_ppm > 0 && self.rng.chance_ppm(fx.confuse_order_ppm) {
            s.target = Target::None;
            s.then = None;
        }
        if fx.accel != Vec2::ZERO {
            s.push += fx.accel * FORCE_SCALE;
            let l = s.push.len();
            if l > PUSH_MAX {
                s.push = s.push * (PUSH_MAX / l);
            }
            s.push = s.push * PUSH_DECAY_INSIDE;
        } else {
            s.push = s.push * PUSH_DECAY_OUTSIDE;
        }

        // resolve the target (lost targets become points)
        if let Target::Obj(j) = s.target {
            if let Some(Obj::Sun(su)) = self.objs.get(j) {
                if !su.present() {
                    s.target = Target::Point(su.pos);
                }
            }
        }
        if let Some(tp) = self.target_pos(s.target) {
            let desired = (tp - s.pos).norm_or(s.heading);
            s.heading = (s.heading + desired * STEER).norm_or(desired);
        }
        let moving = s.target != Target::None;
        let mut newpos = s.pos;
        if moving {
            let vel = s.heading * (SHIP_SPEED * s.speed_mul * fx.speed) + s.push;
            newpos = s.pos + vel;
        } else {
            newpos = newpos + s.push;
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
        s.pos = newpos;

        // wormholes
        if s.cooldown == 0 {
            for k in 0..self.wormholes.len() {
                let wi = self.wormholes[k];
                let (wpos, wsize, wdest) = match &self.objs[wi] {
                    Obj::Wormhole(w) => (w.pos, w.size, w.dest),
                    _ => continue,
                };
                if s.pos.dist2(wpos) < wsize * wsize && s.last_wormhole != wi as u32 + 1 {
                    let di = if wdest >= 0 {
                        wdest as usize
                    } else if self.wormholes.len() < 2 {
                        break;
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
                        s.cooldown = WORMHOLE_COOLDOWN;
                        s.last_wormhole = wi as u32 + 1;
                        if s.target == Target::Obj(wi) {
                            s.target = s.then.take().unwrap_or(Target::Point(s.pos));
                        }
                        self.events.push(Event::WormholeJump { obj: wi });
                    }
                    break;
                }
            }
        }

        // arrival
        match s.target {
            Target::Obj(j) => match self.objs.get(j) {
                Some(Obj::Sun(su)) if su.present() => {
                    if s.pos.dist(su.pos) < su.radius + ARRIVE_MARGIN {
                        let f = s.faction;
                        s.alive = false;
                        self.arrive_at_sun(f, j);
                    }
                }
                Some(Obj::WhiteHole(w)) => {
                    if s.pos.dist(w.pos) < OBJECT_RADIUS {
                        s.alive = false;
                        pending.push(Pending::Invest { obj: j, faction: s.faction, n: 1 });
                    }
                }
                Some(Obj::Supernova(w)) => {
                    if s.pos.dist(w.pos) < OBJECT_RADIUS {
                        s.alive = false;
                        pending.push(Pending::Invest { obj: j, faction: s.faction, n: 1 });
                    }
                }
                _ => {}
            },
            Target::Point(p) => {
                if s.pos.dist(p) < POINT_ARRIVE {
                    s.target = Target::None;
                }
            }
            Target::None => {}
        }
        // idle ships touching a sun are absorbed by it
        if s.alive && s.target == Target::None {
            let mut hit = None;
            for (j, su) in self.suns() {
                if su.present() && s.pos.dist(su.pos) < su.radius + ARRIVE_MARGIN {
                    hit = Some(j);
                    break;
                }
            }
            if let Some(j) = hit {
                let f = s.faction;
                s.alive = false;
                self.arrive_at_sun(f, j);
            }
        }
    }

    fn invest(&mut self, obj: usize, faction: u8, n: u32) {
        let tick = self.tick;
        let fi = (faction as usize).min(3);
        enum Fire {
            Nova,
            White,
        }
        let mut fire = None;
        let mut rng_range = None;
        match &mut self.objs[obj] {
            Obj::Supernova(sn) if tick >= sn.dormant_until => {
                sn.invest[fi] += n;
                if sn.invest[fi] >= sn.def.required_investment {
                    fire = Some(Fire::Nova);
                }
            }
            Obj::WhiteHole(wh) if tick >= wh.dormant_until => {
                wh.invest[fi] += n;
                if wh.invest[fi] >= wh.def.required_investment {
                    rng_range = Some((wh.def.start_return, wh.def.end_return.max(wh.def.start_return)));
                    fire = Some(Fire::White);
                }
            }
            _ => {}
        }
        match fire {
            Some(Fire::Nova) => self.explode(obj, faction),
            Some(Fire::White) => {
                let (a, b) = rng_range.unwrap();
                let count = a + self.rng.below(b - a + 1);
                self.release_burst(obj, faction, count);
            }
            None => {}
        }
    }

    fn explode(&mut self, obj: usize, faction: u8) {
        let tick = self.tick;
        let (pos, def) = match &mut self.objs[obj] {
            Obj::Supernova(sn) => {
                sn.invest = [0; 4];
                sn.dormant_until = tick + sn.def.dormant_period as u64;
                sn.last_blast = Some(tick);
                (sn.pos, sn.def.clone())
            }
            _ => return,
        };
        let r2 = def.blast_radius * def.blast_radius;
        if def.kill_enemy_units {
            for s in self.ships.iter_mut() {
                if s.faction != faction && s.pos.dist2(pos) <= r2 {
                    s.alive = false;
                }
            }
        }
        if def.kill_enemy_suns {
            for j in 0..self.objs.len() {
                let hit = matches!(&self.objs[j], Obj::Sun(s) if s.present() && s.faction >= 0 && s.faction != faction as i8 && s.pos.dist2(pos) <= r2);
                if hit {
                    self.neutralize(j, true);
                }
            }
        }
        self.events.push(Event::Supernova { obj, faction });
        self.spawn_residual(pos, faction, def.unit_residuals);
    }

    fn release_burst(&mut self, obj: usize, faction: u8, count: u32) {
        let tick = self.tick;
        let pos = match &mut self.objs[obj] {
            Obj::WhiteHole(wh) => {
                wh.invest = [0; 4];
                wh.dormant_until = tick + wh.def.dormant_period as u64;
                wh.pos
            }
            _ => return,
        };
        self.events.push(Event::WhiteHole { obj, faction, ships: count });
        self.spawn_residual(pos, faction, count);
    }

    /// Spawn `n` ships of `faction` around `pos`, sending them to the nearest friendly sun.
    fn spawn_residual(&mut self, pos: Vec2, faction: u8, n: u32) {
        let target = self.nearest_own_sun(faction, pos).map(Target::Obj).unwrap_or(Target::None);
        let base = self.target_pos(target).map(|t| (t - pos).norm_or(Vec2::new(1.0, 0.0))).unwrap_or(Vec2::new(1.0, 0.0));
        for _ in 0..n {
            let ang = self.rng.unit() * 360.0;
            let r = 150.0 * self.rng.unit().sqrt();
            let heading = base.rotate(self.rng.signed_unit() * LAUNCH_SPREAD_DEG * 6.0);
            let speed_mul = 1.0 + self.rng.signed_unit() * SPEED_JITTER;
            self.spawn(Ship {
                id: 0,
                pos: pos + dir_deg(ang) * r,
                heading,
                push: Vec2::ZERO,
                faction,
                target,
                then: None,
                speed_mul,
                cooldown: 0,
                last_wormhole: 0,
                alive: true,
                neutral: false,
            });
        }
    }

    fn combat(&mut self) {
        const CELL: f64 = COMBAT_RANGE * 2.0;
        let n = self.ships.len();
        if n < 2 {
            return;
        }
        let cell_of = |p: Vec2| -> (i64, i64) { ((p.x / CELL).floor() as i64, (p.y / CELL).floor() as i64) };
        let key = |c: (i64, i64)| -> i64 { (c.0 + 100_000) * 1_000_000 + (c.1 + 100_000) };
        let mut cells: Vec<(i64, u32)> = Vec::with_capacity(n);
        for (i, s) in self.ships.iter().enumerate() {
            if s.alive && !s.neutral {
                cells.push((key(cell_of(s.pos)), i as u32));
            }
        }
        cells.sort_unstable();
        let r2 = COMBAT_RANGE * COMBAT_RANGE;
        for i in 0..n {
            let a = self.ships[i];
            if !a.alive || a.neutral {
                continue;
            }
            let (cx, cy) = cell_of(a.pos);
            let mut best: Option<(f64, usize)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let k = key((cx + dx, cy + dy));
                    let start = cells.partition_point(|c| c.0 < k);
                    for c in &cells[start..] {
                        if c.0 != k {
                            break;
                        }
                        let j = c.1 as usize;
                        let b = &self.ships[j];
                        if j == i || !b.alive || b.faction == a.faction {
                            continue;
                        }
                        let d = a.pos.dist2(b.pos);
                        if d < r2 && best.map(|(bd, bj)| d < bd || (d == bd && j < bj)).unwrap_or(true) {
                            best = Some((d, j));
                        }
                    }
                }
            }
            if let Some((_, j)) = best {
                let bpos = self.ships[j].pos;
                let bf = self.ships[j].faction;
                self.ships[i].alive = false;
                self.ships[j].alive = false;
                self.events.push(Event::Combat { pos: (a.pos + bpos) * 0.5, a: a.faction, b: bf });
            }
        }
    }

    fn hazards(&mut self) {
        let tick = self.tick;
        for oi in 0..self.objs.len() {
            match self.objs[oi].clone() {
                Obj::Gamma(g) => {
                    let d = dir_deg(g.angle);
                    let half = g.def.width / 2.0;
                    let beam = |p: Vec2, extra: f64| -> bool {
                        let rel = p - g.pos;
                        let fwd = rel.dot(d);
                        let perp = (rel.x * d.y - rel.y * d.x).abs();
                        fwd >= GAMMA_HUB && perp < half + extra
                    };
                    if g.def.kill_units {
                        for s in self.ships.iter_mut() {
                            if s.alive && beam(s.pos, 0.0) {
                                s.alive = false;
                            }
                        }
                    }
                    if g.def.kill_suns {
                        for j in 0..self.objs.len() {
                            let hit = matches!(&self.objs[j], Obj::Sun(s) if s.present() && (s.faction >= 0 || s.cap_progress > 0 || s.fragile.is_some()) && beam(s.pos, s.radius));
                            if hit {
                                self.neutralize(j, true);
                            }
                        }
                    }
                }
                Obj::Shock(sh) => {
                    let d = &sh.def;
                    if tick < d.dormant as u64 || d.period == 0 {
                        continue;
                    }
                    let age = (tick - d.dormant as u64) % d.period as u64;
                    if age >= d.lifetime as u64 {
                        continue;
                    }
                    let reach = d.speed * age as f64 + d.start_offset;
                    let thick = 60.0 + d.speed;
                    if d.kind == "st_incircle" {
                        for s in self.ships.iter_mut() {
                            if s.alive && (s.pos.dist(sh.pos) - reach).abs() < thick {
                                s.alive = false;
                            }
                        }
                    } else {
                        // a straight wall travelling perpendicular to `linerotation`, starting 12000 units behind
                        let dir = dir_deg(d.line_rotation_deg);
                        let at = reach - 12000.0;
                        for s in self.ships.iter_mut() {
                            let along = (s.pos - sh.pos).dot(Vec2::new(-dir.y, dir.x));
                            if s.alive && (along - at).abs() < thick {
                                s.alive = false;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn update_status(&mut self) {
        let nf = self.factions.len();
        let mut has = vec![false; nf];
        for (_, s) in self.suns() {
            if s.faction >= 0 && s.present() {
                has[s.faction as usize] = true;
            }
        }
        for s in &self.ships {
            if s.alive {
                has[s.faction as usize] = true;
            }
        }
        for f in 0..nf {
            if self.factions[f].alive && !has[f] {
                self.factions[f].alive = false;
                self.events.push(Event::Eliminated { faction: f as u8 });
            }
        }
        if !self.factions[0].alive {
            self.status = Status::Lost;
        } else if self.factions.iter().skip(1).all(|f| !f.alive) {
            self.status = Status::Won;
        }
    }
}
