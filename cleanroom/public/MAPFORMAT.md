# Level file format

Levels are plain text (`levels/*.au2`). `levels/levels.txt` lists them in play order, one per line:
`Constellation|Level name|file stem|difficulty 0-3|bonus 0/1`. This document describes exactly the structures that
`tools/gen_levels.py` writes. The sample files in `levels/` are the reference; read them alongside this page.

## Syntax
A level is a nested value:

```
{ name(type)=value, name(type)=value, ... }      object / struct
[count, item, item, ...]                          array: the first element is the item count
name(type)=value                                  a field; `type` is informational (s4/u4/u1 integers, float,
                                                  bool, vector2, orderedarray<T>, enum names, ...)
```
Fields are separated by commas; whitespace is insignificant outside numbers. Booleans are `true`/`false`.
Floats are written with four decimals. Keys that the loader does not know must be ignored (forward compatibility).

## Top level
```
{
  factions(orderedarray<aumapfaction>)=[N, { factionobject(faction)=faction={id(factionid)=fi_player, ...} }, ...],
  objects(orderedarray<aumapobject>)=[M, sun={...}, orbitingsun={...}, ...]
}
```
- `factions`: N = 2..4. Faction ids in order are `fi_player`, `fi_player2`, `fi_player3`, `fi_player4`. Faction 0 is
  the human. Other fields on a faction (`thinkperiod`, `maxsources`, `maxorders`) are tuning values; our files write
  `thinkperiod=0, maxsources=8, maxorders=8`.
- `objects`: M objects in one array. **The index of an object in this array is its identity**: wormholes name their
  partner by this index.

## Objects (tag = object kind)
Positions are `mapposition(vector2)={x(float)=..,y(float)=..}` in world units.

### sun
`faction` (-1 neutral, else faction index), `health`, `growth`, `orbitingpower`, `power` (starting level),
`maxpower` (1, 2, 3 or 5), `startingunitcount` (initial ships for owned suns), `awarenessradius` (-1 = default),
`targetedbyfaction` (array, ignored on load), `longestedge` (ignored), `mapposition`.

### orbitingsun
All sun fields plus an ellipse: `orbitx`, `orbity` (semi-axes in world units), `orbitrotation` (tilt in degrees),
`initialangle` (start angle in degrees), `orbitspeed` (larger = faster; our levels use 9..24), and `curorbitangle`,
`lastorbitdelta` (runtime state, write zero). The ellipse is centred on `mapposition`.

### fragilesun
All orbitingsun fields (with `orbitx = orbity = 0` for a fixed one) plus `brokenseconds` (time gone after
destruction), `rebuildframes` (rebuild duration in steps), `fstate` (`fs_intact`), `fframes`, `destroyedpower`.

### wormhole
`destinationwormhole` (index into `objects` of the partner wormhole; -1 = random other), `size` (radius),
`orbitx/orbity/orbitrotation/orbitspeed/initialorbitangle` (optional circular drift; zero = fixed),
`targetedbyfaction`, `longestedge`, `mapposition`.

### supernova
`requiredinvestment` (ships needed to trigger), `startscale/endscale` (drawn size), `blastradius`, `collapseradius`,
`killenemysuns`, `killenemyunits` (bool), `unitresiduals` (ships left behind), `dormantperiod` (steps between uses).

### whitehole
`requiredinvestment`, `startreturn/endreturn` (burst size range), `growthrate`, `gamesize` (drawn radius),
`dormantperiod`.

### gammaray
`killsuns`, `killunits` (bool), `rotation` (start direction, degrees), `width`, `period` (steps per sweep),
`randomization`, `randomstarttime` (bool).

### shockwave
`type` (`st_incircle` etc.), `dormant` (initial delay), `period`, `speed`, `lifetime`, `linerotation`, `startoffset`.

### fogofwar
`sunradius`, `additionalperring`, `unownedradius`, `enemyradius`, `nonsunradius` (visibility radii), `mapposition`.

### Terrain: forceterrain, speedterrain, reflectterrain, minefieldterrain, confusionterrain, neutralterrain
Common fields: `data` (array, one 0/1 per tile, row-major, `rows*cols` entries), `squares` (array of rectangles
`{r,c,width,height}` that cover the active tiles; an optimisation, may be empty), `tilesize` (world units per tile),
`rows`, `cols`, `outerradius`/`innerradius` (non-zero = a circular zone instead of the tile grid), `switchtime`,
`enabled`, `mapposition` (the grid's bottom-left corner).
Kind-specific: force has `acceleration(vector2)`, `dirswitch`, `changetype`; speed has `speedchange`; minefield has
`deathrate` and `density`; confusion has `factionchangerate` and `loseorderrate`.

## Conventions our levels follow
- Contested layouts are symmetric: one sector rotated N-fold, so each faction has an identical neighbourhood.
- Sun spacing is at least ~1200 units; home suns are level 1 with 100 ships.
