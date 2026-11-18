# Game rules (observable behaviour)

A real-time strategy game of glowing suns and swarms of small ships. This document describes what a player sees and
does. It deliberately gives no formulas or tables: numbers are given as rough ranges, as a player watching the game
would estimate them, and the implementer chooses and tunes the exact values. The game does not need to match any other
game exactly; it needs to be fun, fair and deterministic. Where this document gives a magnitude, staying within it
matters more than the precise value: the game's feel depends on the proportions (ships per capture, ship speed versus
board size, how long special objects take).

## 1. The board
- A flat 2D playfield in world units (our levels span roughly -10000..10000 on each axis). The camera can pan and zoom,
  and fits the whole board when a level starts.
- Objects sit at fixed positions unless stated otherwise: **suns**, plus optional **special objects** (section 6) and
  **terrain zones** (section 7).
- Two to four **factions** play. Faction 0 is the human; the others are driven by the AI. Each faction has a colour.
- There are three **game speeds** (slow, normal, fast), each roughly twice the previous. Speed scales everything that
  happens (production, flight, timers), not the frame rate. Normal is the default; the magnitudes below are at normal.

## 2. Suns
- A sun is owned by one faction or is **neutral**. Neutral suns produce nothing.
- Every sun has a **level** from 0 (empty) up to a per-sun **maximum** (1, 2, 3 or 5 in our levels; levels 4 and 5
  are called **supergiants**). A sun's drawn size follows its **current** level, so suns visibly grow as they develop;
  an empty sun is drawn at roughly level-1 size, with its maximum shown by faint rings marking each level step.
- **Production**: about once a second every owned sun at level 1 or more releases a small **ring** of new ships, evenly
  spaced around its rim. Higher levels release more ships per ring: a level-1 sun makes a couple of ships per second, a
  level-5 sun around eight to ten. The sun visibly pulses on each release. Starting garrisons are likewise laid out
  around their sun.
- **Costs are heavy**: capturing an empty sun, or raising a sun by one level, costs on the order of a **hundred ships**.
  The supergiant steps cost noticeably more, the last step most of all. A level-1 sun therefore needs most of a
  minute of its own production to pay for its next level; the 100 starting ships buy about one capture.
- Each owned sun has two bars, shown on the sun: **health** and **growth**.
  - **Health**: full when the sun is captured. Ordinary suns hold out about equally; supergiants are much tougher,
    the largest toughest of all. Each hostile ship that reaches the sun removes health in proportion to its
    strength. When health reaches zero the sun **dies**: it drops straight to an empty, ownerless sun whatever its
    level (supergiants explode visibly). Damage is shown as an arc on the sun.
  - **Growth**: friendly ships that reach the sun first **repair** missing health, then fill growth; when growth is
    full the sun goes up one level and growth starts again.
- **Capturing an empty sun**: ships of a faction that reach it fill its growth for that faction; when full, the
  faction owns it at level 1. If ships of a different faction arrive while it is being filled, they first drain the
  other faction's progress, then start their own. The previous owner has no special claim.
- A sun at its maximum level with full health **absorbs nothing**: friendly ships that arrive simply stay and circle it
  (section 3). Likewise, when a capture or upgrade completes, friendly ships that were still on their way to that sun
  stop nearby and join its circling cloud instead of being used up.
- Starting state: each faction owns one or more suns at level 1 with an initial stock of ships (our levels use 100).

## 3. Ships and orders
- Ships are small, numerous (hundreds to a few thousand on screen) and drawn as bright dots with short trails. They
  belong to a faction. Every ship is always **visible** somewhere on the board; there is no hidden stock of ships
  inside a sun.
- **Idle ships circle suns.** A ship with nothing to do joins the nearest friendly sun within a moderate distance and
  circles it on a flattened oval (about twice as wide as tall), each ship at a slightly different speed, so every owned
  sun is wrapped in a swirling cloud of its owner's ships. Bigger suns have wider clouds made of several nested ovals; a
  level-1 sun's cloud reaches out to a few times its radius. A cloud travels with a moving sun. An idle ship that is
  too far from any friendly sun hovers where it stopped, jittering slightly.
- **Orders select ships by area, not suns.** The player draws a circle (lasso), or taps a sun of any owner to select
  the area around it (larger for bigger suns). **All** of the player's ships inside are selected: circling ships, ships
  hovering in space, and ships already in flight (which can thus be redirected). The player then taps a target:
  - a sun (friendly: reinforce/upgrade; empty: capture; hostile: attack),
  - a targetable special object (wormhole, white hole, supernova),
  - or empty space, where the swarm stops and hovers until given another order.
  There is no percentage-of-garrison choice, no cap on orders and no automatic repeat; the player's control is where
  they draw the circle. Orders into fogged space are refused (section 6).
- Ships fly towards their target with a little wobble, so a swarm looks loose rather than a tight line. Normal flight
  speed is such that crossing a typical level takes **tens of seconds**. Ships
  **bend around** suns in their way rather than flying through them. Terrain (section 7) and special objects
  (section 6) affect them on the way.
- **Self-defence** (all factions, including the human): when a serious attack is heading for one of a faction's suns,
  part of that sun's circling cloud breaks off by itself, about once a second, and moves out to meet the attackers on
  their side of the sun, until the defenders clearly outnumber the attack. Suns that move along an orbit do not do this.
- **Strength and merging**: when the board becomes very crowded, nearby friendly ships fuse into fewer, larger,
  stronger dots. A fused ship counts as the ships it was made of wherever this document says "ships": it does their
  work when capturing, upgrading, damaging or fighting.

## 4. Combat between ships
- Enemy ships notice each other at close range, swerve towards each other and collide a moment later. A stronger
  (fused) ship survives a collision with a weaker one, weakened by it. Equal numbers cancel out and the larger swarm
  survives with roughly the difference.
- Because idle ships circle their suns, defenders meet attackers in the cloud before the attackers reach the sun.
- Friendly ships never hurt each other. Fights show as small flashes and sparks.

## 5. Winning and losing
- A faction is **out** as soon as it owns no living sun and has too few ships left to capture even one empty sun. It is also out if a single rival owns every sun on the board and is several times
  stronger than it. Games therefore end decisively instead of waiting for the last stray ship.
- The human **wins** when every other faction is out; the human **loses** when out.
- The player may surrender, and the game can be paused. A level can be restarted.
- At the end a **results screen** shows the outcome, simple graphs over time per faction (ships, production, combat
  losses), the game time, and buttons to retry, continue to the next level, or exit.

## 6. Special objects
Each has a fixed position (or an orbit, where stated). Any of them may be omitted from a level.

- **Orbiting sun**: a sun that travels along a closed elliptical path (semi-axes, tilt and speed set by the level)
  around its stored position. Everything else about it is as for an ordinary sun. Ships aimed at it track its motion,
  and its cloud travels with it.
- **Fragile sun**: a sun (fixed or orbiting) that **breaks** when it is killed (health to zero, or hit by a gamma ray).
  Capturing it while empty does not break it. A broken fragile sun is a small wreck for a long time (on the order of a
  couple of minutes, set by the level), during which it cannot be captured; then it rebuilds over a couple of seconds
  as an empty sun.
- **Wormhole**: a circular portal, which may also move along an orbit. Only ships **ordered to** a wormhole use it;
  ships merely flying across it are unaffected. Ships that enter come out of the destination wormhole (a specific
  partner named by the level, or a random other one, chosen per ship so a swarm may split) keeping their speed and
  heading, and then go idle (circle a nearby sun or hover).
- **White hole**: a prize that factions invest ships in. While dormant its prize grows over a few minutes, and the hole
  visibly swells in a few steps. Investing is a tug of war: a rival's ships first remove the current investor's stake,
  then start their own. Once one faction has invested enough (a few hundred ships' worth) the hole **fires**: it
  streams the prize out of one side as a fan of new ships for that faction over several seconds (stronger ships when the
  prize is large). It is then spent for a minute or more and regrows over several seconds.
- **Supernova**: a dormant object that factions invest ships in (around a thousand ships' worth), with the same tug of
  war; it grows visibly with investment. When triggered it **collapses** for a few seconds, pulling ships in from a
  very large radius, then **explodes**: a blast front expands across about the whole map in under a second, shoving
  every ship outward, destroying the other factions' ships (and their suns, if the level says so). The triggering
  faction receives a large group of new ships (several hundred) at the centre. Every surviving ship in range loses its
  order. The supernova then stays dormant for tens of seconds and regrows.
- **Gamma ray**: a **fixed straight lane** across the map, several hundred units wide, at the angle the level gives.
  Roughly every minute (with some randomness, set by the level) it shows a warning for a couple of seconds, then a
  pulse sweeps along the lane in about a second. Any sun in the lane dies, whatever its level (fragile suns break).
  Ships in the lane die only if the level says so (most levels leave ships alone). It does not rotate.
- **Shockwave**: slow walls several hundred units thick (expanding rings, shrinking rings or straight lines, as the
  level says) that creep across the map at about walking pace relative to the board, destroying ships they touch. A
  single wave takes minutes to cross.
- **Fog of war**: when present, a faction sees only what lies within sight of its own **suns** (sight grows with the
  sun's level); ships give no sight. Neutral and enemy suns show a small area around themselves. Orders into fogged
  space are refused. Fog lifts once the human is out of the game.

## 7. Terrain zones
Rectangular tile grids or circular zones that modify ships passing through them:
- **Force**: pushes ships. A rectangular zone pushes in a set direction. A circular zone pushes outward from its centre
  or inward towards it. Some zones turn their push direction by a quarter turn at intervals.
- **Speed**: ships inside move much slower (down to well under half speed) or faster, as the level says. This also
  affects ships circling suns inside the zone.
- **Reflect**: solid walls; ships bounce off them.
- **Minefield**: ships crossing it occasionally die (on the order of one ship in a few dozen per crossing). Mines get used
  up, so areas that many ships have crossed become safe: sending ships through clears a path.
- **Confusion**: ships inside may switch allegiance or lose their order. Converted ships go to the **weakest** faction
  that still has a living sun (helping the underdog); ships that lose their order head for a random sun.
- **Neutral zone**: ships inside are neutral and fight nobody until they leave.

## 8. The AI
Each AI faction plans its moves periodically. Its behaviour should look deliberate rather than busy:
- It waits a few seconds after the start, then plans every several seconds and makes only a few moves each time;
  harder AIs plan more often, make more moves at once and gather ships from more places. It starts gently.
- It upgrades its own suns when safe, captures nearby empty suns, attacks enemy suns it can beat (more boldly when it is
  stronger overall), gathers scattered idle ships into large stacks, and **evacuates** suns it cannot hold.
- It avoids dangerous areas where it can: minefields, gamma-ray lanes, shockwave bands. It uses wormholes when they
  shorten a trip and takes white holes and supernovas into account as targets.
- Difficulty levels 0 (easy) to 3 (hardest) differ in how often it plans, how many moves it makes, and how much it
  wastes; lower difficulties are also handicapped so they grow more slowly.
- The AI must be deterministic given the game seed, and must be able to finish every shipped level.

## 9. Time, determinism and networking-readiness
- The simulation steps at a fixed rate (60 steps per second), independent of rendering.
- Given the same level, seed, difficulty and sequence of timestamped player orders, the game must produce exactly the
  same result on every machine (no floating-point randomness; seeded generator). This enables replays and later
  multiplayer.
- The simulation is headless: it has no dependence on rendering or audio.

## 10. Presentation hints (not rules)
- Suns glow in their faction's colour with a swirling surface, rings marking level steps and an arc showing damage;
  they pulse when they release ships. The target of an order flashes briefly; all suns flash when a level starts.
- Ships are bright dots with short trails; stronger ships are larger dots.
- Supergiants that die explode in a burst of rays and debris in the owner's colour.
- Combat is accompanied by notes from an instrument, chosen from a short sequence that follows a musical bar, so fights
  sound rhythmic; the sequence changes when a faction is winning or losing.
- Upgrading a sun plays a rising tone; a sun reaching the top level plays a larger swell.
