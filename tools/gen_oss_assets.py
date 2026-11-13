"""Synthesize the audio, textures and mesh that the open-source build embeds. Everything is generated from scratch.

    python -I tools/gen_oss_assets.py <assets_dir> [--noteseq=<noteseq.rs>]

Writes, under the same names the Rust code `include_bytes!`s:
  sfx_<library>_<note>.ogg  (9 instrument libraries x 20 notes, plus sun hit/heal sounds)
  sfx_{select,selectall,order,repeataction,synthetic_upgrade,supermassive_level4,supermassive_level5}.ogg
  fx/{cloud,softdiamond,deadsun}.png, swirl/*.png, rock.bin
Deterministic (fixed seeds). Needs numpy and soundfile (libsndfile writes Ogg Vorbis).
"""
import struct
import sys
import zlib
from pathlib import Path

import numpy as np
import soundfile as sf

SR = 44100
RNG = np.random.default_rng(20261113)

# ---------------------------------------------------------------- notes
# Diatonic run C3..C5 and the five sharps the note tables use (note name -> MIDI number).
NOTES = {
    "c1": 48, "d1": 50, "e1": 52, "f1": 53, "g1": 55, "a2": 57, "b2": 59,
    "c2": 60, "d2": 62, "e2": 64, "f2": 65, "g2": 67, "a3": 69, "b3": 71, "c3": 72,
    "d1s": 51, "g1s": 56, "a2s": 58, "c2s": 61, "d2s": 63,
}
LIBS = ["synthetic", "acoustic", "percussion", "analog", "popcorn", "lunar", "voyager", "aubade", "solis"]
ATTACK_AT = 0.25   # the Rust tables play each sample from ~0.25 s, so the note starts there
LENGTH = 1.1


def hz(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def env(n, a, d, s, r_start, r):
    """Attack/decay to sustain level s, release from sample r_start over r seconds (all in seconds)."""
    t = np.arange(n) / SR
    e = np.minimum(t / max(a, 1e-4), 1.0)
    e = e * (s + (1 - s) * np.exp(-np.maximum(t - a, 0) / max(d, 1e-4)))
    e = e * np.clip(1 - (t - r_start) / r, 0, 1)
    return e


def lowpass(x, fc):
    a = np.exp(-2 * np.pi * fc / SR)
    y = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[i] = acc
    return y


def voice(lib, f, n):
    t = np.arange(n) / SR
    ph = 2 * np.pi * f * t
    if lib == "synthetic":     # soft saw-ish pluck
        x = sum(np.sin(k * ph) / k for k in range(1, 7)) * env(n, 0.004, 0.18, 0.15, n / SR - 0.2, 0.2)
    elif lib == "acoustic":    # Karplus-Strong string
        p = int(SR / f)
        buf = RNG.uniform(-1, 1, p)
        out = np.empty(n)
        for i in range(n):
            out[i] = buf[i % p]
            buf[i % p] = 0.4985 * (buf[i % p] + buf[(i + 1) % p])
        x = out
    elif lib == "percussion":  # tuned mallet: sine + inharmonic partial + noise tick
        x = np.sin(ph) * np.exp(-t * 7) + 0.4 * np.sin(2.76 * ph) * np.exp(-t * 13) + 0.2 * RNG.uniform(-1, 1, n) * np.exp(-t * 60)
    elif lib == "analog":      # detuned saws through a falling low-pass
        x = sum(((f * d * t) % 1.0) * 2 - 1 for d in (0.995, 1.0, 1.005)) / 3
        x = lowpass(x, min(900 + 4 * f, 6000)) * env(n, 0.006, 0.3, 0.3, n / SR - 0.25, 0.25)
    elif lib == "popcorn":     # short square blip
        x = np.sign(np.sin(ph)) * 0.5 * np.exp(-t * 14)
    elif lib == "lunar":       # sine bell with slow tremolo
        x = (np.sin(ph) + 0.3 * np.sin(2 * ph)) * np.exp(-t * 3.2) * (0.8 + 0.2 * np.sin(2 * np.pi * 5 * t))
    elif lib == "voyager":     # slow pad: detuned triangle pair
        tri = lambda p: 2 / np.pi * np.arcsin(np.sin(p))
        x = (tri(ph) + tri(ph * 1.004)) * 0.5 * env(n, 0.09, 0.5, 0.6, n / SR - 0.3, 0.3)
    elif lib == "aubade":      # breathy flute-like sine with noise
        x = np.sin(ph) * env(n, 0.05, 0.4, 0.5, n / SR - 0.3, 0.3) + 0.04 * lowpass(RNG.uniform(-1, 1, n), 4000)
    else:                      # solis: FM bell
        x = np.sin(ph + 2.2 * np.exp(-t * 5) * np.sin(3.5 * ph)) * np.exp(-t * 3.5)
    return x


def note_signal(lib, midi, attack_at=ATTACK_AT, length=LENGTH):
    n = int((length - attack_at) * SR)
    x = voice(lib, hz(midi), n)
    out = np.zeros(int(length * SR))
    out[int(attack_at * SR):] = x
    pk = np.abs(out).max() or 1.0
    return out / pk * 0.8


def write_ogg(path, x):
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), x.astype(np.float32), SR, format="OGG", subtype="VORBIS")


def sun_sound(lib, heal):
    t = np.arange(int(0.8 * SR)) / SR
    f = 330 if heal else 140
    sweep = f * (1 + 0.8 * t) if heal else f * (1 - 0.5 * t)
    x = np.sin(2 * np.pi * np.cumsum(sweep) / SR) * np.exp(-t * (5 if heal else 9))
    if not heal:
        x += 0.3 * RNG.uniform(-1, 1, len(t)) * np.exp(-t * 40)
    out = np.zeros(int(LENGTH * SR))
    at = 0.06 if heal else ATTACK_AT
    out[int(at * SR):int(at * SR) + len(x)] = x[: len(out) - int(at * SR)]
    return out / (np.abs(out).max() or 1) * 0.7


def ui_sounds(out):
    def tone(freqs, dur, decay):
        t = np.arange(int(dur * SR)) / SR
        return sum(np.sin(2 * np.pi * f * t) for f in freqs) / len(freqs) * np.exp(-t * decay)

    def seq(parts):
        return np.concatenate(parts)

    write_ogg(out / "sfx_select.ogg", tone([880, 1320], 0.12, 25) * 0.6)
    write_ogg(out / "sfx_selectall.ogg", seq([tone([660], 0.07, 20), tone([880], 0.07, 20), tone([1320], 0.12, 22)]) * 0.6)
    write_ogg(out / "sfx_order.ogg", seq([tone([523], 0.06, 22), tone([784], 0.1, 24)]) * 0.6)
    write_ogg(out / "sfx_repeataction.ogg", seq([tone([784], 0.05, 25), tone([784], 0.05, 25)]) * 0.5)
    write_ogg(out / "sfx_synthetic_upgrade.ogg", seq([tone([440 * 2 ** (i / 12)], 0.09, 12) for i in (0, 4, 7, 12)]) * 0.6)
    for lvl, base in ((4, 110), (5, 82.4)):
        t = np.arange(int(2.2 * SR)) / SR
        x = sum(np.sin(2 * np.pi * base * k * t * (1 + 0.15 * t)) / k for k in (1, 2, 3, 5))
        x = x * np.minimum(t / 0.3, 1) * np.exp(-t * 1.1)
        x += 0.15 * lowpass(RNG.uniform(-1, 1, len(t)), 1200) * np.exp(-t * 1.5)
        write_ogg(out / f"sfx_supermassive_level{lvl}.ogg", x / np.abs(x).max() * 0.8)


# ---------------------------------------------------------------- images
def png(path, w, h, pixels, color_type):
    """pixels: uint8 array (h, w) for grey (type 0) or (h, w, 3|4) for RGB (2) / RGBA (6)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    raw = b"".join(b"\x00" + pixels[y].tobytes() for y in range(h))

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, color_type, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def tile_noise(n, beta, seed):
    """Tileable noise in [0,1]: white noise shaped by 1/f^beta in the Fourier domain (periodic by construction)."""
    r = np.random.default_rng(seed)
    f = np.fft.fftfreq(n)[:, None] ** 2 + np.fft.fftfreq(n)[None, :] ** 2
    f[0, 0] = 1
    spec = np.fft.fft2(r.standard_normal((n, n))) / f ** (beta / 2)
    spec[0, 0] = 0
    a = np.real(np.fft.ifft2(spec))
    a = (a - a.min()) / (a.max() - a.min())
    return a


def u8(a):
    return np.clip(a * 255 + 0.5, 0, 255).astype(np.uint8)


def swirl_masks(out):
    n = 512
    yy, xx = np.mgrid[0:n, 0:n] / n
    # marble: banded sine of a warped coordinate (integer frequencies keep it tileable)
    w1 = tile_noise(n, 2.2, 11)
    w2 = tile_noise(n, 2.0, 12)
    png(out / "swirl/marble_1.png", n, n, u8(0.5 + 0.5 * np.sin(2 * np.pi * (3 * xx + 2 * yy + 2.2 * w1))), 0)
    png(out / "swirl/marble_2.png", n, n, u8(np.clip(0.2 + 0.9 * w2 ** 1.5, 0, 1)), 0)
    # oil: smooth blobs with contrast
    for i, seed in ((1, 21), (2, 22)):
        a = tile_noise(n, 3.0, seed)
        png(out / f"swirl/oil_{i}.png", n, n, u8(np.clip((a - 0.35) * 3.0, 0, 1)), 0)
    # stripes: warped stripes
    w = tile_noise(n, 2.5, 31)
    png(out / "swirl/stripes_1.png", n, n, u8(0.5 + 0.5 * np.sin(2 * np.pi * (8 * yy + 1.5 * w))), 0)
    # bubbles: periodic cellular distance (Worley) -> soft rings
    pts = np.random.default_rng(41).uniform(0, 1, (24, 2))
    d = np.full((n, n), 9.0)
    for px, py in pts:
        dx = np.abs(xx - px)
        dy = np.abs(yy - py)
        dx = np.minimum(dx, 1 - dx)
        dy = np.minimum(dy, 1 - dy)
        d = np.minimum(d, np.hypot(dx, dy))
    png(out / "swirl/bubbles_2.png", n, n, u8(np.clip(1 - d * 9, 0, 1) ** 1.5), 0)


def fx(out):
    n = 64
    yy, xx = np.mgrid[0:n, 0:n]
    r = np.hypot(xx - (n - 1) / 2, yy - (n - 1) / 2) / (n / 2)
    cloud = np.clip(1 - r, 0, 1) ** 1.6 * (0.7 + 0.3 * tile_noise(n, 2.0, 51))
    png(out / "fx/cloud.png", n, n, u8(cloud), 0)
    dx, dy = np.abs(xx - (n - 1) / 2) / (n / 2), np.abs(yy - (n - 1) / 2) / (n / 2)
    diamond = np.clip(1 - (dx + dy) * 1.1, 0, 1) ** 0.8
    png(out / "fx/softdiamond.png", n, n, u8(diamond), 0)
    m = 256
    yy, xx = np.mgrid[0:m, 0:m]
    r = np.hypot(xx - m / 2, yy - m / 2) / (m / 2)
    shell = np.clip(1 - r, 0, 1) ** 0.5 * (0.55 + 0.45 * tile_noise(m, 2.4, 61))
    rgb = np.stack([shell * 0.35, shell * 0.30, shell * 0.28], axis=-1)
    png(out / "fx/deadsun.png", m, m, u8(rgb), 2)


def rock(out):
    """A faceted rock: an icosphere (12 verts, 20 faces) subdivided once (42 verts, 80 faces), then jittered."""
    t = (1 + 5 ** 0.5) / 2
    v = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0), (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t), (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
    f = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
         (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
    v = [np.array(p, float) / np.linalg.norm(p) for p in v]
    cache = {}

    def mid(a, b):
        k = (min(a, b), max(a, b))
        if k not in cache:
            m = v[a] + v[b]
            v.append(m / np.linalg.norm(m))
            cache[k] = len(v) - 1
        return cache[k]

    faces = []
    for a, b, c in f:
        ab, bc, ca = mid(a, b), mid(b, c), mid(c, a)
        faces += [(a, ab, ca), (b, bc, ab), (c, ca, bc), (ab, bc, ca)]
    r = np.random.default_rng(71)
    verts = [p * r.uniform(0.7, 1.15) * np.array([1.0, 0.85, 0.9]) for p in v]
    out.mkdir(parents=True, exist_ok=True)
    blob = struct.pack("<HH", len(verts), len(faces))
    blob += b"".join(struct.pack("<3f", *map(float, p)) for p in verts)
    blob += b"".join(struct.pack("<3H", *fc) for fc in faces)
    (out / "rock.bin").write_bytes(blob)


def noteseq_rs(path):
    """Write `noteseq.rs`: 8 even, 8 winning, 8 losing sixteen-note sequences, from our own seeded random walks."""
    r = np.random.default_rng(99)
    even_scale = ["c1", "d1", "e1", "g1", "a2", "c2", "d2", "e2", "g2", "a3", "c3"]            # C major pentatonic, two octaves
    win_scale = ["c1", "e1", "g1", "c2", "e2", "g2", "c3"]                                      # major arpeggio notes
    lose_scale = ["c1", "d1", "d1s", "f1", "g1", "g1s", "a2s", "c2", "d2", "d2s", "f2", "g2"]   # C minor

    def walk(scale, start, bias):
        i, out = start, []
        for _ in range(16):
            out.append(scale[i])
            i = int(np.clip(i + r.choice([-2, -1, 0, 1, 2], p=bias), 0, len(scale) - 1))
        return out

    sets = {
        "EVEN": ("the even-battle sequences", [walk(even_scale, int(r.integers(2, 8)), [0.2, 0.3, 0.1, 0.3, 0.1]) for _ in range(8)]),
        "WINNING": ("the winning sequences (drift upwards)", [walk(win_scale, int(r.integers(0, 3)), [0.05, 0.1, 0.2, 0.45, 0.2]) for _ in range(8)]),
        "LOSING": ("the losing sequences (drift downwards, minor)", [walk(lose_scale, int(r.integers(5, 11)), [0.2, 0.45, 0.2, 0.1, 0.05]) for _ in range(8)]),
    }
    out = ["//! The 24 sixteen-note battle sequences (`notes.rs` picks one per bar from the battle status).",
           "//! Each entry is a note name as used by `notes::cue_for`. Generated by `tools/gen_oss_assets.py`.", ""]
    for name, (doc, seqs) in sets.items():
        out.append(f"/// {doc}.")
        out.append(f"pub const {name}: [[&str; 16]; 8] = [")
        out += ["    [" + ", ".join(f'"{n}"' for n in sq) + "]," for sq in seqs]
        out.append("];")
    Path(path).write_text("\n".join(out) + "\n", newline="\n")


def main():
    out = Path(sys.argv[1])
    ns = next((a[10:] for a in sys.argv[2:] if a.startswith("--noteseq=")), None)
    if ns:
        noteseq_rs(ns)
    for lib in LIBS:
        for name, midi in NOTES.items():
            write_ogg(out / f"sfx_{lib}_{name}.ogg", note_signal(lib, midi))
        if lib in ("synthetic", "acoustic", "percussion"):   # the other libraries reuse the synthetic sun sounds
            write_ogg(out / f"sfx_{lib}_hitsun.ogg", sun_sound(lib, False))
            write_ogg(out / f"sfx_{lib}_healsun.ogg", sun_sound(lib, True))
        if lib == "percussion":
            write_ogg(out / "sfx_percussion_upgrade.ogg", sun_sound(lib, True))
    ui_sounds(out)
    swirl_masks(out)
    fx(out)
    rock(out)
    print(f"generated assets in {out}")


if __name__ == "__main__":
    main()
