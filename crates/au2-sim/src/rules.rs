//! Tuning numbers. RULES.md deliberately leaves these to the implementer; every value is documented in DECISIONS.md.

/// Fixed simulation rate.
pub const TICKS_PER_SEC: u32 = 60;

/// Ship cruise speed in world units per tick (700 u/s).
pub const SHIP_SPEED: f64 = 700.0 / 60.0;
/// Per-ship speed variation (+-) so swarms look like swarms.
pub const SPEED_JITTER: f64 = 0.08;
/// Heading jitter at launch, degrees (+-).
pub const LAUNCH_SPREAD_DEG: f64 = 5.0;
/// How strongly a ship steers back towards its target each tick (0 = never, 1 = instantly).
pub const STEER: f64 = 0.15;
/// Ships within this distance of an enemy fight (both die).
pub const COMBAT_RANGE: f64 = 36.0;

/// Sun radius by maximum level (bigger maximum = bigger sun).
pub fn sun_radius(max_level: u8) -> f64 {
    match max_level {
        0 | 1 => 220.0,
        2 => 300.0,
        3 => 380.0,
        4 => 440.0,
        _ => 500.0,
    }
}

/// Ship production in tenths of a ship per second, by level (index = level). Level 1 = 1/s, level 5 = 6/s.
pub const PRODUCTION_TENTHS: [i32; 6] = [0, 10, 20, 30, 45, 60];
/// Production accumulator threshold: one ship per (TICKS_PER_SEC * 10) accumulated tenth-ticks.
pub const PRODUCTION_UNIT: i32 = (TICKS_PER_SEC as i32) * 10;

/// Ships needed to advance from level L to L+1 (index = L).
pub const UPGRADE_COST: [i32; 6] = [10, 20, 30, 50, 80, 0];
/// Ships a faction needs to deliver to a neutral sun to take it over.
pub const CAPTURE_COST: i32 = 10;
/// Progress removed by a hostile ship that hits an owned sun with no garrison (half, rounded up, if garrisoned).
pub const HIT_DAMAGE: i32 = 2;

/// Extra reach beyond the sun radius at which a ship counts as having arrived.
pub const ARRIVE_MARGIN: f64 = 10.0;
/// Idle (destination reached) ships this close to a sun's rim are absorbed by it.
pub const POINT_ARRIVE: f64 = 24.0;

/// Wormhole exit offset beyond its radius and re-entry cooldown (ticks).
pub const WORMHOLE_EXIT_MARGIN: f64 = 40.0;
pub const WORMHOLE_COOLDOWN: u16 = 45;

/// Radius within which a ship counts as having arrived at an investable object (white hole / supernova).
pub const OBJECT_RADIUS: f64 = 300.0;
/// Gamma ray: the emitter hub is harmless inside this distance.
pub const GAMMA_HUB: f64 = 700.0;
/// Gamma ray: one full revolution takes `period * GAMMA_TICKS_PER_PERIOD` ticks.
pub const GAMMA_TICKS_PER_PERIOD: u32 = 30;
/// A faction with no suns left sends its idle ships at the nearest sun this often (ticks) so no game can stall.
pub const ORPHAN_RECALL_TICKS: u64 = 600;
/// Fragile suns stay gone for `brokenseconds * FRAGILE_TICKS_PER_UNIT` ticks.
pub const FRAGILE_TICKS_PER_UNIT: u32 = 10;
/// Orbit angular speed: `orbitspeed / ORBIT_SPEED_DIV` degrees per second.
pub const ORBIT_SPEED_DIV: f64 = 2.0;

/// Force terrain: push velocity gained per tick = acceleration * FORCE_SCALE (units per tick, per tick).
pub const FORCE_SCALE: f64 = 0.02;
pub const PUSH_MAX: f64 = 20.0;
pub const PUSH_DECAY_OUTSIDE: f64 = 0.97;
pub const PUSH_DECAY_INSIDE: f64 = 0.99;
/// Speed terrain: multiplier = 1 + speedchange / SPEED_CHANGE_UNIT.
pub const SPEED_CHANGE_UNIT: f64 = 32.0;

/// Max orders issued per rolling second per faction is `maxorders` (from the level).
pub const ORDER_WINDOW_TICKS: u64 = TICKS_PER_SEC as u64;

/// AI think interval in ticks by difficulty 0..3.
pub const AI_THINK_TICKS: [u64; 4] = [150, 90, 45, 20];
