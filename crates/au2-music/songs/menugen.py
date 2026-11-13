#!/usr/bin/env python3
"""Writes menu.song.json: the title / main-menu music, a long haunting drone piece.

A title-screen track: quiet at first, low drones that swell, no
beat. In the song format this is: one 32-bar (68 s) free-running loop in D minor, one chord
per eight bars (17 s), everything with multi-second attacks and releases, so notes bleed into each other and
never start or stop audibly. Nothing sits on the beat grid; the other stems of the file are empty
placeholders (the format requires them).

    python menugen.py [path/to/au2-music]     # optional: also prints per-track levels
"""
import json, os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from songgen import dump

BARS = 32
SPAN = 128  # steps per chord = 8 bars = 17.07 s

# One chord per span: drone root and fifth, the pad voicing (open, low to high), the ghost's two notes and a high
# overtone. D minor, Bb maj7, G minor, A major (the dominant pulls back to the top of the loop).
CHORDS = [
    dict(root="D2", fifth="A2", pad=["D3", "A3", "F4", "E5"], lead=["A4", "F4"], hi="A5"),
    dict(root="Bb1", fifth="F2", pad=["Bb2", "F3", "A3", "D4"], lead=["D5", "C5"], hi="F5"),
    dict(root="G1", fifth="D2", pad=["G2", "D3", "Bb3", "F4"], lead=["Bb4", "A4"], hi="D6"),
    dict(root="A1", fifth="E2", pad=["A2", "E3", "C#4", "G4"], lead=["C#5", "E5"], hi="E6"),
]


def synth(osc, attack, release, **kw):
    d = dict(type="synth", osc=osc, attack=attack, decay=1.0, sustain=1.0, release=release)
    d.update(kw)
    return d


INSTRUMENTS = {
    # The floor: a pure sine, felt more than heard on a phone, plus a saw drone that carries the pitch.
    "sub": synth([dict(wave="sine", level=1.0)], 4.0, 5.0),
    "drone": synth([dict(wave="saw", level=0.6), dict(wave="saw", level=0.4, octave=1, detune=7)], 4.5, 5.5,
                   lowpass=dict(cutoff=420, q=1.2), unison=3, spread=14, width=0.6, drive=0.6),
    # Wide, slow, breathing choir of detuned saws behind a dark lowpass.
    "choir": synth([dict(wave="saw", level=0.7), dict(wave="tri", level=0.5, octave=1)], 4.5, 6.5,
                   lowpass=dict(cutoff=1000, q=1.0), unison=5, spread=22, width=1.0,
                   vibrato=dict(rate=4.2, depth=9, delay=2.0), drive=0.4),
    # The ghost: a thin voice with a late, wide vibrato, slow in and slow out.
    "ghost": synth([dict(wave="sine", level=1.0), dict(wave="sine", level=0.25, octave=1)], 2.2, 4.5,
                   unison=2, spread=9, width=0.8, vibrato=dict(rate=4.6, depth=30, delay=1.4)),
    "shimmer": synth([dict(wave="sine", level=1.0)], 5.0, 6.5, unison=3, spread=18, width=1.0,
                     vibrato=dict(rate=5.2, depth=14, delay=2.0)),
    # Wind: noise squeezed through a resonant filter tuned to the chord.
    "breath": synth([dict(wave="noise", level=1.0)], 6.0, 6.0, lowpass=dict(cutoff=380, q=6.0, track=0.9)),
    # A far-off bell on every chord change; long ring, inharmonic.
    "toll": dict(type="synth", osc=[dict(wave="sine", level=1.0)], attack=0.004, decay=7.0, sustain=0.0, release=6.0,
                 fm=dict(ratio=2.4, index=2.6, decay=3.5)),
}


def pads():
    out = []
    for i, c in enumerate(CHORDS):
        for n in c["pad"]:
            out.append([i * SPAN, n, SPAN, 0.55])
    return out


def per_span(f, first=0):
    """`f(start_step, chord)` events for every chord from index `first` on (tracks enter one chord at a time)."""
    return [e for i, c in enumerate(CHORDS) if i >= first for e in f(i * SPAN, c)]


def track(inst, events, gain, pan=0.0, **fx):
    t = dict(instrument=inst, gain=gain, pan=pan)
    t.update(fx)
    t["events"] = events
    return t


def build():
    tracks = [
        track("sub", per_span(lambda s, c: [[s, c["root"], SPAN, 0.8]]), 0.07),
        track("drone", per_span(lambda s, c: [[s, c["root"], SPAN, 0.6], [s, c["fifth"], SPAN, 0.5]]), 0.08, reverb=0.35),
        track("choir", pads(), 0.042, chorus=0.5, reverb=0.8, hp=120),
        track("breath", per_span(lambda s, c: [[s + 8, c["pad"][1], SPAN - 8, 0.5]]), 0.077, pan=0.15, reverb=0.7),
        # The ghost is silent for the first chord (17 s of drones alone), then sings two long notes a chord.
        track("ghost", per_span(lambda s, c: [[s + 14, c["lead"][0], 42, 0.5], [s + 72, c["lead"][1], 46, 0.45]], first=1), 0.039,
              pan=-0.2, reverb=0.9, hp=200, delay=dict(steps=12, feedback=0.6, mix=0.4, pingpong=True, damp=0.55)),
        # Overtones high above, only in the last two chords so the piece swells and then thins before the loop.
        track("shimmer", [[2 * SPAN + 4, CHORDS[2]["hi"], SPAN - 8, 0.4], [3 * SPAN + 4, CHORDS[3]["hi"], SPAN - 8, 0.4]], 0.024,
              pan=0.3, reverb=1.0, hp=400),
        track("toll", per_span(lambda s, c: [[s, c["pad"][0][:-1] + str(int(c["pad"][0][-1]) + 1), 1, 0.55]]), 0.056,
              pan=0.1, reverb=1.0, delay=dict(steps=16, feedback=0.45, mix=0.35, pingpong=True, damp=0.5)),
    ]
    empty = dict(bars=1, tracks=[])
    return dict(
        format="aurora-song/1", name="menu", sample_rate=44100, seed=41, master=0.8,
        reverb=dict(decay=7.5, damp=0.55), instruments=INSTRUMENTS,
        stems=dict(intro=empty, beat=empty, beathigh=empty, pad_normal=empty, pad_winning=empty, pad_losing=empty,
                   fills=[empty], menu=dict(bars=BARS, tracks=tracks)),
    )


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    p = os.path.join(here, "menu.song.json")
    with open(p, "w", newline="\n") as f:
        f.write(dump(build()) + "\n")
    print("wrote", p)
    if sys.argv[1:]:
        print(subprocess.run([sys.argv[1], "levels", p], capture_output=True, text=True).stdout)
