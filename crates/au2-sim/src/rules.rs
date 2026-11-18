//! Tuning numbers. RULES.md deliberately leaves these to the implementer; every value is documented in DECISIONS.md.

/// Fixed simulation rate.
pub const TICKS_PER_SEC: u32 = 60;

// ------------------------------------------------------------------------------------------------ ships
/// Ship cruise speed in world units per tick (300 u/s: a 10000-unit level takes ~33 s to cross).
pub const SHIP_SPEED: f64 = 300.0 / 60.0;
/// Per-ship speed variation (+-) so swarms look like swarms.
pub const SPEED_JITTER: f64 = 0.08;
/// How strongly a ship steers towards where it wants to go each tick (0 = never, 1 = instantly).
pub const STEER: f64 = 0.12;
/// Flight wobble: heading swings by up to this many degrees, at a per-ship phase.
pub const WOBBLE_DEG: f64 = 14.0;
/// Wobble frequency in degrees of phase per tick.
pub const WOBBLE_RATE: f64 = 4.0;
/// Ships flying past a sun that is not their target start bending away this far outside its radius.
pub const AVOID_MARGIN: f64 = 120.0;
/// A ship counts as having arrived at a sun this far outside its radius.
pub const ARRIVE_MARGIN: f64 = 6.0;
/// Ships ordered to empty space spread over a disc of radius `POINT_SPREAD * sqrt(n)` (clamped) around the point.
pub const POINT_SPREAD: f64 = 9.0;
pub const POINT_SPREAD_MAX: f64 = 450.0;
/// ... and stop when this close to their own spot.
pub const POINT_ARRIVE: f64 = 12.0;

// ------------------------------------------------------------------------------------------------ idle ships
/// An idle ship joins the nearest friendly sun whose rim is at most this far away; otherwise it hovers.
pub const JOIN_DISTANCE: f64 = 900.0;
/// Orbit ovals: semi-major axis = sun radius * (ORBIT_MIN + band * ORBIT_BAND_STEP + jitter * ORBIT_BAND_STEP).
pub const ORBIT_MIN: f64 = 2.4;
pub const ORBIT_BAND_STEP: f64 = 0.5;
/// Height / width of the orbit oval.
pub const ORBIT_FLATTEN: f64 = 0.5;
/// Tangential speed of circling ships (units per tick), +- ORBIT_SPEED_JITTER.
pub const ORBIT_SPEED: f64 = 1.6;
pub const ORBIT_SPEED_JITTER: f64 = 0.35;
/// Hovering ships jitter by up to this much per tick.
pub const HOVER_JITTER: f64 = 0.35;

/// Number of nested orbit bands for a sun at `level` (a level-1 cloud has 2 bands: about 2.4..3.4 radii, so the
/// flattened ovals still clear the sun at their top and bottom).
pub fn orbit_bands(level: u8) -> u32 {
    1 + (level.max(1) as u32 + 1) / 2
}

/// Outer extent of a sun's circling cloud (the widest oval's semi-major axis).
pub fn cloud_radius(sun_radius: f64, level: u8) -> f64 {
    sun_radius * (ORBIT_MIN + orbit_bands(level) as f64 * ORBIT_BAND_STEP)
}

// ------------------------------------------------------------------------------------------------ suns
/// Sun radius by **current** level (an empty sun is drawn and collides at level-1 size).
pub fn sun_radius(level: u8) -> f64 {
    match level {
        0 | 1 => 100.0,
        2 => 125.0,
        3 => 150.0,
        4 => 185.0,
        _ => 220.0,
    }
}

/// Ticks between production rings.
pub const RING_TICKS: u32 = TICKS_PER_SEC;
/// Ships per production ring by level (index = level): level 1 = 2/s, level 5 = 9/s.
pub const RING_SHIPS: [u32; 6] = [0, 2, 3, 5, 7, 9];

/// Ships of growth needed to capture an empty sun (it becomes level 1).
pub const CAPTURE_COST: i32 = 100;
/// Ships of growth needed to go from level L to L+1 (index = L; index 0 is the capture).
pub const UPGRADE_COST: [i32; 6] = [CAPTURE_COST, 100, 120, 160, 220, 0];
/// Maximum health by current level. Ordinary suns are equal; supergiants (4, 5) are much tougher.
pub const MAX_HEALTH: [i32; 6] = [100, 100, 100, 100, 200, 320];

// ------------------------------------------------------------------------------------------------ combat
/// Enemy ships notice each other (and swerve together) within this range.
pub const SEEK_RANGE: f64 = 100.0;
/// Enemy ships closer than this collide.
pub const HIT_RANGE: f64 = 12.0;
/// How hard a ship that has seen an enemy steers towards it.
pub const SEEK_STEER: f64 = 0.5;

// ------------------------------------------------------------------------------------------------ self-defence
/// A sun is under serious attack when at least this much enemy strength is heading for it...
pub const DEFENCE_MIN_ATTACK: u32 = 12;
/// ... from within this distance of its centre.
pub const DEFENCE_RADIUS: f64 = 3500.0;
/// Defenders keep breaking off until they outnumber the attack by this factor.
pub const DEFENCE_RATIO: f64 = 1.5;
/// Ticks between defence decisions, and the share of the remaining need sent each time.
pub const DEFENCE_TICKS: u64 = TICKS_PER_SEC as u64;
pub const DEFENCE_SHARE: f64 = 0.5;

// ------------------------------------------------------------------------------------------------ winning
/// A faction that owns every sun is "several times stronger" when it has this many times the ships of a rival.
pub const DOMINANCE_RATIO: u32 = 3;
/// Statistics are sampled every this many ticks (for the results screen).
pub const STATS_TICKS: u64 = 2 * TICKS_PER_SEC as u64;

// ------------------------------------------------------------------------------------------------ crowding
/// Merging starts when the board holds more than this many ship entities.
pub const MERGE_THRESHOLD: usize = 2500;
/// Same-faction idle ships closer than this may fuse...
pub const MERGE_RANGE: f64 = 10.0;
/// ... up to this strength per fused ship.
pub const MERGE_MAX: u32 = 10;
/// How often (ticks) merging runs.
pub const MERGE_TICKS: u64 = 30;

// ------------------------------------------------------------------------------------------------ special objects
/// Wormhole exit offset beyond its radius.
pub const WORMHOLE_EXIT_MARGIN: f64 = 30.0;
/// Radius within which a ship counts as having reached an investable object (white hole / supernova).
pub const OBJECT_RADIUS: f64 = 160.0;
/// Fragile suns stay broken for `brokenseconds * FRAGILE_TICKS_PER_UNIT` ticks (240 -> 2 minutes).
pub const FRAGILE_TICKS_PER_UNIT: u32 = TICKS_PER_SEC / 2;
/// Orbit angular speed: `orbitspeed / ORBIT_SPEED_DIV` degrees per second.
pub const ORBIT_SPEED_DIV: f64 = 2.0;

/// Gamma ray: average interval between pulses is `period * GAMMA_TICKS_PER_PERIOD` ticks (90 -> 60 s).
pub const GAMMA_TICKS_PER_PERIOD: u32 = 40;
/// Gamma ray: warning before a pulse, and the time the pulse takes to sweep the lane.
pub const GAMMA_WARN_TICKS: u64 = 2 * TICKS_PER_SEC as u64;
pub const GAMMA_SWEEP_TICKS: u64 = TICKS_PER_SEC as u64;
/// Gamma ray: the lane runs this far either side of the emitter position.
pub const GAMMA_HALF_LENGTH: f64 = 16000.0;

/// White hole: prize grows from `startreturn` to `endreturn` over this many ticks, in `WHITE_STEPS` visible steps.
pub const WHITE_GROW_TICKS: u64 = 180 * TICKS_PER_SEC as u64;
pub const WHITE_STEPS: u32 = 4;
/// White hole: firing streams the prize over this many ticks; after the spent period it regrows over `WHITE_REGROW`.
pub const WHITE_FIRE_TICKS: u64 = 5 * TICKS_PER_SEC as u64;
pub const WHITE_REGROW_TICKS: u64 = 6 * TICKS_PER_SEC as u64;
/// Minimum spent time (the level's `dormantperiod` is in tenths of a second and raised to this).
pub const WHITE_MIN_SPENT: u64 = 60 * TICKS_PER_SEC as u64;
/// A white-hole prize above this many ships is streamed as ships of strength 2 (above 2x, strength 3, ...).
pub const WHITE_STRONG_ABOVE: u32 = 200;

/// Supernova: collapse lasts this long, the blast front then crosses `blastradius` in `NOVA_BLAST_TICKS`.
pub const NOVA_COLLAPSE_TICKS: u64 = 3 * TICKS_PER_SEC as u64;
pub const NOVA_BLAST_TICKS: u64 = 45;
/// Supernova: pull speed during the collapse (units per tick) and shove speed of the blast front.
pub const NOVA_PULL: f64 = 3.0;
pub const NOVA_SHOVE: f64 = 14.0;
/// Supernova dormant time is `dormantperiod * NOVA_TICKS_PER_UNIT` ticks (180 -> 30 s), then it regrows.
pub const NOVA_TICKS_PER_UNIT: u64 = 10;

/// Shockwave speed in world units per tick when the level gives none (100 u/s: walking pace on a 10000 board).
pub const SHOCK_DEFAULT_SPEED: f64 = 100.0 / 60.0;
/// Shockwave wall thickness.
pub const SHOCK_THICKNESS: f64 = 350.0;

// ------------------------------------------------------------------------------------------------ terrain
/// Force terrain: push velocity gained per tick = acceleration * FORCE_SCALE (units per tick, per tick).
pub const FORCE_SCALE: f64 = 0.002;
/// Push never exceeds ~80 % of ship speed, so ships can always make (slow) headway against a current.
pub const PUSH_MAX: f64 = 4.0;
pub const PUSH_DECAY_OUTSIDE: f64 = 0.95;
pub const PUSH_DECAY_INSIDE: f64 = 0.98;
/// Speed terrain: multiplier = 1 + speedchange / SPEED_CHANGE_UNIT, clamped to [SPEED_MIN, SPEED_MAX].
pub const SPEED_CHANGE_UNIT: f64 = 32.0;
pub const SPEED_MIN: f64 = 0.3;
pub const SPEED_MAX: f64 = 3.0;
/// Minefields: each cell (tile, or a MINE_CELL square of a circular zone) starts with
/// `MINES_BASE + density / 10` mines; a ship inside dies with probability `deathrate * MINE_PPM_PER_RATE` ppm per
/// tick, scaled by the share of mines left. Every death uses one mine.
pub const MINE_CELL: f64 = 500.0;
pub const MINES_BASE: u32 = 3;
pub const MINE_PPM_PER_RATE: f64 = 2.0;

// ------------------------------------------------------------------------------------------------ AI
/// AI: first plan after this many ticks.
pub const AI_START_TICKS: u64 = 4 * TICKS_PER_SEC as u64;
/// AI plan interval in ticks by difficulty 0..3.
pub const AI_THINK_TICKS: [u64; 4] = [600, 420, 300, 210];
/// AI production handicap by difficulty, in percent of normal ring size.
pub const AI_PRODUCTION_PCT: [u32; 4] = [60, 80, 100, 100];
