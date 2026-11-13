"""Generate the level set shipped with the open-source build.

    python -I tools/gen_levels.py [out_dir] [--rust=<levels.rs>]        (default out_dir: <repo>/levels)

Every layout here is designed from scratch by this script: nothing is read from, or derived from, any other game's
level data. The output is the plain-text map format the loader (`au2-data`) understands, plus `levels.txt`
(`<constellation>|<level name>|<file stem>|<difficulty>|<bonus 0/1>` per line, in play order).

Fairness: contested layouts are built as one sector that is rotated N-fold around the origin, so every faction gets an
identical neighbourhood. The script is deterministic (a fixed seed per level), so the output is reproducible.
"""
import math
import random
import sys
from pathlib import Path


# ---------------------------------------------------------------- object emitters
def _vec(x, y):
    return f"mapposition(vector2)={{\n\t\t\tx(float)={x:.4f},\n\t\t\ty(float)={y:.4f}\n\t\t}}"


def _sunfields(x, y, maxp, faction, power, units):
    return (
        f"\t\tfaction(faction)={faction},\n\t\thealth(s4)=100,\n\t\tgrowth(s4)=0,\n\t\torbitingpower(s4)=0,\n"
        f"\t\tpower(s4)={power},\n\t\tmaxpower(s4)={maxp},\n\t\tstartingunitcount(u4)={units},\n"
        f"\t\tawarenessradius(float)=-1.0000,\n\t\ttargetedbyfaction(orderedarray<s4>)=[0],\n"
        f"\t\t{_vec(x, y)}\n\t}}"
    )


def sun(x, y, maxp, faction=-1, units=100):
    power = 1 if faction >= 0 else 0
    if faction < 0:
        units = 0
    return "sun={\n" + _sunfields(x, y, maxp, faction, power, units).replace("\t\tmapposition", "\t\tlongestedge(s4)=0,\n\t\tmapposition", 1)


def _orbit(ox, oy, rot, ang, speed):
    return (
        f"\t\torbitx(s4)={ox},\n\t\torbity(s4)={oy},\n\t\torbitrotation(s4)={rot},\n\t\tinitialangle(s4)={ang},\n"
        f"\t\torbitspeed(u4)={speed},\n\t\tcurorbitangle(u4)=0,\n\t\tlastorbitdelta(fixedvector2)={{\n\t\t\tx(s4)=0,\n\t\t\ty(s4)=0\n\t\t}},\n"
    )


def orbiter(cx, cy, maxp, ox, oy, rot, ang, speed, faction=-1, units=100):
    power = 1 if faction >= 0 else 0
    if faction < 0:
        units = 0
    return "orbitingsun={\n" + _orbit(ox, oy, rot, ang, speed) + _sunfields(cx, cy, maxp, faction, power, units)


def fragile(cx, cy, maxp, ox=0, oy=0, rot=0, ang=0, speed=16, broken=240):
    return (
        f"fragilesun={{\n\t\tbrokenseconds(u4)={broken},\n\t\trebuildframes(u4)=256,\n\t\tfstate(fragilestate)=fs_intact,\n"
        f"\t\tfframes(u4)=0,\n\t\tdestroyedpower(u4)=0,\n" + _orbit(ox, oy, rot, ang, speed) + _sunfields(cx, cy, maxp, -1, 0, 0)
    )


def wormhole(x, y, dest, size=400, ox=0, oy=0, speed=0, ang=0):
    return (
        f"wormhole={{\n\t\tdestinationwormhole(s4)={dest},\n\t\tsize(s4)={size},\n\t\torbitx(s4)={ox},\n\t\torbity(s4)={oy},\n"
        f"\t\torbitrotation(s4)=0,\n\t\torbitspeed(s4)={speed},\n\t\tinitialorbitangle(s4)={ang},\n"
        f"\t\ttargetedbyfaction(orderedarray<s4>)=[0],\n\t\tlongestedge(s4)=0,\n\t\t{_vec(x, y)}\n\t}}"
    )


def supernova(x, y, invest=400, blast=7000, collapse=9000, kill_suns=False, kill_units=True, dormant=180):
    return (
        f"supernova={{\n\t\trequiredinvestment(s4)={invest},\n\t\tstartscale(s4)=400,\n\t\tendscale(s4)=600,\n"
        f"\t\tblastradius(s4)={blast},\n\t\tcollapseradius(s4)={collapse},\n\t\tkillenemysuns(bool)={str(kill_suns).lower()},\n"
        f"\t\tkillenemyunits(bool)={str(kill_units).lower()},\n\t\tunitresiduals(u4)=200,\n\t\tdormantperiod(s4)={dormant},\n"
        f"\t\ttargetedbyfaction(orderedarray<s4>)=[0],\n\t\tlongestedge(s4)=0,\n\t\t{_vec(x, y)}\n\t}}"
    )


def gamma(x, y, rot, width=800, period=60, kill_suns=True, kill_units=False, rand=0, rstart=False):
    return (
        f"gammaray={{\n\t\tkillsuns(bool)={str(kill_suns).lower()},\n\t\tkillunits(bool)={str(kill_units).lower()},\n"
        f"\t\trotation(s4)={rot},\n\t\twidth(s4)={width},\n\t\tperiod(s4)={period},\n\t\trandomization(s4)={rand},\n"
        f"\t\trandomstarttime(bool)={str(rstart).lower()},\n\t\t{_vec(x, y)}\n\t}}"
    )


def fog(x=0, y=0, sun_r=2500, per_ring=1250, non_sun=800):
    return (
        f"fogofwar={{\n\t\tsunradius(s4)={sun_r},\n\t\tadditionalperring(s4)={per_ring},\n\t\tunownedradius(s4)=0,\n"
        f"\t\tenemyradius(s4)=0,\n\t\tnonsunradius(s4)={non_sun},\n\t\t{_vec(x, y)}\n\t}}"
    )


def _squares(rows, cols, cells):
    """Greedy row-major rectangle cover of the active tiles (what the editor stores alongside `data`)."""
    done = [[False] * cols for _ in range(rows)]
    out = []
    for r in range(rows):
        for c in range(cols):
            if not cells[r][c] or done[r][c]:
                continue
            w = 1
            while c + w < cols and cells[r][c + w] and not done[r][c + w]:
                w += 1
            h = 1
            while r + h < rows and all(cells[r + h][c + i] and not done[r + h][c + i] for i in range(w)):
                h += 1
            for rr in range(r, r + h):
                for cc in range(c, c + w):
                    done[rr][cc] = True
            out.append((r, c, w, h))
    return out


def terrain(tag, x, y, rows, cols, cells, tile=1000, extra=""):
    flat = [1 if cells[r][c] else 0 for r in range(rows) for c in range(cols)]
    sq = _squares(rows, cols, cells)
    sqtxt = ",".join(f"{{\n\t\t\tr(u4)={r},\n\t\t\tc(u4)={c},\n\t\t\twidth(u4)={w},\n\t\t\theight(u4)={h}\n\t\t}}" for r, c, w, h in sq)
    return (
        f"{tag}={{\n{extra}\t\tdata(orderedarray<u1>)=[{len(flat)},{','.join(map(str, flat))}],\n"
        f"\t\tsquares(orderedarray<terrainsquare>)=[{len(sq)}{',' if sq else ''}{sqtxt}],\n"
        f"\t\ttilesize(s4)={tile},\n\t\trows(s4)={rows},\n\t\tcols(s4)={cols},\n\t\touterradius(s4)=0,\n\t\tinnerradius(s4)=0,\n"
        f"\t\tswitchtime(s4)=0,\n\t\tenabled(bool)=true,\n\t\t{_vec(x, y)}\n\t}}"
    )


def block(tag, cx, cy, w_tiles, h_tiles, extra="", tile=1000, mask=None):
    cells = [[(mask(r, c) if mask else True) for c in range(w_tiles)] for r in range(h_tiles)]
    return terrain(tag, cx - w_tiles * tile / 2, cy - h_tiles * tile / 2, h_tiles, w_tiles, cells, tile, extra)


def force(cx, cy, w, h, ax, ay, **kw):
    ex = f"\t\tacceleration(vector2)={{\n\t\t\tx(float)={ax:.4f},\n\t\t\ty(float)={ay:.4f}\n\t\t}},\n\t\tdirswitch(s4)=0,\n\t\tchangetype(s4)=0,\n"
    return block("forceterrain", cx, cy, w, h, ex, **kw)


def speed(cx, cy, w, h, change=32, **kw):
    return block("speedterrain", cx, cy, w, h, f"\t\tspeedchange(s4)={change},\n", **kw)


def mines(cx, cy, w, h, rate=16, density=100, tile=500, mask=None):
    return block("minefieldterrain", cx, cy, w, h, f"\t\tdeathrate(s4)={rate},\n\t\tdensity(u1)={density},\n", tile=tile, mask=mask)


def reflect(cx, cy, w, h, mask=None, tile=500):
    return block("reflectterrain", cx, cy, w, h, "", tile=tile, mask=mask)


# ---------------------------------------------------------------- level assembly
def level_text(nfactions, objects):
    fac = ",".join(
        f"{{\n\t\tfactionobject(faction)=faction={{id(factionid)={'fi_player' + ('' if i == 0 else str(i + 1))},thinkperiod(u4)=0,maxsources(u4)=8,maxorders(u4)=8}}\n\t}}"
        for i in range(nfactions)
    )
    return (
        "{\n\tfactions(orderedarray<aumapfaction>)=" + f"[{nfactions},{fac}],\n"
        f"\tobjects(orderedarray<aumapobject>)=[{len(objects)},{','.join(objects)}]\n}}"
    )


def rot(p, deg):
    a = math.radians(deg)
    return (p[0] * math.cos(a) - p[1] * math.sin(a), p[0] * math.sin(a) + p[1] * math.cos(a))


def pol(r, deg):
    a = math.radians(deg)
    return (r * math.cos(a), r * math.sin(a))


def ring(n, r, off=0):
    return [pol(r, off + 360.0 * i / n) for i in range(n)]


def spread(pts, n, off=0):
    """Rotate a sector's points n-fold. Yields (faction_index, point) so each faction sees the same sector."""
    for k in range(n):
        for p in pts:
            yield k, rot(p, off + 360.0 * k / n)


def homes(n, r, maxp=3, off=90, units=100):
    return [sun(*pol(r, off + 360.0 * k / n), maxp, faction=k, units=units) for k in range(n)]


def neutrals_sym(n, sector, off=90):
    """sector: list of (x, y, maxpower) given for faction 0's wedge; replicated for all n."""
    out = []
    for k in range(n):
        for (x, y, m) in sector:
            out.append(sun(*rot((x, y), off - 90 + 360.0 * k / n), m))
    return out


def lvl(n, objs):
    return level_text(n, objs)


LEVELS = []  # (constellation, name, stem, difficulty, bonus, text)


def add(cons, name, difficulty, nf, objs, bonus=False):
    stem = (cons + "_" + name).lower().replace(" ", "_")
    LEVELS.append((cons, name, stem, difficulty, bonus, lvl(nf, objs)))


# ---------------------------------------------------------------- level designs
def design():
    # ---------------- DAWN: the basics
    add("Dawn", "Twin", 0, 2, homes(2, 3500, 2, 180) + neutrals_sym(2, [(-1800, 0, 1), (0, 2600, 1)], 90)
        + [sun(0, 0, 3)])
    add("Dawn", "Trident", 0, 3, homes(3, 4200, 2) + neutrals_sym(3, [(-1700, 1500, 1), (1700, 1500, 1)]) + [sun(0, 0, 3)])
    add("Dawn", "Quartet", 1, 4, homes(4, 4400, 2, 45) + neutrals_sym(4, [(0, 2300, 2), (1900, 1100, 1)], 45) + [sun(0, 0, 5)])
    add("Dawn", "Staircase", 0, 2,
        [sun(-4800, -2400, 2, 0), sun(4800, 2400, 2, 1)]
        + [sun(-3200 + 1600 * i, -1600 + 800 * i, 1) for i in range(6)]
        + [sun(-3200 + 1600 * i, 1600 - 800 * i, 1) for i in range(6) if i not in (2, 3)] + [sun(0, 0, 2)])
    add("Dawn", "Crown", 1, 3,
        homes(3, 5200, 3) + [sun(*p, 1) for p in ring(9, 2800, 20)] + [sun(*p, 2) for p in ring(3, 1400, 90)])
    add("Dawn", "Bastion", 2, 2,
        homes(2, 4800, 3, 180) + [sun(*p, 1) for p in ring(12, 3000, 15)] + [sun(0, 0, 5)]
        + [sun(*p, 2) for p in ring(4, 1500, 0)])
    add("Dawn", "Pinwheel", 2, 4,
        homes(4, 5200, 2, 45) + [sun(*pol(2200, 45 + 90 * k + 30), 2) for k in range(4)]
        + [sun(*pol(3600, 45 + 90 * k - 25), 1) for k in range(4)] + [sun(0, 0, 5)])

    # ---------------- TIDEWORK: orbiting suns
    add("Tidework", "Carousel", 0, 2,
        homes(2, 4600, 2, 180) + [orbiter(0, 0, 3, 2200, 2200, 0, 90 * k, 12) for k in range(4)]
        + [sun(0, 0, 5)])
    add("Tidework", "Pendulum", 1, 2,
        homes(2, 4800, 2, 0)
        + [orbiter(-1500 + 3000 * (k % 2), 0, 2, 0, 2600, 0, 180 * (k % 2), 14) for k in range(2)]
        + [orbiter(0, 0, 1, 3600, 800, 90, 60 * k, 10) for k in range(6)])
    add("Tidework", "Orrery", 2, 3,
        homes(3, 5600, 3)
        + [orbiter(0, 0, 2, 1800, 1800, 0, 120 * k, 18) for k in range(3)]
        + [orbiter(0, 0, 1, 3400, 3400, 0, 60 + 72 * k, 9) for k in range(5)] + [sun(0, 0, 5)])
    add("Tidework", "Eclipse", 3, 4,
        homes(4, 5200, 2, 45)
        + [orbiter(*pol(2800, 45 + 90 * k), 2, 1300, 1300, 0, 45 * k, 20) for k in range(4)]
        + [orbiter(0, 0, 3, 1500, 1500, 0, 90 * k, 24) for k in range(2)] + [sun(0, 0, 5)])
    add("Tidework", "Spindle", 2, 2,
        homes(2, 5000, 3, 180)
        + [orbiter(*pol(2200, 90 + 180 * s), 2, 1000, 1000, 0, 90 * k, 16 + 4 * k) for s in range(2) for k in range(2)]
        + [sun(0, 3600, 1), sun(0, -3600, 1)])

    # ---------------- BLASTWORKS: supernovas and gamma rays
    add("Blastworks", "Fuse", 0, 2,
        homes(2, 4500, 2, 180) + neutrals_sym(2, [(-2000, 1200, 1), (-2000, -1200, 1)], 90)
        + [supernova(0, 0, 400, 6500, 8500), sun(0, 2800, 2), sun(0, -2800, 2)])
    add("Blastworks", "Beacon", 1, 3,
        homes(3, 5000, 2) + neutrals_sym(3, [(-1500, 1800, 1), (1500, 1800, 1)]) + [supernova(0, 0, 500, 7000, 9000)])
    add("Blastworks", "Scythe", 1, 2,
        homes(2, 4800, 2, 180) + [sun(*p, 1) for p in ring(10, 3000, 10)] + [sun(0, 0, 3)]
        + [gamma(0, 0, 90, 700, 90, rstart=False)])
    add("Blastworks", "Sweep", 2, 3,
        homes(3, 5200, 3) + [sun(*p, 1) for p in ring(9, 3100, 0)] + [sun(*p, 2) for p in ring(3, 1500, 90)]
        + [gamma(0, 0, 30, 800, 70, rand=20, rstart=True), supernova(0, 0, 600, 5000, 7000)])
    add("Blastworks", "Meltdown", 3, 4,
        homes(4, 5400, 2, 45) + neutrals_sym(4, [(0, 2600, 2), (1500, 1500, 1)], 45)
        + [supernova(0, 0, 800, 8000, 10000, kill_suns=True), gamma(0, 0, 0, 600, 80, rand=30, rstart=True)])

    # ---------------- CURRENTS: force and speed terrain
    add("Currents", "Rapids", 0, 2,
        homes(2, 5000, 2, 180) + neutrals_sym(2, [(-1700, 1400, 1), (-1700, -1400, 1)], 90) + [sun(0, 0, 3)]
        + [force(0, 0, 10, 2, -30, 0), force(0, 4000, 8, 1, 30, 0)])
    add("Currents", "Slipstream", 1, 2,
        homes(2, 5200, 2, 180) + [sun(*p, 1) for p in ring(8, 2900, 22)] + [sun(0, 0, 3)]
        + [speed(0, 0, 14, 4, 32, mask=lambda r, c: abs(r - 1.5) < 1.2 or c in (0, 13))])
    add("Currents", "Eddy", 2, 3,
        homes(3, 5400, 3) + [sun(*p, 1) for p in ring(6, 3000, 30)] + [sun(*p, 2) for p in ring(3, 1500, 90)]
        + [force(*pol(2200, 90 + 120 * k), 4, 4, *rot((26, 0), 60 + 120 * k)) for k in range(3)])
    add("Currents", "Conveyor", 2, 2,
        [sun(-5200, 0, 3, 0), sun(5200, 0, 3, 1)] + [sun(-2600 + 1300 * i, 0, 1) for i in range(5) if i != 2] + [sun(0, 0, 3)]
        + [sun(-1800, 2600, 2), sun(1800, 2600, 2), sun(-1800, -2600, 2), sun(1800, -2600, 2)]
        + [force(0, 1300, 12, 1, 28, 0), force(0, -1300, 12, 1, -28, 0)])

    # ---------------- THE DEEP: minefields and fog
    add("Deep", "Shoals", 0, 2,
        homes(2, 4800, 2, 180) + neutrals_sym(2, [(-1800, 1600, 1), (-1800, -1600, 1)], 90) + [sun(0, 0, 3)]
        + [mines(0, 0, 8, 8, 12, 80)])
    add("Deep", "Reef", 1, 3,
        homes(3, 5200, 3) + [sun(*p, 1) for p in ring(6, 3200, 0)] + [sun(*p, 2) for p in ring(3, 1700, 90)]
        + [mines(*pol(1900, 30 + 120 * k), 5, 5, 14, 90) for k in range(3)])
    add("Deep", "Murk", 1, 2,
        homes(2, 5000, 2, 180) + [sun(*p, 1) for p in ring(10, 3000, 8)] + [sun(0, 0, 5)]
        + [fog(0, 0, 2400, 1200, 700)])
    add("Deep", "Abyss", 3, 3,
        homes(3, 5600, 3) + [sun(*p, 1) for p in ring(6, 3300, 30)] + [sun(*p, 2) for p in ring(3, 1700, 90)] + [sun(0, 0, 5)]
        + [fog(0, 0, 2000, 1000, 600), mines(0, 0, 8, 8, 20, 100, mask=lambda r, c: 1 <= r <= 6 and 1 <= c <= 6)])

    # ---------------- GATEWAYS: wormholes
    # (a wormhole's destination is an index into the object list, so wormholes are appended last in known order)
    def with_wormholes(base, pairs, size=450):
        objs = list(base)
        start = len(objs)
        for i, ((ax, ay), (bx, by)) in enumerate(pairs):
            objs.append(wormhole(ax, ay, start + 2 * i + 1, size))
            objs.append(wormhole(bx, by, start + 2 * i, size))
        return objs

    add("Gateways", "Doorway", 0, 2,
        with_wormholes(homes(2, 4800, 2, 180) + neutrals_sym(2, [(-1600, 1400, 1), (-1600, -1400, 1)], 90) + [sun(0, 0, 3)],
                       [((-2200, 3400), (2200, -3400))]))
    add("Gateways", "Switchyard", 1, 3,
        with_wormholes(homes(3, 5200, 2) + neutrals_sym(3, [(-1700, 1800, 1), (1700, 1800, 1)]) + [sun(0, 0, 3)],
                       [(pol(2800, 90 + 120 * k), pol(2800, 150 + 120 * k)) for k in range(3)]))
    add("Gateways", "Loop", 2, 2,
        with_wormholes(homes(2, 5000, 3, 180) + [sun(*p, 1) for p in ring(8, 3100, 0)] + [sun(0, 0, 5)],
                       [((0, 4300), (0, -4300)), ((-4300, 0), (4300, 0))]))
    add("Gateways", "Tangle", 3, 4,
        with_wormholes(homes(4, 5200, 2, 45) + neutrals_sym(4, [(0, 2600, 2), (1700, 1400, 1)], 45) + [sun(0, 0, 5)],
                       [(pol(3600, 45 + 90 * k), pol(3600, 135 + 90 * k)) for k in range(0, 2)]), )

    # ---------------- GLASSHOUSE: fragile suns
    add("Glasshouse", "Brittle", 1, 2,
        homes(2, 4800, 2, 180)
        + [fragile(*p, 2) for p in ring(6, 2600, 0)] + [sun(0, 0, 3)])
    add("Glasshouse", "Cascade", 2, 3,
        homes(3, 5400, 3)
        + [fragile(*p, 1, speed=0) for p in ring(9, 3200, 10)] + [fragile(*p, 2, speed=0) for p in ring(3, 1600, 90)]
        + [sun(0, 0, 5)])
    add("Glasshouse", "Tremor", 3, 2,
        homes(2, 5200, 3, 180)
        + [fragile(*pol(2800, 90 + 180 * s), 2, 1200, 1200, 0, 120 * k, 18, broken=120) for s in range(2) for k in range(3)]
        + [gamma(0, 0, 90, 700, 100, kill_suns=True)])


SUBTITLES = {
    "Dawn": "FIRST CONTACT", "Tidework": "ORBITS", "Blastworks": "SUPERNOVAS AND RAYS", "Currents": "FORCE AND SPEED",
    "Deep": "MINES AND FOG", "Gateways": "WORMHOLES", "Glasshouse": "FRAGILE SUNS",
}


def rust_table():
    """`crates/aurora-native/src/levels.rs` for the open-source build: same shape as the private build's table."""
    out = [
        "//! Constellations and levels shipped with the game. Generated by `tools/gen_levels.py`; do not edit.",
        "",
        "pub struct Level {",
        "    pub name: &'static str,",
        "    /// File stem of the map (`<map>.au2` in `levels/`).",
        "    pub map: &'static str,",
        "    /// Constellation difficulty: 0 easy .. 3 insane.",
        "    pub difficulty: u8,",
        "    pub bonus: bool,",
        "}",
        "",
        "pub struct Constellation {",
        "    pub name: &'static str,",
        "    pub sub: &'static str,",
        "    pub levels: &'static [Level],",
        "}",
        "",
        "pub const CONSTELLATIONS: &[Constellation] = &[",
    ]
    order = []
    for cons, *_ in LEVELS:
        if cons not in order:
            order.append(cons)
    for cons in order:
        out.append(f'    Constellation {{ name: "{cons.upper()}", sub: "{SUBTITLES[cons]}", levels: &[')
        for c, name, stem, diff, bonus, _ in LEVELS:
            if c == cons:
                out.append(f'        Level {{ name: "{name.upper()}", map: "{stem}", difficulty: {diff}, bonus: {str(bonus).lower()} }},')
        out.append("    ] },")
    out.append("];")
    return "\n".join(out) + "\n"


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--rust=")]
    rust_out = next((a[7:] for a in sys.argv[1:] if a.startswith("--rust=")), None)
    out = Path(args[0]) if args else Path(__file__).resolve().parent.parent / "levels"
    out.mkdir(parents=True, exist_ok=True)
    for f in out.glob("*.au2"):
        f.unlink()
    design()
    index = []
    for cons, name, stem, diff, bonus, text in LEVELS:
        (out / f"{stem}.au2").write_text(text, newline="\n")
        index.append(f"{cons}|{name}|{stem}|{diff}|{int(bonus)}")
    (out / "levels.txt").write_text("\n".join(index) + "\n", newline="\n")
    if rust_out:
        Path(rust_out).write_text(rust_table(), newline="\n")
    print(f"wrote {len(LEVELS)} levels to {out}")


if __name__ == "__main__":
    main()
