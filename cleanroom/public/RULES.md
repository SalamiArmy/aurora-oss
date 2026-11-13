# Game rules (observable behaviour)

A real-time strategy game of glowing suns and swarms of small ships. This document describes what a player sees and
does. It deliberately gives no formulas or tables: numbers are given as rough ranges, and the implementer chooses and
tunes them. The game does not need to match any other game exactly; it needs to be fun, fair and deterministic.

## 1. The board
- A flat 2D playfield in world units (our levels span roughly -10000..10000 on each axis). The camera can pan and zoom.
- Objects sit at fixed positions unless stated otherwise: **suns**, plus optional **special objects** (section 6) and
  **terrain zones** (section 7).
- Two to four **factions** play. Faction 0 is the human; the others are driven by the AI. Each faction has a colour.

## 2. Suns
- A sun is owned by one faction or is **neutral**. Neutral suns produce nothing.
- Every sun has a **level** from 0 (undeveloped) up to a per-sun **maximum** (1, 2, 3 or 5 in our levels). Bigger
  maximum = bigger drawn radius.
- An owned sun at level 1 or more **produces ships** steadily for its owner. Higher level = more ships per second.
  Level 1 is on the order of one ship per second; level 5 is several times that. Production stops while the sun is at
  level 0.
- A sun can be **upgraded** by feeding it its owner's ships: ships that reach a friendly sun that is below its maximum
  are absorbed and fill its progress; when full it advances one level. A sun at its maximum absorbs friendly ships
  harmlessly into a reserve (the ships are not wasted entirely but nothing further happens).
- Hostile ships that reach a sun **damage** it: each hostile ship removes some progress, and defenders absorbed by the
  sun's owner partially offset this. When an owned sun's progress drops below zero it loses a level; at level 0 the
  next hit makes it **neutral**. Ships of a faction that reaches a neutral sun begin **capturing** it for themselves.
- Starting state: each faction owns one or more suns at level 1 with an initial stock of ships (our levels use 100).

## 3. Ships and orders
- Ships are small, numerous (hundreds on screen) and drawn as dots or short trails. They belong to a faction.
- The player gives an **order** by selecting one or more of their suns (tap/click or lasso) and then tapping a target:
  another sun (friendly, neutral or hostile), a special object that can be targeted, or empty space (which only sends
  ships towards that point).
- When an order is issued, a fraction of the ships currently stationed at the selected suns (the player may choose a
  fraction such as 25 %, 50 %, 100 %) leave in a **swarm** and fly towards the target in a roughly straight line,
  spreading slightly so they look like a swarm, not a single point.
- Ships only affect the world where they are. A swarm does not need a path: it flies straight, subject to terrain
  (section 7) and special objects (section 6).
- There is a modest cap on simultaneous orders per faction so the player cannot spam.
- Production of a sun that has an order standing can be set to **repeat**: new ships also leave towards the same target
  until the order is cancelled or the target is lost.

## 4. Combat between ships
- When ships of different factions come within a short range of each other they **fight**: each ship that meets an
  enemy is destroyed, together with that enemy. Equal numbers cancel out; the larger swarm survives with the difference.
- Friendly ships never hurt each other. Neutral parties do not exist among ships.
- Combat is resolved continuously; there is no health on individual ships.

## 5. Winning and losing
- A faction is **eliminated** when it owns no suns and has no ships left.
- The human **wins** when every other faction is eliminated; the human **loses** when eliminated.
- The player may surrender, and the game can be paused. A level can be restarted.

## 6. Special objects
Each has a fixed position. Any of them may be omitted from a level.

- **Orbiting sun**: a sun that travels along a closed elliptical path (semi-axes, tilt and speed set by the level)
  around its stored position. Everything else about it is as for an ordinary sun. Ships aimed at it track its motion.
- **Fragile sun**: an orbiting-or-fixed sun that is **destroyed** when it is taken over or attacked down to zero; it is
  gone for a while (tens of seconds, set by the level), then **rebuilds** as an empty neutral sun. While gone, ships pass
  through its place.
- **Wormhole**: a circular portal. A ship entering it re-emerges from another wormhole (a specific partner named by the
  level, or a random other one), keeping its heading. Wormholes can be targets of orders.
- **White hole**: a dormant object that a faction can **invest ships in**. After enough investment it releases a large
  burst of ships for the investing faction at once, then goes dormant again.
- **Supernova**: a dormant object that a faction can invest ships in. When triggered it **explodes**, destroying ships
  (and optionally suns) of other factions within a large radius.
- **Gamma ray**: a rotating beam from a fixed point that periodically sweeps across the board, destroying anything it
  touches (ships, suns or both, as set by the level). It may start at a random phase.
- **Shockwave**: periodic rings or lines that expand across the board and destroy ships they touch.
- **Fog of war**: when present, a faction only sees objects and ships within some distance of its own suns and ships.

## 7. Terrain zones
Rectangular tile grids (or circular zones) that modify ships flying through them:
- **Force**: pushes ships in a set direction (accelerates them).
- **Speed**: makes ships faster or slower.
- **Reflect**: solid walls; ships bounce off them.
- **Minefield**: ships die at a steady rate while inside.
- **Confusion**: ships inside randomly change faction or lose their order.
- **Neutral zone**: ships inside are neutral and fight nobody until they leave.

## 8. The AI
Each AI faction repeatedly decides which of its suns should send ships to which targets. It should:
- upgrade its own suns when safe,
- capture nearby neutral suns,
- attack enemy suns it can beat, and defend threatened suns,
- ignore dangerous areas where it can (minefields, gamma rays) and use wormholes,
- come in difficulty levels 0 (easy) to 3 (insane) by deciding more often and wasting fewer ships.
The AI must be deterministic given the game seed, and must be able to finish every shipped level.

## 9. Time, determinism and networking-readiness
- The simulation steps at a fixed rate (60 steps per second), independent of rendering.
- Given the same level, seed, difficulty and sequence of timestamped player orders, the game must produce exactly the
  same result on every machine (no floating-point randomness; seeded generator). This enables replays and later
  multiplayer.
- The simulation is headless: it has no dependence on rendering or audio.

## 10. Presentation hints (not rules)
- Suns glow in their faction's colour with a swirling surface; ships are bright dots that leave short trails.
- Combat is accompanied by notes from an instrument, chosen from a short sequence that follows a musical bar, so fights
  sound rhythmic; the sequence changes when a faction is winning or losing.
- Upgrading a sun plays a rising tone; a sun reaching the top level plays a larger swell.
