# Design decisions

RULES.md deliberately gives no formulas, and MAPFORMAT.md leaves a few field meanings open. This file records every
interpretation made while implementing them. All numbers live in `crates/au2-sim/src/rules.rs` and can be re-tuned.

Only `cleanroom/public/RULES.md`, `cleanroom/public/MAPFORMAT.md` and files already in this repository were used.

## Determinism
- Fixed 60 Hz step (`Sim::step` = one tick). No wall clock anywhere in `au2-sim`.
- Randomness: one seeded splitmix64 generator inside the `Sim`, drawn in a fixed order. The AI draws from the same one.
- Floats: `f64`, using only `+ - * /` and `sqrt` (all correctly rounded by IEEE 754). `sin`/`cos` are **not** taken from
  libm (those can differ per platform); `math.rs` has its own Taylor-series versions. `state_hash()` hashes the whole
  state for replay/determinism tests.
- Commands are applied at the start of the tick they are given to; a replay is `(tick, Command)` pairs plus the seed.

## Ships and suns
- Stationed ships are a **counter** on the sun (`stock`), not entities; ships only become entities when they leave
  in a swarm. This keeps hundreds of ships cheap and makes "fraction of the ships at the sun" exact.
- Ship speed 700 units/s with +-8 % per-ship variation; launch heading jitter +-5 degrees; ships spawn inside the
  sun disc. Each tick a ship blends its heading 15 % towards the target, so it flies roughly straight, tracks moving
  targets, and recovers after bouncing or being pushed.
- Sun radius by maximum level: 1 -> 220, 2 -> 300, 3 -> 380, 4 -> 440, 5 -> 500.
- Production (ships/s) by level 0..5: 0, 1, 2, 3, 4.5, 6.
- Upgrade cost (ships to go from level L to L+1), L = 0..4: 10, 20, 30, 50, 80.
- A friendly ship arriving at an owned sun **below its maximum** adds 1 progress (never joins the stock); at the maximum
  it joins the stock ("reserve"), where it can be ordered out again.
- Hostile hit: removes 2 progress, or 1 if the sun has garrison stock (the garrison loses one ship). This is how
  "defenders partially offset" damage. When progress goes below 0 the sun drops a level and restarts at half of that
  level's upgrade cost; at level 0 the next hit makes it neutral (stock and standing order lost).
- Capture: neutral suns count a per-faction tally; a ship of the same faction adds 1, a rival subtracts 1. At 10 the
  sun becomes the faction's, **level 0** (so "production stops at level 0" matters) and must be upgraded.
- `health`, `growth`, `orbitingpower` and `awarenessradius` in sun records are parsed but unused. Neutral suns ignore
  their `power`/`startingunitcount` (always level 0, no ships).
- A ship whose target sun disappears (fragile sun destroyed) flies to where it was and stops there.
- A ship that stops at a point inside a sun's radius is absorbed by that sun like any arrival.

## Orders
- An order names 1..`maxsources` own suns, a target, a percentage 1..100 (ceil of stock * pct / 100) and an optional
  `repeat` flag. Targets: any present sun, a wormhole, a white hole, a supernova, or a world point.
- "Modest cap on simultaneous orders": at most `maxorders` (8) orders per rolling second per faction (rate limit), and at
  most `maxorders` suns with a standing repeat order.
- Repeat: while the order stands, every ship the source produces leaves for the target. It ends on `Cancel`, if the
  source sun changes hands, or if the target is lost (sun destroyed).
- An order may carry a `then` target: used with a wormhole target, ships continue to `then` after the jump. Without it
  they stop at the exit.
- Orders to the sun's own position are valid and are how a sun is upgraded from its own stock.
- **Orphaned ships:** a faction with no suns whose ships sit idle sends them (every 10 s) at the nearest sun. Without
  this a handful of stranded ships keeps a faction "alive" forever and a game can never end.

## Combat
- Enemy ships within 36 units pair off and both die; the nearest enemy (ties: lowest ship index) is chosen, ships are
  processed in index order, so results are deterministic. A uniform spatial grid keeps it cheap.

## Special objects
- **Orbit**: angular speed is `orbitspeed / 2` degrees per second (speed 12 = one lap per minute). Position =
  centre + ellipse(orbitx, orbity) rotated by `orbitrotation`, starting at `initialangle`. `curorbitangle` and
  `lastorbitdelta` are runtime state and ignored.
- **Fragile sun**: destroyed when a takeover completes or when it would go neutral. Gone for `brokenseconds * 10`
  ticks (240 -> 40 s, matching the rules' "tens of seconds"), then `rebuildframes` ticks of regrowing, then an empty
  neutral sun. While gone it is not targetable and ships pass through. `fstate`/`fframes`/`destroyedpower` ignored.
- **Wormhole**: ships entering the radius re-emerge at the partner (or a random *other* wormhole for -1) just outside
  its radius, keeping heading. 45-tick re-entry cooldown, and a ship never re-enters the wormhole it last used (this
  stops endless loops when a wormhole sits on the straight line to the target, as in the *Loop* level).
- **White hole**: ships sent at it are absorbed as investment, tallied per faction. At `requiredinvestment` the investor
  gets a burst of a random size in `startreturn..=endreturn` that heads for the nearest friendly sun; then dormant for
  `dormantperiod` ticks (investment ignored). `growthrate` and `gamesize` are cosmetic (`gamesize` is the pick radius).
- **Supernova**: same investment mechanic. On trigger every non-investor ship (if `killenemyunits`) and enemy sun (if
  `killenemysuns`) within `blastradius` is destroyed instantly; the investor receives `unitresiduals` ships.
  `collapseradius`, `startscale`, `endscale` are cosmetic. Dormant for `dormantperiod` ticks afterwards.
- **Gamma ray**: a beam from its position, rotating one full turn in `period * 30` ticks (period 90 = 45 s). My first
  attempt (`period * 5`) wiped every home sun within seconds. It only acts beyond 700 units from the emitter (the hub
  is safe, so a sun on the emitter survives). It hits things within `width / 2` of its centre line (plus a sun's
  radius). Suns hit become neutral level 0 and lose their stock (fragile suns are destroyed); ships die only if
  `killunits`. `randomstarttime` randomises the initial angle; `randomization` adds +-N degrees after each turn.
- **Shockwave**: no shipped level uses it and it has no position field in the format, so it is centred at the origin
  unless `mapposition` is given. `st_incircle`: a ring expanding at `speed` units/tick for `lifetime` ticks, repeating
  every `period` ticks after `dormant`; any other `type` is a straight wall sweeping across. Ships touching it die.
- **Fog of war**: the sim never hides anything; `Sim::vision(faction)` returns what a faction can see (own suns:
  `sunradius + additionalperring * (level-1)`, own ships: `nonsunradius`, plus `unownedradius` / `enemyradius` rings
  when non-zero). The app uses it to hide enemy ships and suns for the human. The AI ignores fog (it sees everything).

## Terrain
- Tile grids: `mapposition` is the bottom-left corner, `data` row-major, row r covers y = pos.y + r * tilesize.
  Circular zones (`outerradius > 0`) are an annulus around `mapposition` (treated as the centre).
- **Force**: ships gain `acceleration * 0.02` units/tick of extra velocity per tick inside, capped at 20 units/tick,
  decaying outside. **Speed**: multiplier `1 + speedchange / 32` (clamped 0.25..4). **Reflect**: wall tiles; a ship
  whose move would enter one has the blocked axis of its heading flipped. **Minefield**: death probability per second
  = `deathrate/100 * density/100`. **Confusion**: `factionchangerate` / `loseorderrate` are percent per second.
  **Neutral**: ships inside neither fight nor can be fought.
- `switchtime > 0` toggles a zone every `switchtime` ticks (a force zone with `dirswitch != 0` flips direction instead).
  `changetype` is ignored.

## Winning
- A faction is eliminated when it owns no sun and has no ship in flight. The human (faction 0) wins when all others
  are eliminated and loses when eliminated. Surrender neutralises the faction's suns and removes its ships. Once a game
  is decided `Sim::step` does nothing. Pause/restart are the app's job (restart = build a new `Sim`).

## The AI (`ai.rs`)
- Runs through the same `Sim::issue` as a human. Factions other than 0 are AI; `SimConfig::human_ai` also drives faction
  0 and `enable_ai = false` turns the AI off (tests).
- Thinks every 150 / 90 / 45 / 20 ticks for difficulty 0..3 (a non-zero `thinkperiod` in the level overrides it).
- Each think: defend threatened suns, feed under-developed suns (from their own stock, then from a neighbour's
  surplus), capture nearby neutrals, attack enemy suns it can beat. Easy AIs over-commit ships (x2.5) and need 2x
  superiority; insane needs 1.15x and sends x1.1. After 5 minutes it gradually accepts even odds (down to 0.9x) so
  mirror matches do not stall.
- Hazards: it estimates extra losses for paths through minefields (+8 % per 250-unit sample) and routes single flights
  through a wormhole when that shortens the trip by 30 %+ (difficulty 2+). **It does not evade gamma rays or
  supernovas** and ignores fog.

## App (`au2-app`)
See the README for controls. Choices are listed there because they are user-visible.
