"""Data-driven composer for aurora-song/1. A song is a dict of musical choices (key, mode, chord loop,
palette, groove, arp shape, loop length); `build(spec)` turns it into the JSON structure documented in
the song JSON format (see `format.rs`). Everything is deterministic: the same spec always yields the same file.

What every song gets, because the game needs it:
  * a kick on every beat of both main loops, hardest on the bar line (units spawn on bars / half bars /
    beats, and the spawn points pulse there), with hats on the off-beat 8ths;
  * bass, chords and arps side-chained to that kick, so the whole mix pumps with it;
  * pads that pump on every beat too, with no drums of their own, so they sit on either main loop;
  * fills made only of the tonic pentatonic, because a fill can start on any bar of the chord loop.
"""
import json
import re

NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
SCALES = {
    "minor": [0, 2, 3, 5, 7, 8, 10],
    "dorian": [0, 2, 3, 5, 7, 9, 10],
    "phrygian": [0, 1, 3, 5, 7, 8, 10],
    "major": [0, 2, 4, 5, 7, 9, 11],
    "lydian": [0, 2, 4, 6, 7, 9, 11],
    "mixolydian": [0, 2, 4, 5, 7, 9, 10],
}
# The five notes of the mode that sit well over every chord of the loop.
PENTA = {"minor": [0, 3, 5, 7, 10], "dorian": [0, 3, 5, 7, 10], "phrygian": [0, 3, 5, 7, 10],
         "major": [0, 2, 4, 7, 9], "lydian": [0, 2, 4, 7, 9], "mixolydian": [0, 2, 4, 7, 9]}


def name(m):
    return f"{NOTES[m % 12]}{m // 12 - 1}"


# --- instruments ----------------------------------------------------------------------------------
def o(wave, level=1.0, octave=0, detune=0):
    return {"wave": wave, "level": level, "octave": octave, "detune": detune}


def synth(osc, a, d, s, r, lp=None, uni=None, fm=None, vib=None, drive=0.0):
    """lp = (cutoff, env, q, decay, track); uni = (voices, spread cents, width); fm = (ratio, index, decay);
    vib = (rate Hz, depth cents, delay s)."""
    i = {"type": "synth", "osc": osc, "attack": a, "decay": d, "sustain": s, "release": r}
    if lp:
        lp = tuple(lp) + (0, 0.707, 0, 0)[len(lp) - 1:] if len(lp) < 5 else lp
        i["lowpass"] = {"cutoff": lp[0], "env": lp[1], "q": lp[2], "decay": lp[3], "track": lp[4]}
    if uni:
        i["unison"], i["spread"], i["width"] = uni
    if fm:
        i["fm"] = {"ratio": fm[0], "index": fm[1], "decay": fm[2]}
    if vib:
        i["vibrato"] = {"rate": vib[0], "depth": vib[1], "delay": vib[2]}
    if drive:
        i["drive"] = drive
    return i


def kit(kick=(150, 48, 0.22, 34, 0.5, 1.0), snare=(190, 0.20, 0.55), clap=(0.20, 1300), hat=(0.045, 8000),
        ohat=(0.22, 7000), tom=(0.32, 1.7)):
    return dict(
        kick=dict(type="kick", start=kick[0], end=kick[1], decay=kick[2], sweep=kick[3], click=kick[4], drive=kick[5]),
        snare=dict(type="snare", tone=snare[0], decay=snare[1], noise=snare[2]),
        rim=dict(type="snare", tone=430, decay=0.06, noise=0.25),
        clap=dict(type="clap", decay=clap[0], tone=clap[1]),
        hat=dict(type="hat", decay=hat[0], tone=hat[1]),
        ohat=dict(type="hat", decay=ohat[0], tone=ohat[1]),
        shaker=dict(type="hat", decay=0.05, tone=5500, attack=0.012),
        crash=dict(type="hat", decay=1.5, tone=4500),
        tom=dict(type="tom", decay=tom[0], sweep=tom[1]),
        rise=dict(type="sweep", **{"from": 350, "to": 9000, "q": 1.2}),
        fall=dict(type="sweep", **{"from": 9000, "to": 250, "q": 1.0}),
    )


SAW2 = [o("saw", .5), o("saw", .35, 1)]
PALETTES = {
    # Analog: detuned saws, a round FM bell, a rubbery saw bass.
    "warm": dict(
        pad=synth(SAW2, .5, 2.0, .8, 1.3, (900, 900, .8), (3, 14, .9)),
        bright=synth([o("saw", .55), o("square", .2, 1)], .2, 1.5, .75, 1.0, (2200, 2000, .9), (5, 18, 1.0)),
        dark=synth([o("tri", .7), o("saw", .25)], 1.2, 2.0, .9, 1.8, (420, 260, .7), (3, 10, .8)),
        bell=synth([o("sine", .8)], .002, 1.1, 0, .9, None, None, (3.5, 2.4, .22)),
        pluck=synth([o("saw", .5), o("square", .3)], .002, .22, 0, .16, (520, 4200, 1.6, .11, .4), (2, 8, .5)),
        lead=synth([o("square", .35), o("saw", .4)], .015, .4, .65, .28, (1700, 1800, 1.0, 0, .5), (3, 9, .6), None, (5.4, 11, .22)),
        bass=synth([o("saw", .6), o("sine", .7)], .004, .25, .55, .08, (190, 1200, 1.2, .16, .3), None, None, None, 1.5),
        sub=synth([o("sine", 1.0)], .02, .3, 1.0, .18, None, None, None, None, 0.8),
        stab=synth([o("saw", .6)], .003, .2, 0, .14, (800, 3600, 1.3, .13, .4), (3, 12, .9)),
        **kit()),
    # Glass: triangles and sines, long FM chimes, a soft sine bass.
    "glass": dict(
        pad=synth([o("tri", .7), o("sine", .3, 1)], .8, 2.5, .85, 1.8, (1500, 900, .7), (3, 9, .9)),
        bright=synth([o("tri", .6), o("saw", .25, 1)], .3, 1.8, .75, 1.3, (3000, 1500, .8), (5, 12, 1.0)),
        dark=synth([o("sine", .8), o("tri", .3, -1)], 1.6, 2.5, .9, 2.2, (380, 160, .7), (3, 7, .7)),
        bell=synth([o("sine", .7), o("sine", .2, 2)], .002, 1.6, 0, 1.3, None, None, (5.0, 1.8, .35)),
        pluck=synth([o("tri", .7), o("sine", .3, 1)], .002, .32, 0, .25, (1400, 3200, 1.2, .16, .5), None, (2.0, 1.4, .08)),
        lead=synth([o("tri", .6), o("sine", .3, 1)], .02, .5, .7, .35, (2400, 1500, .9, 0, .5), (3, 7, .6), (1.0, .9, .3), (5.0, 9, .25)),
        bass=synth([o("sine", .8), o("tri", .5, 1)], .006, .3, .65, .1, (260, 800, 1.0, .2, .3), None, (1.0, 1.2, .12), None, 1.2),
        sub=synth([o("sine", 1.0)], .02, .3, 1.0, .18, None, None, None, None, 0.6),
        stab=synth([o("tri", .7), o("sine", .3, 1)], .003, .25, 0, .2, (1200, 3000, 1.1, .15, .4), (3, 9, .9), (2.0, 1.2, .1)),
        **kit(kick=(130, 46, 0.26, 30, 0.4, 0.8), snare=(210, 0.16, 0.5), clap=(0.16, 1500), hat=(0.035, 9000), ohat=(0.18, 8000))),
    # Dusk: dark saws through slow filters, hollow square lead, deep kick.
    "dusk": dict(
        pad=synth([o("saw", .5), o("tri", .4, -1)], 1.0, 2.5, .75, 1.8, (600, 650, .9), (3, 18, .9)),
        bright=synth([o("saw", .5), o("saw", .25, 1)], .4, 2.0, .65, 1.2, (1300, 1400, 1.0), (5, 20, 1.0)),
        dark=synth([o("saw", .35), o("sine", .7, -1)], 1.8, 2.5, .9, 2.4, (300, 140, .8), (3, 12, .7)),
        bell=synth([o("tri", .6), o("sine", .3)], .004, 1.3, 0, 1.0, None, None, (2.0, 2.0, .3)),
        pluck=synth([o("saw", .5), o("square", .25, -1)], .003, .26, 0, .14, (420, 3000, 1.8, .13, .4), (2, 10, .5)),
        lead=synth([o("square", .45), o("tri", .3)], .02, .45, .6, .3, (1300, 1400, 1.2, 0, .5), (3, 10, .5), None, (5.0, 13, .3)),
        bass=synth([o("saw", .55), o("sine", .75)], .005, .3, .6, .1, (170, 900, 1.3, .2, .3), None, None, None, 1.8),
        sub=synth([o("sine", 1.0)], .02, .3, 1.0, .2, None, None, None, None, 1.0),
        stab=synth([o("saw", .6), o("square", .2)], .004, .24, 0, .16, (600, 2600, 1.5, .15, .4), (3, 14, .9)),
        **kit(kick=(120, 43, 0.28, 28, 0.45, 1.3), snare=(170, 0.24, 0.5), clap=(0.24, 1100), hat=(0.05, 7500), ohat=(0.26, 6500), tom=(0.4, 1.6))),
    # Neon: bright supersaws, a snappy resonant bass, a tight clicky kick.
    "neon": dict(
        pad=synth(SAW2, .3, 1.5, .8, 1.0, (1500, 1700, .9), (5, 16, 1.0)),
        bright=synth([o("saw", .6), o("saw", .3, 1)], .1, 1.2, .7, .8, (3000, 2600, .9), (7, 20, 1.0)),
        dark=synth([o("saw", .4), o("sine", .6, -1)], 1.2, 2.0, .85, 1.8, (480, 320, .8), (3, 12, .8)),
        bell=synth([o("sine", .6), o("square", .12)], .002, .8, 0, .5, (4500, 1500, .8), None, (4.0, 2.2, .15)),
        pluck=synth([o("saw", .6)], .002, .16, 0, .1, (800, 5000, 2.0, .09, .5), (3, 10, .7)),
        lead=synth([o("saw", .6)], .01, .3, .7, .22, (2400, 2400, 1.1, 0, .5), (5, 14, .8), None, (5.8, 10, .2)),
        bass=synth([o("saw", .7), o("square", .3, -1)], .003, .2, .5, .07, (220, 1800, 2.0, .12, .3), None, None, None, 2.0),
        sub=synth([o("sine", 1.0)], .015, .3, 1.0, .15, None, None, None, None, 1.0),
        stab=synth([o("saw", .7)], .002, .16, 0, .12, (1000, 4500, 1.6, .1, .4), (5, 16, 1.0)),
        **kit(kick=(170, 50, 0.20, 40, 0.65, 1.4), snare=(200, 0.18, 0.6), clap=(0.18, 1400), hat=(0.04, 8500), ohat=(0.2, 7500), tom=(0.26, 1.8))),
    # Bright: square and triangle chimes over soft saws, a rounder mid-forward bass.
    "bright": dict(
        pad=synth([o("saw", .45), o("tri", .4, 1)], .6, 1.8, .8, 1.3, (1300, 1100, .8), (3, 12, .9)),
        bright=synth([o("saw", .5), o("square", .25, 1)], .2, 1.4, .7, .9, (2600, 2000, .9), (5, 15, 1.0)),
        dark=synth([o("tri", .7), o("sine", .5, -1)], 1.4, 2.0, .9, 1.8, (460, 220, .7), (3, 8, .7)),
        bell=synth([o("sine", .7), o("tri", .15, 1)], .002, .9, 0, .7, None, None, (7.0, 1.3, .18)),
        pluck=synth([o("square", .45), o("tri", .4, 1)], .002, .18, 0, .12, (1100, 3800, 1.4, .1, .5), (2, 7, .5)),
        lead=synth([o("tri", .6), o("square", .25)], .012, .35, .7, .25, (2600, 1800, 1.0, 0, .5), (3, 8, .6), None, (5.6, 10, .2)),
        bass=synth([o("square", .45), o("sine", .8)], .004, .22, .55, .09, (240, 1300, 1.2, .15, .3), None, None, None, 1.4),
        sub=synth([o("sine", 1.0)], .02, .3, 1.0, .18, None, None, None, None, 0.8),
        stab=synth([o("square", .4), o("saw", .4)], .003, .2, 0, .14, (900, 3600, 1.3, .12, .4), (3, 10, .9)),
        **kit(kick=(155, 50, 0.22, 36, 0.55, 1.0), snare=(230, 0.15, 0.55), clap=(0.17, 1600), hat=(0.035, 9000), ohat=(0.2, 8000))),
}

# --- grooves ---------------------------------------------------------------------------------------
# Per bar, as (step, velocity); "lo" is the calm main loop, "hi" the intense one. Every groove keeps a
# kick on all four beats (the spawn grid at any game speed), loudest on beat 1, then 3.
FOUR_LO = [(0, 1.0), (4, .72), (8, .88), (12, .72)]
FOUR_HI = [(0, 1.0), (4, .9), (8, .95), (12, .9)]
OFF8 = [(2, .8), (6, .8), (10, .8), (14, .9)]
GROOVES = {
    # House: four on the floor, clap on 2 and 4, open hat on the off-beats.
    "four": dict(
        lo=dict(kick=FOUR_LO, clap=[(12, .55)], hat=OFF8, ohat=[], shaker=[], rim=[(4, .5)]),
        hi=dict(kick=FOUR_HI, clap=[(4, .9), (12, 1.0)], hat=[(s, .45) for s in range(0, 16, 2)] + [(s, .25) for s in range(1, 16, 2)],
                ohat=OFF8, shaker=[], rim=[]),
        bass=dict(lo=[(0, 3, 0, .9), (6, 2, 0, .7), (8, 3, 0, .85), (14, 2, 1, .7)],
                  hi=[(2, 1.7, 0, .9), (6, 1.7, 0, .85), (10, 1.7, 0, .9), (14, 1.7, 1, .85)]),
        stab=[(6, 1.5, .8), (14, 1.5, .7)]),
    # Drive: kick plus a pushing 8th-note bass and shaker 16ths.
    "drive": dict(
        lo=dict(kick=FOUR_LO, clap=[(4, .5), (12, .6)], hat=OFF8, ohat=[], shaker=[(s, .3 if s % 4 else .5) for s in range(16)], rim=[]),
        hi=dict(kick=FOUR_HI + [(14, .6)], clap=[(4, .9), (12, 1.0)], hat=[(s, .4) for s in range(0, 16, 2)], ohat=OFF8,
                shaker=[(s, .35 if s % 4 else .6) for s in range(16)], rim=[]),
        bass=dict(lo=[(0, 1.6, 0, .9), (2, 1.6, 0, .7), (6, 1.6, 0, .75), (8, 1.6, 0, .85), (10, 1.6, 0, .7), (14, 1.6, 1, .75)],
                  hi=[(s, 1.5, 1 if s in (6, 14) else 0, .9 if s % 4 == 2 else .75) for s in range(0, 16, 2)]),
        stab=[(3, 1, .7), (6, 1, .8), (11, 1, .7), (14, 1, .8)]),
    # Broken: syncopated kicks between the beats, snare on 2 and 4, busy hats.
    "broken": dict(
        lo=dict(kick=FOUR_LO + [(7, .6)], clap=[], hat=OFF8, ohat=[], shaker=[], rim=[(4, .55), (12, .6)]),
        hi=dict(kick=FOUR_HI + [(7, .7), (11, .6)], clap=[(4, .85), (12, .95)], hat=[(s, .45) for s in (0, 2, 3, 6, 8, 10, 11, 14, 15)],
                ohat=[(6, .7), (14, .8)], shaker=[], rim=[(15, .5)]),
        bass=dict(lo=[(0, 4, 0, .9), (7, 2, 0, .75), (10, 3, 0, .8)],
                  hi=[(0, 2, 0, .9), (3, 1.5, 0, .8), (7, 1.5, 0, .85), (10, 2, 0, .85), (14, 1.5, 1, .8)]),
        stab=[(3, 1.2, .75), (10, 1.2, .8)]),
    # Half: a slow, heavy feel: big beat 1 and 3, the back-beat on 3, toms answering.
    "half": dict(
        lo=dict(kick=[(0, 1.0), (4, .6), (8, .85), (12, .6)], clap=[], hat=[(s, .6) for s in (2, 6, 10, 14)], ohat=[], shaker=[], rim=[(8, .6)]),
        hi=dict(kick=[(0, 1.0), (4, .8), (8, .95), (10, .6), (12, .8)], clap=[(8, 1.0)], hat=[(s, .5) for s in range(0, 16, 2)],
                ohat=[(14, .8)], shaker=[(s, .3) for s in range(1, 16, 2)], rim=[(4, .5), (12, .5)]),
        bass=dict(lo=[(0, 7, 0, .9), (8, 5, 0, .8)],
                  hi=[(0, 3, 0, .9), (6, 2, 0, .8), (8, 3, 0, .9), (11, 1.5, 0, .7), (14, 2, 1, .8)]),
        stab=[(6, 2, .7)]),
    # Pulse: straight and hypnotic, 16th hats, a bass that answers each kick.
    "pulse": dict(
        lo=dict(kick=FOUR_LO, clap=[(12, .5)], hat=[(s, .55 if s % 4 == 2 else .3) for s in range(0, 16, 2)], ohat=[], shaker=[], rim=[]),
        hi=dict(kick=FOUR_HI, clap=[(4, .85), (12, .95)], hat=[(s, .5 if s % 4 == 2 else .28) for s in range(16)], ohat=[(14, .8)], shaker=[],
                rim=[(7, .4), (15, .45)]),
        bass=dict(lo=[(2, 2, 0, .85), (8, 2, 0, .8), (10, 2, 0, .8)],
                  hi=[(1, 1, 0, .8), (2, 1.5, 0, .9), (5, 1, 0, .75), (6, 1.5, 0, .85), (9, 1, 0, .8), (10, 1.5, 0, .9), (13, 1, 0, .75), (14, 1.5, 1, .85)]),
        stab=[(2, 1, .7), (6, 1, .7), (10, 1, .7), (14, 1, .75)]),
}

# Arpeggio shapes: (chord voice 0..3, octave) per note.
ARPS = {
    "up": [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (3, 0), (2, 0), (1, 0)],
    "down": [(0, 1), (3, 0), (2, 0), (1, 0), (0, 0), (1, 0), (2, 0), (3, 0)],
    "skip": [(0, 0), (2, 0), (1, 0), (3, 0), (2, 0), (0, 1), (3, 0), (1, 0)],
    "wide": [(0, 0), (2, 0), (0, 1), (1, 0), (3, 0), (1, 1), (2, 0), (0, 1)],
}
TRESILLO = [0, 3, 6, 8, 11, 14]  # 3-3-2: the accents of every arp

# Two-bar lead rhythms, (step, length).
RHYTHMS = [
    [(0, 3), (3, 3), (6, 2), (8, 6), (18, 2), (20, 2), (22, 2), (24, 7)],
    [(0, 6), (6, 2), (8, 4), (12, 4), (16, 6), (22, 2), (24, 7)],
    [(2, 2), (4, 2), (6, 4), (10, 2), (12, 4), (18, 2), (20, 2), (22, 6), (28, 4)],
    [(0, 2), (2, 2), (4, 4), (8, 3), (11, 3), (14, 2), (16, 8), (24, 4), (28, 4)],
    [(0, 4), (4, 2), (6, 6), (12, 4), (16, 3), (19, 3), (22, 2), (24, 8)],
    [(0, 8), (8, 4), (12, 4), (16, 12)],
]


class Lcg:
    def __init__(self, seed):
        self.x = (seed * 2654435761 + 12345) & 0x7FFFFFFF

    def next(self):
        self.x = (self.x * 1103515245 + 12345) & 0x7FFFFFFF
        return self.x >> 8

    def pick(self, seq):
        return seq[self.next() % len(seq)]


def build(spec):
    mode = spec["mode"]
    scale = SCALES[mode]
    root = spec["root"]  # MIDI note of the tonic, 48..59
    prog = spec["progression"]  # scale degrees (0-based), one chord per `chord_bars`
    cb = spec["chord_bars"]
    B = spec["bars"]
    I = spec.get("intro_bars", 4)
    groove = GROOVES[spec["groove"]]
    arp_shape = ARPS[spec.get("arp", "up")]
    seed = spec["seed"]
    S16 = 16

    def pitch(deg):
        return root + scale[deg % 7] + 12 * (deg // 7)

    def chord_deg(bar):
        return prog[(bar // cb) % len(prog)]

    def place(m, lo):
        """The pitch class of m in the octave starting at lo."""
        return lo + (m - lo) % 12

    def voicing(bar, lo=55):
        """Root, third, fifth, seventh of the bar's chord, all inside one octave from `lo`, so successive
        chords move by the smallest steps (voice leading)."""
        d = chord_deg(bar)
        r = pitch(d)
        # The colour tone: a minor seventh, else a major ninth, else the root again. Never a major seventh
        # or a flat ninth, which would sit a semitone from the root inside one octave.
        colour = next((pitch(d + k) for k, iv in ((6, 10), (8, 2)) if (pitch(d + k) - r) % 12 == iv), r)
        v = sorted(place(pitch(d + k), lo) for k in (0, 2, 4))
        c = place(colour, lo)
        return v + [c + 12 if c in v else c] if c in v else sorted(v + [c])

    def triad_pcs(bar):
        d = chord_deg(bar)
        return [pitch(d + k) % 12 for k in (0, 2, 4)]

    def bass_note(bar):
        return place(pitch(chord_deg(bar)), 33)  # A1..G#2: 55..104 Hz

    def track(inst, events, gain=1.0, pan=0.0, **fx):
        events = sorted(([int(s), n, round(float(l), 3), round(min(max(float(v), 0.0), 1.0), 3)] for s, n, l, v in events), key=lambda e: e[0])
        t = {"instrument": inst, "gain": round(gain, 3), "pan": pan}
        t.update({k: v for k, v in fx.items() if v})
        t["events"] = events
        return t

    def echo(steps=3, fb=0.45, mix=0.35, damp=0.3):
        return {"steps": steps, "feedback": fb, "mix": mix, "pingpong": True, "damp": damp}

    def pump(depth, release=1.5, every=0):
        return {"depth": depth, "release": release, "every": every}

    # ---- parts ------------------------------------------------------------------------------------
    def pad_chords(bars, offset=0, octave=0, vel=0.6, lo=55, sevenths=True, first=0):
        ev = []
        for b in range(first, bars, cb):
            # Re-strike on each chord change; hold just short of the next one.
            ln = min(cb, bars - b) * S16 - 0.5
            v = voicing(b + offset, lo)
            for k, m in enumerate(v if sevenths else v[:3]):
                ev.append([b * S16, name(m + 12 * octave), ln, vel * (0.7 if k == 3 else 1.0)])
        return ev

    def arp(bars, vel, dense, octave=1, offset=0, first=0):
        ev = []
        for b in range(first, bars):
            v = voicing(b + offset, 55)
            steps = range(16) if dense else TRESILLO
            for n, s in enumerate(steps):
                k, up = arp_shape[(n if dense else TRESILLO.index(s) + (b % 2) * 3) % len(arp_shape)]
                accent = 1.0 if s in TRESILLO else 0.55
                ev.append([b * S16 + s, name(v[k] + 12 * (octave + up)), 0.9 if dense else 1.6, vel * accent])
        return ev

    def bass(bars, pattern, vel=1.0, offset=0, first=0):
        ev = []
        for b in range(first, bars):
            r = bass_note(b + offset)
            for s, ln, up, v in pattern:
                ev.append([b * S16 + s, name(r + 12 * up), ln, vel * v])
        return ev

    def stabs(bars, vel):
        ev = []
        for b in range(bars):
            for s, ln, v in groove["stab"]:
                for m in voicing(b, 60)[:3]:
                    ev.append([b * S16 + s, name(m), ln, vel * v])
        return ev

    # ---- the tune ---------------------------------------------------------------------------------
    # A ladder of pentatonic pitches; a phrase is a walk on it (up to the middle, back down after) whose
    # strong notes snap to the chord. Phrases come in the form A A' B A'' so the lead is a tune.
    lead_lo, lead_hi = 64, 86
    ladder = [p for p in range(lead_lo, lead_hi + 1) if (p - root) % 12 in PENTA[mode]]

    def snap(p, bar, want_root=False):
        pcs = [pitch(chord_deg(bar)) % 12] if want_root else triad_pcs(bar)
        cands = [q for q in range(lead_lo - 2, lead_hi + 3) if q % 12 in pcs]
        return min(cands, key=lambda q: (abs(q - p), q))

    def contour(rng, n):
        return [0] + [rng.pick([1, 1, 2, -1] if k < n / 2 else [-1, -1, -2, 1]) for k in range(1, n)]

    rng = Lcg(seed * 7919 + 13)
    rh_a = RHYTHMS[rng.next() % 5]
    rh_b = RHYTHMS[(RHYTHMS.index(rh_a) + 1 + rng.next() % 4) % 5]
    start_a = len(ladder) // 3 + rng.next() % 2
    forms = {
        "A": (rh_a, contour(rng, len(rh_a)), start_a),
        "B": (rh_b, contour(rng, len(rh_b)), start_a + 2),
        "C": (RHYTHMS[5], contour(rng, len(RHYTHMS[5])), start_a + 3),
    }

    def phrase(form, bar0):
        """Notes of one two-bar phrase starting at bar `bar0`: [(step in phrase, midi, length)]."""
        kind = form[0]
        rh, steps, start = forms[kind]
        steps = list(steps)
        if form == "A2":
            steps[-1] += 1  # question
        idx, out, prev = start, [], None
        for k, ((s, ln), d) in enumerate(zip(rh, steps)):
            idx += d
            if not 0 <= idx < len(ladder):  # bounce off the ends of the range instead of sticking there
                idx -= 2 * d
            p = ladder[idx]
            bar = bar0 + s // S16
            last = k == len(rh) - 1
            if form == "A3" and last:
                p = snap(p, bar, want_root=True)  # answer: land on the chord's root
            elif s % 8 == 0 or ln >= 4:
                p = snap(p, bar)
            if p == prev and not last:  # snapping landed on the note just played: move on in the walk's direction
                p = ladder[min(max(idx + (1 if d > 0 else -1), 0), len(ladder) - 1)]
            prev = p
            out.append((s, p, ln))
        return out

    def lead(bars, vel, phrases=None, offset=0):
        order = ["A", "A2", "B", "A3", "A", "A2", "C", "A3"]
        ev = []
        for w in range(bars // 2):
            if phrases is not None and w % 4 not in phrases:
                continue
            for s, p, ln in phrase(order[w % 8], w * 2 + offset):
                ev.append([w * 32 + s, name(p), ln * 0.92, vel * (1.0 if s % 8 == 0 else 0.82)])
        return ev

    # ---- drums ------------------------------------------------------------------------------------
    def drums(bars, which, kvel=1.0, fill=True, first=0):
        g = groove[which]
        parts = {k: [] for k in ("kick", "clap", "hat", "ohat", "shaker", "rim", "snare", "tom", "crash", "rise", "fall")}
        for b in range(first, bars):
            last = fill and b == bars - 1
            turn = fill and b % 4 == 3 and not last
            for inst in ("kick", "clap", "hat", "ohat", "shaker", "rim"):
                for s, v in g[inst]:
                    if last and which == "hi" and inst in ("clap", "ohat", "rim") and s >= 8:
                        continue  # the roll takes over
                    # A touch of deterministic velocity movement keeps the hats from sounding like a machine gun.
                    hv = v * (0.9 + 0.1 * ((b * 7 + s * 3) % 5) / 4) if inst in ("hat", "shaker") else v
                    parts[inst].append([b * S16 + s, "C2", 1.0, hv * (kvel if inst == "kick" else 1.0)])
            if turn:  # every fourth bar: a pick-up into the next
                parts["kick"].append([b * S16 + 14, "C2", 1.0, .65 * kvel])
                parts["ohat"].append([b * S16 + 14, "C2", 1.0, .7])
            if last and which == "hi":  # the loop's last bar: snare roll and toms into the downbeat
                for k, s in enumerate((8, 10, 12, 13, 14, 15)):
                    parts["snare"].append([b * S16 + s, "C2", 1.0, .45 + .1 * k])
                for s, n in ((11, 50), (14, 47), (15, 43)):
                    parts["tom"].append([b * S16 + s, name(n), 1.0, .8])
                parts["rise"].append([b * S16, "C4", 16, .8])
            elif last:
                parts["kick"].append([b * S16 + 14, "C2", 1.0, .6 * kvel])
                parts["snare"].append([b * S16 + 15, "C2", 1.0, .5])
                parts["rise"].append([b * S16 + 8, "C4", 8, .5])
        if fill and first == 0:  # the downbeat of the loop
            parts["crash"].append([0, "C2", 8, .9 if which == "hi" else .45])
            parts["fall"].append([0, "C4", 12 if which == "hi" else 8, .7 if which == "hi" else .4])
        hi = which == "hi"
        mix = dict(kick=(1.0, 0, 0), clap=(.55, .08, .25), snare=(.5, -.05, .2), rim=(.4, .25, .15), hat=(.34 if hi else .3, -.25, .05),
                   ohat=(.3, .2, .08), shaker=(.3, .35, .05), tom=(.6, -.15, .2), crash=(.3, .1, .3), rise=(.22, 0, .3), fall=(.2, 0, .3))
        return [track(i, ev, mix[i][0], mix[i][1], reverb=mix[i][2]) for i, ev in parts.items() if ev]

    # ---- stems ------------------------------------------------------------------------------------
    S = {}
    # Intro: the last chords of the loop, so it resolves into bar 1. The pulse is there from the first bar
    # (kick on 1 and 3), the full four-on-the-floor arrives half way, a roll and riser lead into the loop.
    off = (B - I) % (cb * len(prog))
    half = I // 2
    intro_kick = [[b * S16 + s, "C2", 1.0, v] for b in range(I) for s, v in ([(0, .9), (8, .75)] if b < half else FOUR_LO)]
    intro_hat = [[b * S16 + s, "C2", 1.0, .6 + .1 * (b >= half)] for b in range(1, I) for s in (2, 6, 10, 14)]
    roll = [[(I - 1) * S16 + s, "C2", 1.0, .35 + .09 * k] for k, s in enumerate((8, 10, 12, 13, 14, 15))]
    bell_pings = [[b * S16 + s, name(place(pitch(chord_deg(b + off) + 4), 76)), 3, .6] for b in range(I) for s in (0, 6)]
    S["intro"] = {"bars": I, "tracks": [
        track("pad", pad_chords(I, off, 0, .55), .5, 0, reverb=.5, chorus=.5, hp=150, duck=pump(.35)),
        track("pluck", [[e[0], e[1], e[2], e[3] * (.5 + .5 * e[0] / (I * S16))] for e in arp(I, .8, False, 1, off)], .5, .2, delay=echo(), reverb=.3, hp=200),
        track("bell", bell_pings, .32, -.3, delay=echo(3, .5, .45), reverb=.5, hp=300),
        track("bass", bass(I, groove["bass"]["lo"], 1.0, off, half), .55, 0, duck=pump(.5)),
        track("kick", intro_kick, 1.0),
        track("hat", intro_hat, .28, -.25, reverb=.05),
        track("snare", roll, .5, -.05, reverb=.2),
        track("rise", [[half * S16, "C4", (I - half) * S16, .9]], .25, 0, reverb=.3),
    ]}

    def main(which):
        hi = which == "hi"
        tr = [
            track("bass", bass(B, groove["bass"][which]), .62 if hi else .55, 0, duck=pump(.6 if hi else .5, 1.6)),
            track("pluck", arp(B, .85 if hi else .75, hi), .42 if hi else .45, .3, delay=echo(), reverb=.3, hp=200, duck=pump(.3)),
            track("pad", pad_chords(B, 0, 0, .5), .3 if hi else .26, 0, reverb=.4, chorus=.5, hp=180, duck=pump(.55 if hi else .45, 2.0)),
            track("lead" if hi else "bell", lead(B, .8 if hi else .7, None if hi else (0, 2)), .34 if hi else .36, -.2,
                  delay=echo(4, .35, .28) if hi else echo(3, .5, .4), reverb=.4, chorus=.3 if hi else 0, hp=250, duck=pump(.2)),
        ]
        if hi:
            tr.append(track("stab", stabs(B, .8), .3, .15, delay=echo(3, .3, .2), reverb=.35, hp=250, duck=pump(.3)))
        return {"bars": B, "tracks": tr + drums(B, which, 1.0 if hi else .92)}

    S["beat"] = main("lo")
    S["beathigh"] = main("hi")

    # Pads: no drums; each breathes on every beat (duck every 4 steps) so it locks to either main loop.
    S["pad_normal"] = {"bars": B, "tracks": [
        track("pad", pad_chords(B, 0, 0, .6), .42, 0, reverb=.5, chorus=.6, hp=160, duck=pump(.4, 2.0, 4)),
        track("bell", [[b * S16 + 8, name(place(pitch(chord_deg(b) + 4), 79)), 6, .5] for b in range(0, B, 2)], .22, .3,
              delay=echo(6, .5, .5), reverb=.6, hp=400),
    ]}
    sparkle = []
    for b in range(B):
        v = voicing(b, 67)
        for n, s in enumerate(range(0, 16, 2)):
            k, up = arp_shape[(n + b) % len(arp_shape)]
            sparkle.append([b * S16 + s, name(v[k] + 12 * (1 + up)), 1.6, .6 if s % 4 == 0 else .42])
    S["pad_winning"] = {"bars": B, "tracks": [
        track("bright", pad_chords(B, 0, 1, .5, 55, False), .3, 0, reverb=.5, chorus=.5, hp=250, duck=pump(.5, 2.0, 4)),
        track("sub", [[b * S16 + s, name(bass_note(b)), ln, .9] for b in range(B) for s, ln in ((0, 7.5), (8, 7.5))], .36, 0, duck=pump(.7, 1.8, 4)),
        track("bell", sparkle, .2, -.25, delay=echo(3, .5, .45), reverb=.55, hp=500),
        track("shaker", [[b * S16 + s, "C2", 1.0, .5 if s % 4 == 2 else .28] for b in range(B) for s in range(16)], .2, .35, reverb=.1),
    ]}
    # Losing: the chords drop an octave and darken; a heartbeat (low tom on 1 and the "a" of 1) and a
    # hollow bell falling through the chord.
    falling = []
    for b in range(0, B, 2):
        v = voicing(b, 62)
        for k, s in enumerate((0, 6, 12)):
            falling.append([b * S16 + s, name(v[2 - k] + 12), 5, .55 - .08 * k])
    S["pad_losing"] = {"bars": B, "tracks": [
        track("dark", pad_chords(B, 0, -1, .65, 55, False), .5, 0, reverb=.6, chorus=.4, hp=70, duck=pump(.35, 2.5, 4)),
        track("dark", [[b * S16, name(bass_note(b) + 19), cb * S16 - .5, .4] for b in range(0, B, cb)], .3, .2, reverb=.6, chorus=.4, hp=120),
        track("bell", falling, .22, -.2, delay=echo(6, .6, .55, .45), reverb=.7, hp=350),
        track("tom", [[b * S16 + s, name(bass_note(b)), 1.0, v] for b in range(B) for s, v in ((0, .9), (3, .55))], .4, 0, reverb=.25),
    ]}

    # Fills: one-shots that can land on any bar, so tonic pentatonic only. All of them end early enough
    # for their echoes to ring out before the cut.
    def penta(i, base=72):
        p = PENTA[mode]
        return place(root, base) + p[i % 5] + 12 * (i // 5)

    frng = Lcg(seed * 31 + 7)
    fills = []
    # 1: a bell line that climbs and falls back.
    line, i = [], 2
    for k, (s, ln) in enumerate(RHYTHMS[frng.next() % 5]):
        i = min(max(i + ([1, 2, 1, -1] if k < 4 else [-1, -2, -1, 1])[frng.next() % 4], 0), 9)
        line.append([s, name(penta(i)), ln, .7])
    fills.append({"bars": 4, "tracks": [
        track("bell", line, .36, .25, delay=echo(3, .55, .5), reverb=.6, hp=350),
        track("fall", [[0, "C4", 16, .5]], .14, 0, reverb=.4)]})
    # 2: a drum break: riser and accelerating snare into a crash and toms.
    brk_snare = [[s, "C2", 1.0, .35 + .04 * k] for k, s in enumerate([0, 4, 8, 10, 12, 13, 14, 15])]
    brk_tom = [[16 + s, name(n), 1.0, v] for s, n, v in ((0, 50, .9), (3, 47, .8), (6, 45, .8), (8, 43, .9), (11, 47, .7), (14, 43, .8))]
    fills.append({"bars": 4, "tracks": [
        track("rise", [[0, "C4", 16, .9]], .26, 0, reverb=.3),
        track("snare", brk_snare, .42, -.05, reverb=.25),
        track("crash", [[16, "C2", 8, .9]], .3, .1, reverb=.35),
        track("tom", brk_tom, .55, -.15, reverb=.3),
        track("fall", [[16, "C4", 16, .7]], .18, 0, reverb=.3)]})
    # 3: a pluck cascade running up two octaves and back.
    run = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 8, 7, 6, 5, 4, 3]
    casc = [[s, name(penta(run[s % 16], 60 + 12 * (s // 16))), .9, .75 if s % 4 == 0 else .5] for s in range(32)]
    fills.append({"bars": 4, "tracks": [
        track("pluck", casc, .4, -.2, delay=echo(3, .5, .45), reverb=.45, hp=250),
        track("bell", [[32, name(penta(5)), 8, .7], [32, name(penta(7)), 8, .6]], .28, .2, delay=echo(6, .5, .5), reverb=.6, hp=350)]})
    # 4: the tune's own first phrase, an octave up, as an answer from far away.
    ans = [[s, name(p + 12 if (p - root) % 12 in PENTA[mode] else penta(2, 84)), ln, .65] for s, p, ln in phrase("A", 0)]
    fills.append({"bars": 4, "tracks": [
        track("bell", ans, .34, 0, delay=echo(4, .55, .5), reverb=.7, hp=400, chorus=.3),
        track("shaker", [[s, "C2", 1.0, .2 + .5 * s / 32] for s in range(0, 32)], .18, .3, reverb=.1)]})
    S["fills"] = fills
    return {
        "format": "aurora-song/1",
        "name": spec["name"],
        "sample_rate": 44100,
        "seed": seed,
        "master": spec.get("master", 1.0),
        "reverb": {"decay": spec.get("reverb", 2.2), "damp": spec.get("damp", 0.35)},
        "instruments": PALETTES[spec["palette"]],
        "stems": S,
    }


# --- mix balance ---------------------------------------------------------------------------------
# The level each track should measure on its own in the rendered stem ("rms", or "peak" for drums), by
# stem class and instrument (a list when a stem uses the instrument on several tracks). gen_all.py
# renders each song, compares, and rescales the track gains to these, so every palette comes out with the
# same balance: kick on top, bass under it, hats audible on a phone speaker, pads well below the main loops
# (target RMS: main loops 0.03-0.07, pads 0.011-0.021).
P, R = "peak", "rms"
TARGETS = {
    "beathigh": dict(kick=(P, .30), bass=(R, .032), pluck=(R, .028), pad=(R, .026), lead=(R, .032), stab=(R, .018), clap=(P, .22), hat=(P, .11),
                     ohat=(P, .14), shaker=(P, .07), rim=(P, .10), snare=(P, .17), tom=(P, .20), crash=(P, .10), rise=(P, .14), fall=(P, .09)),
    "beat": dict(kick=(P, .28), bass=(R, .028), pluck=(R, .026), pad=(R, .024), bell=(R, .024), clap=(P, .13), hat=(P, .10), ohat=(P, .10),
                 shaker=(P, .06), rim=(P, .10), snare=(P, .10), crash=(P, .05), rise=(P, .08), fall=(P, .06)),
    "intro": dict(pad=(R, .024), pluck=(R, .020), bell=(R, .014), bass=(R, .026), kick=(P, .27), hat=(P, .09), snare=(P, .15), rise=(P, .15)),
    "pad_normal": dict(pad=(R, .021), bell=(R, .006)),
    "pad_winning": dict(bright=(R, .016), sub=(R, .015), bell=(R, .010), shaker=(P, .035)),
    "pad_losing": dict(dark=[(R, .020), (R, .008)], bell=(R, .008), tom=(P, .10)),
    "fill": dict(bell=(R, .019), pluck=(R, .019), fall=(P, .06), rise=(P, .13), snare=(P, .12), crash=(P, .10), tom=(P, .20), shaker=(P, .04)),
}


def stem_list(song):
    st = song["stems"]
    return [(k, st[k]) for k in ("intro", "beat", "beathigh", "pad_normal", "pad_winning", "pad_losing")] + [
        (f"fill{i + 1}", f) for i, f in enumerate(st["fills"])]


def balance(song, levels):
    """Scale track gains towards TARGETS. `levels` = {stem name: [(instrument, rms, peak), ...]} as printed by
    `au2-music levels`. Returns the largest correction applied (1.0 = already balanced)."""
    worst = 1.0
    for stem_name, stem in stem_list(song):
        targets = TARGETS["fill" if stem_name.startswith("fill") else stem_name]
        seen = {}
        for t, (inst, rms, peak) in zip(stem["tracks"], levels[stem_name]):
            assert inst == t["instrument"], (stem_name, inst, t["instrument"])
            k = seen.get(inst, 0)
            seen[inst] = k + 1
            want = targets[inst]
            kind, value = want[k] if isinstance(want, list) else want
            have = peak if kind == P else rms
            if have > 1e-6:
                f = min(max(value / have, 0.25), 4.0)
                t["gain"] = round(min(t["gain"] * f, 4.0), 3)
                worst = max(worst, f, 1 / f)
    return worst


def dump(song):
    """JSON with one event per line: readable diffs without a 10 000-line file."""
    s = json.dumps(song, indent=1)
    s = re.sub(r'\[\s*(\d+),\s*("[^"]+"),\s*([-\d.e]+),\s*([-\d.e]+)\s*\]', r'[\1,\2,\3,\4]', s)
    return re.sub(r'(\[\d+,"[^"]+",[-\d.e]+,[-\d.e]+\],?)\s*\n\s*(?=\[\d)', r'\1 ', s)
