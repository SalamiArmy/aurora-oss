#!/usr/bin/env python3
"""Writes every bundled song (*.song.json) next to this file, with the composer in songgen.py."""
import os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # works under python -I too
from songgen import balance, build, dump

# root = MIDI note of the tonic (48..59). progression = scale degrees (0-based), one chord per chord_bars;
# keep diminished triads out of it (degree 1 in minor, 5 in dorian, 4 in phrygian, 3 in lydian, 2 in mixolydian).
SONGS = [
    # A minor, Am F C G: the reference song (tests render it).
    dict(name="dawn", seed=1, root=57, mode="minor", progression=[0, 5, 2, 6], chord_bars=2, bars=8, palette="warm", groove="four", arp="up"),
    # D dorian, Dm F C G: glassy and unhurried.
    dict(name="tide", seed=3, root=50, mode="dorian", progression=[0, 2, 6, 3], chord_bars=2, bars=8, palette="glass", groove="half", arp="wide",
         reverb=2.8),
    # E minor, Em C Am Bm, a chord a bar: the fastest-moving one.
    dict(name="ember", seed=5, root=52, mode="minor", progression=[0, 5, 3, 4], chord_bars=1, bars=8, palette="neon", groove="drive", arp="skip",
         reverb=1.8),
    # C lydian over sixteen bars: C D Am G | C D Em G.
    dict(name="lumen", seed=11, root=48, mode="lydian", progression=[0, 1, 5, 4, 0, 1, 2, 4], chord_bars=2, bars=16, palette="bright",
         groove="pulse", arp="wide"),
    # F# phrygian, F#m G D Em: the dark one.
    dict(name="vesper", seed=17, root=54, mode="phrygian", progression=[0, 1, 5, 6], chord_bars=2, bars=8, palette="dusk", groove="broken",
         arp="down", reverb=3.0, damp=0.45),
    # G mixolydian, G F C G.
    dict(name="halcyon", seed=23, root=55, mode="mixolydian", progression=[0, 6, 3, 0], chord_bars=2, bars=8, palette="warm", groove="broken",
         arp="skip"),
    # F minor, Fm Ab Eb Bbm.
    dict(name="nadir", seed=29, root=53, mode="minor", progression=[0, 2, 6, 3], chord_bars=2, bars=8, palette="dusk", groove="pulse", arp="up",
         reverb=2.6),
    # D major, D A Bm G.
    dict(name="zenith", seed=37, root=50, mode="major", progression=[0, 4, 5, 3], chord_bars=2, bars=8, palette="bright", groove="four",
         arp="down"),
]



def measure(tool, path):
    """Run `au2-music levels` and parse it into {stem: [(instrument, rms, peak), ...]}."""
    out = subprocess.run([tool, "levels", path], capture_output=True, text=True, check=True).stdout
    levels, cur = {}, None
    for line in out.splitlines():
        w = line.split()
        if line.startswith(" "):
            levels[cur].append((w[0], float(w[2]), float(w[4])))
        else:
            cur = w[0]
            levels[cur] = []
    return levels


if __name__ == "__main__":
    # usage: gen_all.py [--balance path/to/au2-music] [song names...]
    # Without --balance the gains are the composer's first guesses; the bundled files are written with it.
    here = os.path.dirname(os.path.abspath(__file__))
    args = sys.argv[1:]
    tool = None
    if args[:1] == ["--balance"]:
        tool, args = os.path.abspath(args[1]), args[2:]
    for s in SONGS:
        if args and s["name"] not in args:
            continue
        p = os.path.join(here, s["name"] + ".song.json")
        song = build(s)

        def write():
            with open(p, "w", newline=chr(10)) as f:
                f.write(dump(song) + chr(10))

        write()
        if tool:
            for _ in range(3):
                worst = balance(song, measure(tool, p))
                write()
                if worst < 1.03:
                    break
        print("wrote", p)
