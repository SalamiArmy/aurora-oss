# Design decisions

RULES.md deliberately gives no formulas, and MAPFORMAT.md leaves a few field meanings open. This file records every
interpretation made while implementing them. All numbers live in `crates/au2-sim/src/rules.rs` and can be re-tuned.

Only `cleanroom/public/RULES.md`, `cleanroom/public/MAPFORMAT.md` and files already in this repository were used.

## Determinism
- Fixed 60 Hz step (`Sim::step` = one tick). No wall clock anywhere in `au2-sim`.
- Randomness: one seeded splitmix64 generator inside the `Sim`, drawn in a fixed order. The AI draws from the same one.
- Floats: `f64`, using only `+ - * /` and `sqrt` (all correctly rounded by IEEE 754). `sin`/`cos`/`atan2` are **not** taken
  from libm (those can differ per platform); `math.rs` has its own Taylor-series and polynomial versions. `state_hash()` hashes the whole
  state for replay/determinism tests.
- Commands are applied at the start of the tick they are given to; a replay is `(tick, Command)` pairs plus the seed.

## Ships and suns
RULES.md was substantially revised in `cb9bf25` (heavier economy, visible ships, area orders). Everything below follows
the revised text; the earlier interpretations (hidden stock, percentage orders, repeat orders, rate caps) are gone.
- **Every ship is an entity** (`Sim::ships`, kept sorted by `id`). A ship is flying (`Mode::Fly`, it has a target),
  circling a sun (`Mode::Orbit`, idle) or hovering (`Mode::Hover`, idle). There is no hidden stock.
- Ship speed 300 units/s (+-8 % per ship): a 10000-unit level takes ~33 s to cross ("tens of seconds"); neighbouring
  suns (1200..2500 apart) are 4..8 s apart. Each tick a ship steers 12 % towards its goal; a per-ship sinusoidal wobble
  of +-14 degrees keeps swarms loose.
- **Bending around suns**: inside `radius + 120` of a sun that is not its target a flying ship steers away and to the
  side it is already passing on; as a hard guarantee it is never placed inside such a sun.
- **Circling**: an idle ship joins the nearest friendly sun whose rim is within 900 units (else it hovers, jittering by
  up to 0.35 units a tick). The oval's semi-major axis is `radius * (2.4 + (band + random) * 0.5)` with
  `1 + (level + 1) / 2` bands (level 1: 2 bands, 2.4..3.4 radii; level 5: 4 bands); its height is half its width, so it
  still clears the sun at top and bottom. Tangential speed 1.6 units/tick +-35 % per ship, all circling the same way.
  A ship that has settled sticks to its oval (so the cloud travels with an orbiting sun); a newcomer flies onto it.
  Hovering idle ships re-check every 30 ticks whether a friendly sun is near enough to join; ships ordered to empty
  space are *held* and never join on their own.
- Ships ordered to a point spread over a disc of radius `9 * sqrt(n)` (max 450) so a swarm hovers as a loose cloud.
- **Sun size by current level**: radius 100 / 125 / 150 / 185 / 220 for levels 1..5; an empty sun is level-1 size.
  The renderer draws faint rings at the radii of the levels still to come.
- **Production**: once per second (per sun, from its own timer, so suns do not all pulse together) a ring of
  2 / 3 / 5 / 7 / 9 ships (levels 1..5) leaves evenly spaced around the rim, rotated 37 degrees per ring, and joins the
  sun's cloud. Starting garrisons are placed on the sun's orbits.
- **Costs**: capture 100; upgrades 1->2 100, 2->3 120, 3->4 160, 4->5 220. A level-1 sun pays for its next level in
  50 s. **Health** by current level: 100 for levels 1..3, 200 for level 4, 320 for level 5 (`rules::MAX_HEALTH`).
- **Arrival** (ships ordered to a sun, within `radius + 6`): own sun: repair first, then growth; at full growth the sun
  gains a level (health refilled, radius grows). Enemy sun: health falls by the ship's strength; at 0 the sun dies:
  empty, ownerless, level 0, growth and capture progress cleared. Empty sun: the capture tug of war (a rival first
  drains the other faction's progress, then starts its own); at 100 the faction owns it at level 1, full health. A fused
  ship whose strength is not all needed survives with the rest and joins the cloud (or goes idle).
- **Surplus**: a capture or upgrade that completes marks `(sun, faction)` for the rest of the tick; every friendly ship
  still flying to that sun (and any arriving later in the same tick) joins its cloud instead of being used. A sun at its
  maximum with full health uses nothing: arrivals simply join its cloud.
- Circling ships do not feed their sun by themselves; the player (or AI) orders them into it.
- `health`, `growth`, `orbitingpower` and `awarenessradius` in sun records are parsed but unused. Neutral suns ignore
  their `power`/`startingunitcount`; owned suns start at `max(power, 1)` (an owned sun is never empty).

## Orders
- `Command::Order { faction, ships: Vec<u32>, target }`: the ship ids selected by area. Ids that are not the faction's
  live ships are ignored; if none remain the order is rejected (`NoShips`). There is no cap, percentage or repeat.
  Ships in flight are simply re-targeted. Ids (rather than re-evaluating an area when the order applies) mean the ships
  the player saw highlighted are exactly the ones that go, and a replay stays exact.
- Selecting: `Sim::ships_in_circle(faction, centre, r)`; tapping a sun selects a circle of `cloud_radius + 60` (larger
  for bigger suns). A lasso drag is a ring whose diameter runs from the press point to the pointer (no freehand trail), so the
  ring always fits exactly between them and updates live while dragging.
- Targets: any present sun, a wormhole, a white hole, a supernova, or a world point. With fog of war the target's
  position must be visible to the ordering faction (`Fogged`); suns are always visible through their own small area.

## Self-defence
- Checked once a second per owned, non-orbiting sun (staggered by sun index). The attack is the strength of enemy ships
  *ordered to this sun* within 3500 units. At 12 or more ("serious"), circling ships of that sun (those on the
  attackers' side first) break off - half of what is still missing, at least 8 - and fly, held, to a point
  `cloud_radius + 150` out towards the attackers' centroid, until the defenders reach 1.5x the attack. When no attack
  remains they return to circling. Ordering a defender gives it back to the player. Applies to every faction.

## Combat
- Enemy ships within 100 units steer hard (50 % a tick) towards the nearest one - circling and hovering ships too - and
  collide within 12 units. A collision removes `min(a, b)` strength from both (equal ships both die; a stronger fused
  ship survives weakened). Ships are processed in index order, ties by lowest index, so results are exact.
- Neighbour search: a coarse 200-unit map of which factions are where first, so ships with no enemy nearby skip the
  fine search; then a faction-keyed sorted grid.
- Per-faction `combat_losses` feed the results screen.

## Merging under crowding
- Every 30 ticks, if the board holds more than 2500 ship entities, idle (circling or hovering, not defending) ships of
  the same faction, same sun and same hold state within 10 units fuse, up to strength 10; the lower index absorbs the
  other. Strength is conserved and counts everywhere a ship would. Fused ships are drawn as larger dots.

## Game speeds
- Slow / normal / fast run 30 / 60 / 120 fixed steps per real second (app side; the sim is unchanged, so speed never
  affects determinism or replays).

## Winning
- Checked every tick. A faction is out when it owns no present sun and its total strength is below the capture cost
  (100), or when one other faction owns every present sun (no empty ones) and has at least 3x its strength. An out
  faction's ships are removed. Human out = lost; all others out = won. Surrender kills the faction's suns (they drop
  to empty) and removes its ships. Once decided `Sim::step` does nothing.
- Statistics (`Sim::stats`) are sampled every 2 s and at the end: ship strength, production per second, cumulative
  combat losses and sun count per faction. The results screen draws the first three as graphs.

## Special objects
- **Orbit**: angular speed is `orbitspeed / 2` degrees per second (speed 12 = one lap per minute). Position =
  centre + ellipse(orbitx, orbity) rotated by `orbitrotation`, starting at `initialangle`. `curorbitangle` and
  `lastorbitdelta` are runtime state and ignored. Orbiting suns do not self-defend.
- **Fragile sun**: breaks only when it dies (health 0, gamma ray, supernova); capturing it empty is fine. Broken for
  `brokenseconds * 30` ticks (our 240 -> 2 minutes, "a couple of minutes"), a wreck that cannot be targeted (ships
  flying to it go idle); then it rebuilds over `rebuildframes` ticks (256 -> 4.3 s) into an empty sun.
- **Wormhole**: only ships *ordered to* it enter (within its radius); others fly across untouched. They come out just
  beyond the destination's radius keeping their heading, and go idle (join a nearby sun or hover). A random destination
  (`-1`) is drawn per ship among the other wormholes. The old `then` follow-up is gone.
- **White hole**: `Growing` (accepts investment; the prize steps from `startreturn` to `endreturn` in 4 visible steps
  over 3 minutes) -> `Firing` (the stake reached `requiredinvestment`: the current prize streams out over 5 s as a
  70-degree fan towards the winner's nearest sun; above 200 ships it comes as stronger ships of strength
  `1 + prize / 200`) -> `Spent` (`max(60 s, dormantperiod * 10 ticks)`) -> `Regrow` (6 s). Investment is a single
  stake with the tug of war. `growthrate` is ignored; `gamesize` is the base radius. No shipped level has one.
- **Supernova**: same tug of war on `requiredinvestment`; its radius grows from `startscale/2` to `endscale/2` with the
  stake. Triggered: a 3 s collapse pulls every ship within `collapseradius` towards it at 3 units/tick; then the blast
  front expands to `blastradius` in 45 ticks. As it passes a ship: other factions' ships die (`killenemyunits`), the rest
  are shoved outward and lose their orders; other factions' suns die (`killenemysuns`). The trigger gets
  `unitresiduals` new ships at the centre. Dormant for `dormantperiod * 10` ticks (our 180 -> 30 s), then ready again.
- **Gamma ray**: a fixed lane through its position at `rotation` degrees, `width` wide, 16000 units each way. Mean
  interval `period * 40` ticks (90 -> 60 s), each next one jittered by +-`randomization` percent; the first comes after
  a full interval, or with `randomstarttime` at a seeded random point in the last three quarters of it. A 2 s warning,
  then the pulse sweeps the lane in 1 s; suns it passes die whatever their level (`killsuns`), ships only with
  `killunits`. Empty suns in the lane lose capture progress; fragile ones break.
- **Shockwave**: no shipped level uses it. `speed` is units per second (0 -> 100 u/s, walking pace), walls 350 thick.
  A `type` containing "in" and "circle" (`st_incircle`) is a ring shrinking from `startoffset` (0 -> 12000); another
  "circle" type expands from `startoffset`; anything else is a straight wall along `linerotation` starting 12000 units
  behind the position plus `startoffset`. `dormant`, `period` (0 = once) and `lifetime` (0 = until it has travelled
  24000) are ticks. Ships touching a wall die; suns are unaffected.
- **Fog of war**: sight comes only from suns: own suns see `sunradius + additionalperring * (level-1)`; empty and enemy
  suns show `unownedradius` / `enemyradius` around themselves, or 2.5 sun radii when those are 0. `nonsunradius` is
  ignored (ships give no sight). Fog lifts once the human is out or the game is decided. The AI plans with full
  knowledge, but its orders obey the same refusal rule.

## Terrain
- Tile grids: `mapposition` is the bottom-left corner, `data` row-major, row r covers y = pos.y + r * tilesize.
  Circular zones (`outerradius > 0`) are an annulus around `mapposition` (treated as the centre).
- **Force**: push gained per tick = `acceleration * 0.002`, capped at 4 units/tick (80 % of ship speed, so ships can
  always make headway; a stronger push trapped whole swarms in *Eddy*), decaying 2 % a tick inside and 5 % outside.
  Circular zones push radially: outward if `acceleration.x + acceleration.y >= 0`, else inward, with strength
  `|acceleration|`. A force zone with `dirswitch != 0` turns its push a quarter turn every `switchtime` ticks (a radial
  zone then swirls). Circling ships are not pushed; hovering ones drift.
- **Speed**: multiplier `1 + speedchange / 32`, clamped 0.3..3; it also scales the angular speed of ships circling
  inside the zone.
- **Reflect**: wall tiles; a ship whose move would enter one has the blocked axis of its heading flipped.
- **Minefield**: each tile (or 500-unit cell of a circular zone) holds `3 + density / 25` mines. A ship inside dies
  (strength -1) with probability `deathrate * 2 * density / 100` ppm per tick, scaled by the share of mines left in its
  cell; each death uses one mine, so swarms clear a path. *Abyss* (deathrate 20, ~3000-unit crossing) loses about one
  ship in 40 per crossing at first.
- **Confusion**: `factionchangerate` / `loseorderrate` are percent per second. A converted ship joins the weakest (by
  total strength) other faction that still owns a sun, idle; a ship that loses its order flies to a random present sun.
- **Neutral**: ships inside neither fight nor can be fought.
- `switchtime > 0` toggles other zones on/off every `switchtime` ticks. `changetype` is ignored.

## The AI (`ai.rs`)
- Runs through the same `Sim::issue` as a human, with ship-id orders. Factions other than 0 are AI; `SimConfig::human_ai`
  also drives faction 0 and `enable_ai = false` turns the AI off (tests).
- First plan after 4 s (staggered per faction), then every 10 / 7 / 5 / 3.5 s (+ up to 25 % jitter) for difficulty
  0..3 (a non-zero `thinkperiod` overrides it). Moves per plan 1 / 2 / 2 / 3, but only `1 + plans / 4` early on ("starts
  gently"); a move may gather from 1 / 2 / 3 / 4 suns (nearest first; if those fall short, the biggest stacks).
- Each plan scores candidate moves and makes the best few, never spending the same ships twice nor sending two moves to
  one target: evacuate suns facing more than 1.4x their cloud + health (difficulty 1+); repair / upgrade safe suns from
  their own cloud (or neighbours'); capture empty suns (cost + 1.2x nearby enemies, times a waste factor
  1.5 / 1.3 / 1.15 / 1.1); attack enemy suns with one decisive wave (full health + 1.25x defenders + half of what the sun
  makes during the flight, times a margin 1.7 / 1.45 / 1.3 / 1.2 divided by the square root of its strength ratio to the
  strongest rival, up to 20 % lower after 6 minutes so games do not stall); invest in white holes / supernovas when it
  can afford it; hunt big stray stacks of factions that lost their suns; gather its own loose hovering ships into the
  nearest cloud; move idle stacks from rear suns to the front; and **stage**: when the best attack is unaffordable from
  its nearest suns but the empire has enough, bring idle stacks to the own sun nearest the target.
- Hazards (difficulty 1+): routes through minefields cost extra ships and score lower; targets in a gamma lane whose
  pulse is due before arrival + 10 s, routes through a lane under warning, and targets a shockwave wall will reach are
  skipped. Wormholes (difficulty 2+): a flight goes to a wormhole when entering it and flying on from the exit is 30 %+
  shorter; the ships come out idle and a later plan sends them on.
- Handicap: AI suns' rings are 60 / 80 / 100 / 100 % size for difficulty 0..3 (accumulated, so nothing is rounded away).
- Verified by `tests/levels_ai.rs`: every shipped level finishes against an idle human within 20 minutes, and at least
  80 % of hardest-AI mirror matches are decided within 20 minutes (31 of 32 at the time of writing).

## App (`au2-app`)
See the README for controls. Choices are listed there because they are user-visible.
- The selection is a sorted list of ship ids; it is cleared when an order is sent, and dead ships drop out of it.
- The results screen replaces the old victory/defeat box: outcome, game time, graphs (ships, production, combat
  losses) per faction from `Sim::stats`, and Retry / Next level (after a win) / Exit.
- A dying sun shows a burst of rays and debris in its owner's colour, larger for supergiants. All suns flash (a ping)
  when a level starts and the target of an order flashes.

## 3D presentation (`view3d.rs`, `draw3d.rs`, `render.rs`)
The owner asked for a more 3D look: parallax stars and 3D models for the suns. The board stays a 2D simulation; this is
presentation only and nothing in `au2-sim` changed.
- **Software 3D, not the GPU.** wgpu was the alternative. Rejected for now because (1) on the GNU Windows toolchain its
  recent `windows`/`windows-sys` dependencies are likely to need `dlltool`, the problem that pins softbuffer to 0.4.6
  (not tried: adding wgpu was not attempted); (2) Android would
  trade the working `ANativeWindow` presenter for a Vulkan/GLES surface that cannot be tried here without a device;
  (3) the headless `--snapshot` PNG and the render tests would need an offscreen GPU adapter, which CI runners and
  this machine may not have. The software path keeps all three working unchanged. The cost is CPU time: 5-13 ms per
  1280x800 frame in release on this machine (fog levels are the heaviest), and much less on a phone, whose canvas is
  drawn at about 480 px on the short side and scaled up. `au2-app` is built at `opt-level = 2` in dev builds too.
- **Camera**: perspective, 32 degree vertical field of view, tilted 48 degrees from straight down towards the far
  (+y) edge. `Game::zoom` is now "pixels per world unit at the look-at point"; `to_screen` / `to_world` project and
  ray-cast onto the board plane, so picking, lasso circles and drag-panning work on the tilted board. `fit` searches the
  largest zoom that keeps the board's corners on screen, clear of the top info strip, and centres them vertically.
- **Suns** are ray-traced spheres (exact under perspective), centred on the board so the circling ships ring their
  equator: a key light from the upper left, emissive glow (brighter when owned and with level), a rim and a specular
  highlight, and a slowly spinning banded noise surface generated in code (no PNG decoder needed). They are drawn 1.4x
  their simulated radius so they read as solid bodies at the fitted zoom (clouds start at 2.4 radii, so they clear).
  White holes and supernovas are spheres too. A broken fragile sun is a few tumbling rocks: the faceted rock mesh in
  `assets/rock.bin` (compiled in with `include_bytes!`, so Android needs no asset plumbing), flat-shaded by a small
  depth-tested triangle rasterizer.
- **Depth**: solid surfaces write a per-pixel depth buffer on the canvas; ships, rings on the board and level rings are
  depth-tested, so ships behind a sun are hidden by it. Board-plane things (terrain tiles, gamma lanes, shockwaves,
  wormhole portals, fog discs, lasso) are projected polygons / ellipses, clipped at the near plane.
- **Starfield**: four layers of stars on planes 1500 to 40000 units *below* the board, seen through the same camera,
  so deeper layers move less when panning and zooming (true parallax), plus a few faint nebulae on the deepest layer.
  Stars come from a hashed lattice per layer; when zoomed out the lattice is sampled more coarsely (only every 2nd,
  4th... cell), which keeps a subset of the same stars instead of re-rolling them.
- HUD, bars, pips and text stay flat screen overlays. The menu keeps the old 2D star background.

## Android front end
None of this has run on a device; it is verified only by `cargo check --target aarch64-linux-android -p au2-android`
(with the MSVC host toolchain: the GNU one lacks `dlltool` for the host-side build dependencies) plus the desktop tests.
- **Structure**: `au2-app` is now a library (all game/render/input code) plus the desktop `main.rs`. A thin `au2-android`
  cdylib crate holds `android_main`; it is empty on other targets so the workspace still builds on the desktop. A
  `cdylib` in `au2-app` itself was rejected so the desktop build stays exactly as it was.
- **Activity**: winit's `android-native-activity` backend (no Java/Kotlin, no Gradle). Suspend drops the window/surface and
  opens the pause overlay; resume builds new ones (`ApplicationHandler::suspended/resumed`). Rotation is handled in
  the app (`configChanges`), which re-fits the board when portrait/landscape flips.
- **Presenting**: softbuffer 0.4.6 (pinned for the GNU desktop toolchain, see `au2-app/Cargo.toml`) has no Android
  backend and 0.4.7+ needs `dlltool` on GNU Windows, so `present.rs` writes into the `ANativeWindow` via the `ndk` crate.
  The buffer is canvas-sized and the compositor scales it to the screen.
- **Assets**: levels are read through a `LevelSource` trait. Desktop = `FsSource` (the upward directory search is
  unchanged); Android = the APK asset manager (`levels/levels.txt`, `levels/<stem>.au2`). `levels/` and `assets/` are
  copied to `crates/au2-android/apk-assets/` (git-ignored) by `tools/build-android.sh` and packed by cargo-apk. `assets/`
  (sound) is bundled but unused: **audio is off on Android**, because rodio/cpal need `oboe` (C++, NDK) and so cannot
  even be type-checked locally; `au2-android` depends on `au2-app` with `default-features = false`. (The desktop app does
  not play sound either at the moment: `audio.rs` is not wired into `main`.)
- **Touch** (`touch.rs`, translated into the same `Input` as the mouse): tap = left click; one finger drag = draw a
  lasso circle (from anywhere, since ships can be anywhere); two fingers = pan with the midpoint + pinch zoom; presses on
  the bottom bar / menus never draw. With nothing selected a tap on a sun selects its area; with a selection a tap
  anywhere is the order (own sun = reinforce / upgrade). Pick radius is 22 px (6 on desktop), drag slop 14 px (8). No
  hover cursor in touch mode.
- **HUD scaling**: on Android the canvas is `floor(short side / 480)` times smaller than the screen (UI px are
  "logical"); touch mode has a 60 px bottom bar, 42-44 px buttons, and the bar's buttons shrink to fit narrow
  (portrait) screens. **All** selects all your ships and **None** clears the selection, on desktop too.
  Android Back = Esc (deselect first, then the menu).
- **Packaging**: `cargo-apk` 0.10 (aapt2/zipalign/apksigner from the SDK; no Gradle) rather than `cargo-apk2`/`cargo-ndk`:
  the better-known tool, whose manifest settings live in `[package.metadata.android]` of `au2-android`. I could not
  fetch its documentation while writing this, so key names and the `CARGO_APK_RELEASE_KEYSTORE[_PASSWORD]` variables
  are from memory and are the first thing to check if the workflow fails.
- **Signing**: cargo-apk cannot choose a key alias, so it always signs with a throwaway debug keystore generated per run
  (it refuses to build a release otherwise), and `tools/build-android.sh` then re-signs with `apksigner` using
  `ANDROID_KEYSTORE_B64` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_ALIAS` when all three exist (key password assumed
  equal to the store password). Without them it keeps the debug-signed APK and prints a warning (log + workflow annotation).
- **CI**: `release.yml` gets an `android` job (artifact `android-arm64`, published as
  `aurora-oss-<tag>-android-arm64.apk`); `ci.yml` has the same job build-only. The release job still publishes the desktop
  zips if the Android job fails. SDK/NDK are installed with the runner's `sdkmanager` (platform 33, build-tools 34.0.0,
  NDK 26.3.11579264); shared steps live in `.github/actions/android-setup`.
