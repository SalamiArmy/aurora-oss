//! Deterministic stem renderer. All event timing is integer (a 16th step is a whole number of samples,
//! a bar is `128 * sample_rate / 60`), so a stem's length is exact and loop stems wrap tails around:
//! note releases, delay repeats, chorus, sidechain envelopes and the reverb tail of a loop stem all run
//! across the loop point, so the join is seamless.
//!
//! Signal path of a track: voices (stereo) -> high-pass -> chorus -> duck -> delay -> gain/pan -> stem bus,
//! with a post-fader send into the song's reverb. The stem bus is soft-clipped (`tanh`) after `master`.
use crate::format::{parse_note, Delay, Duck, Instrument, Reverb, Song, Stem, StemKind, Track, Wave};
use crate::{FPS, FRAMES_PER_BAR, FRAMES_PER_STEP, STEPS_PER_BAR};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// One stereo sample.
type Frame = [f32; 2];

/// One rendered stem: interleaved stereo f32.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedStem {
    pub kind: StemKind,
    pub sample_rate: u32,
    /// Interleaved L,R frames.
    pub samples: Vec<f32>,
    pub looped: bool,
}

impl RenderedStem {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    /// Length in game frames (multiple of 128).
    pub fn game_frames(&self) -> u32 {
        (self.frames() as u32 / (self.sample_rate / FPS)) as u32
    }
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.sample_rate as f32
    }
}

struct Rng(u64);
impl Rng {
    fn new(seed: u64, a: u64, b: u64, c: u64) -> Rng {
        let mut s = seed ^ 0x9E37_79B9_7F4A_7C15;
        for v in [a, b, c] {
            s = (s ^ v.wrapping_add(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            s ^= s >> 29;
        }
        Rng(s | 1)
    }
    /// -1..1
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// Render every stem of a song (validates first). Stems render in parallel; the result does not depend
/// on the thread count.
pub fn render_song(song: &Song) -> Result<Vec<RenderedStem>, Box<dyn std::error::Error>> {
    song.validate()?;
    let list = song.stem_list();
    let mut out: Vec<Option<RenderedStem>> = (0..list.len()).map(|_| None).collect();
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots = std::sync::Mutex::new(&mut out);
    std::thread::scope(|sc| {
        for _ in 0..workers.min(list.len()) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(kind, stem)) = list.get(i) else { break };
                let r = render_stem(song, kind, stem);
                slots.lock().unwrap()[i] = Some(r);
            });
        }
    });
    Ok(out.into_iter().map(|s| s.expect("every stem rendered")).collect())
}

pub fn samples_per_step(sample_rate: u32) -> usize {
    (sample_rate / FPS * FRAMES_PER_STEP) as usize
}

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

fn osc_sample(wave: Wave, phase: f32, dt: f32, rng: &mut Rng) -> f32 {
    match wave {
        Wave::Sine => (phase * TAU).sin(),
        Wave::Saw => 2.0 * phase - 1.0 - poly_blep(phase, dt),
        Wave::Square => {
            let v = if phase < 0.5 { 1.0 } else { -1.0 };
            v + poly_blep(phase, dt) - poly_blep((phase + 0.5).fract(), dt)
        }
        Wave::Tri => 4.0 * (phase - 0.5).abs() - 1.0,
        Wave::Noise => rng.next(),
    }
}

/// Zero-delay-feedback state variable filter (stable under fast cutoff sweeps).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    fn new(fc: f32, q: f32, sr: f32) -> Svf {
        let mut f = Svf::default();
        f.set(fc, q, sr);
        f
    }
    fn set(&mut self, fc: f32, q: f32, sr: f32) {
        let g = (PI * fc.clamp(15.0, sr * 0.45) / sr).tan();
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// (lowpass, bandpass, highpass)
    #[inline]
    fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - self.k * v1 - v2)
    }
}

fn mono(v: Vec<f32>) -> Vec<Frame> {
    v.into_iter().map(|s| [s, s]).collect()
}

/// Soft saturation that leaves full scale at 1 and lifts everything below it.
#[inline]
fn saturate(x: f32, drive: f32) -> f32 {
    let g = 1.0 + drive;
    (x * g).tanh() / g.tanh()
}

/// One note as a stereo buffer (gate + release/decay tail).
fn voice(inst: &Instrument, midi: i32, gate_samples: usize, vel: f32, sr: u32, rng: &mut Rng) -> Vec<Frame> {
    let srf = sr as f32;
    let secs = |t: f32| (t * srf) as usize;
    match inst {
        Instrument::Synth { osc, attack, decay, sustain, release, lowpass, unison, spread, width, fm, vibrato, drive } => {
            let rel = secs(*release).max(64);
            let total = gate_samples + rel;
            let a = (attack * srf).max(1.0);
            let d = (decay * srf).max(1.0);
            let base = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
            let nu = (*unison).clamp(1, 7) as usize;
            let stereo = nu > 1 && *width > 0.0;
            // Unison voice: frequency ratio and stereo gains. Neighbouring detunes go to opposite sides.
            let uni: Vec<(f32, f32, f32)> = (0..nu)
                .map(|u| {
                    let x = if nu > 1 { 2.0 * u as f32 / (nu - 1) as f32 - 1.0 } else { 0.0 };
                    let pan = width * x * if u % 2 == 0 { 1.0 } else { -1.0 };
                    let ang = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                    (2f32.powf(spread * x / 1200.0), ang.cos() * std::f32::consts::SQRT_2, ang.sin() * std::f32::consts::SQRT_2)
                })
                .collect();
            let no = osc.len();
            // A unison stack starts with scattered phases (a wide, soft front); a single voice starts at
            // zero so basses and plucks hit the same way every time.
            let mut phases: Vec<f32> = (0..nu * no).map(|_| if nu > 1 { rng.next() * 0.5 + 0.5 } else { 0.0 }).collect();
            let incs: Vec<f32> = uni
                .iter()
                .flat_map(|u| osc.iter().map(move |o| base * u.0 * 2f32.powf(o.octave as f32 + o.detune / 1200.0) / srf))
                .collect();
            let norm = 1.0 / (nu as f32).sqrt();
            let mut filt = lowpass.map(|l| (l, [Svf::default(); 2]));
            let key = lowpass.map_or(1.0, |l| 2f32.powf((midi as f32 - 57.0) / 12.0 * l.track));
            let mut fenv = 1.0f32;
            let fenv_k = lowpass.map_or(1.0, |l| if l.decay > 0.0 { (-1.0 / (l.decay * srf)).exp() } else { 1.0 });
            let (mut mph, mut fm_idx) = (0.0f32, fm.map_or(0.0, |f| f.index * (0.4 + 0.6 * vel)));
            let fm_k = fm.map_or(1.0, |f| if f.decay > 0.0 { (-1.0 / (f.decay * srf)).exp() } else { 1.0 });
            let fm_inc = fm.map_or(0.0, |f| base * f.ratio / srf);
            let vib = vibrato.map(|v| (TAU * v.rate / srf, v.depth * std::f32::consts::LN_2 / 1200.0, (v.delay * srf).max(1.0)));
            let mut out = Vec::with_capacity(total);
            let mut gate_level = 0.0;
            for i in 0..total {
                let fi = i as f32;
                let env = if i < gate_samples {
                    let e = if fi < a { fi / a } else { sustain + (1.0 - sustain) * (-5.0 * (fi - a) / d).exp() };
                    gate_level = e;
                    e
                } else {
                    let r = (i - gate_samples) as f32 / rel as f32;
                    gate_level * (-6.0 * r).exp() * (1.0 - r)
                };
                let vf = vib.map_or(1.0, |(w, depth, delay)| 1.0 + depth * (w * fi).sin() * (fi / delay).min(1.0));
                let pm = if fm.is_some() {
                    let m = fm_idx * (mph * TAU).sin() / TAU;
                    mph = (mph + fm_inc * vf).fract();
                    fm_idx *= fm_k;
                    m
                } else {
                    0.0
                };
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for (u, &(_, gl, gr)) in uni.iter().enumerate() {
                    let mut s = 0.0;
                    for (k, o) in osc.iter().enumerate() {
                        let j = u * no + k;
                        let ph = phases[j] + pm;
                        s += o.level * osc_sample(o.wave, ph - ph.floor(), incs[j].min(0.49), rng);
                        phases[j] = (phases[j] + incs[j] * vf).fract();
                    }
                    l += s * gl;
                    r += s * gr;
                }
                if !stereo {
                    l = (l + r) * 0.5;
                    r = l;
                }
                if let Some((lp, f)) = &mut filt {
                    if i % 16 == 0 {
                        let e = if lp.decay > 0.0 { fenv } else { env };
                        let fc = lp.cutoff * key + lp.env * (0.5 + 0.5 * vel) * e;
                        f[0].set(fc, lp.q, srf);
                        let (ic1, ic2) = (f[1].ic1, f[1].ic2);
                        f[1] = Svf { ic1, ic2, ..f[0] };
                    }
                    fenv *= fenv_k;
                    l = f[0].tick(l).0;
                    r = if stereo { f[1].tick(r).0 } else { l };
                }
                if *drive > 0.0 {
                    l = saturate(l * norm, *drive) / norm;
                    r = saturate(r * norm, *drive) / norm;
                }
                let g = env * vel * norm;
                out.push([l * g, r * g]);
            }
            out
        }
        Instrument::Kick { start, end, decay, sweep, click, drive } => {
            let n = secs(decay * 1.6);
            let mut ph = 0.0f32;
            let mut prev = 0.0f32;
            let fade = secs(0.004).max(1);
            mono(
                (0..n)
                    .map(|i| {
                        let t = i as f32 / srf;
                        // The body sweeps start -> end; a second, much faster sweep on top is the beater's knock.
                        let f = end + (start - end) * (-t * sweep).exp() + 2.5 * start * (-t * 260.0).exp();
                        ph += TAU * f / srf;
                        let amp = (-5.0 * t / decay).exp() * (1.0 - (-t * 3000.0).exp());
                        let body = saturate(ph.sin() * amp, *drive);
                        let nz = rng.next();
                        let tick = (nz + prev) * 0.5 * (-t * 700.0).exp() * click * 0.5;
                        prev = nz;
                        let tail = ((n - i) as f32 / fade as f32).min(1.0);
                        (body + tick) * tail * vel
                    })
                    .collect(),
            )
        }
        Instrument::Snare { tone, decay, noise } => {
            let n = secs(decay * 1.4);
            let (mut p1, mut p2) = (0.0f32, 0.0f32);
            let mut hp = Svf::new(1800.0, 0.8, srf);
            mono(
                (0..n)
                    .map(|i| {
                        let t = i as f32 / srf;
                        let bend = 1.0 + 0.5 * (-t * 60.0).exp();
                        p1 += TAU * tone * bend / srf;
                        p2 += TAU * tone * 1.48 * bend / srf;
                        let body = (p1.sin() + 0.6 * p2.sin()) * 0.6 * (-12.0 * t / decay).exp();
                        let nz = hp.tick(rng.next()).2 * (-6.0 * t / decay).exp();
                        ((1.0 - noise) * body + noise * nz) * (1.0 - (-t * 4000.0).exp()) * vel
                    })
                    .collect(),
            )
        }
        Instrument::Hat { decay, tone, attack } => {
            let n = secs(decay * 1.4 + attack);
            let mut hp = Svf::new(*tone, 1.2, srf);
            let fade = secs(0.003).max(1);
            mono(
                (0..n)
                    .map(|i| {
                        let t = i as f32 / srf;
                        let front = if *attack > 0.0 { (t / attack).min(1.0) } else { 1.0 };
                        let env = (-6.0 * (t - attack).max(0.0) / decay).exp() * front;
                        let tail = ((n - i) as f32 / fade as f32).min(1.0);
                        hp.tick(rng.next()).2 * 0.5 * env * tail * vel
                    })
                    .collect(),
            )
        }
        Instrument::Clap { decay, tone } => {
            let n = secs(decay * 1.4 + 0.033);
            let mut bp = Svf::new(*tone, 1.4, srf);
            mono(
                (0..n)
                    .map(|i| {
                        let t = i as f32 / srf;
                        // Three hands landing 11 ms apart, then the room.
                        let env = if t < 0.033 { (-(t % 0.011) * 260.0).exp() } else { (-6.0 * (t - 0.033) / decay).exp() };
                        bp.tick(rng.next()).1 * bp.k * 1.6 * env * vel
                    })
                    .collect(),
            )
        }
        Instrument::Tom { decay, sweep } => {
            let n = secs(decay * 1.6);
            let base = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
            let mut ph = 0.0f32;
            let fade = secs(0.004).max(1);
            mono(
                (0..n)
                    .map(|i| {
                        let t = i as f32 / srf;
                        ph += TAU * base * (1.0 + (sweep - 1.0) * (-t * 25.0).exp()) / srf;
                        let amp = (-5.0 * t / decay).exp() * (1.0 - (-t * 2000.0).exp());
                        let tail = ((n - i) as f32 / fade as f32).min(1.0);
                        saturate(ph.sin() * amp, 0.6) * tail * vel
                    })
                    .collect(),
            )
        }
        Instrument::Sweep { from, to, q } => {
            let rising = to > from;
            let rel = secs(0.03).max(1);
            let n = gate_samples + if rising { rel } else { 0 };
            let mut bp = Svf::new(*from, *q, srf);
            let gate = gate_samples.max(1) as f32;
            mono(
                (0..n)
                    .map(|i| {
                        let x = (i as f32 / gate).min(1.0);
                        if i % 32 == 0 {
                            bp.set(from * (to / from).powf(x), *q, srf);
                        }
                        let env = if rising {
                            x * x * if i >= gate_samples { 1.0 - (i - gate_samples) as f32 / rel as f32 } else { 1.0 }
                        } else {
                            (1.0 - x) * (1.0 - x) * (i as f32 / (0.005 * srf)).min(1.0)
                        };
                        bp.tick(rng.next()).1 * bp.k * 2.0 * env * vel
                    })
                    .collect(),
            )
        }
    }
}

/// A single hit of `inst` as a mono buffer: lets the game synthesise one-shot sounds (its spawn kick)
/// with the same drum models the songs use.
pub fn one_shot(inst: &Instrument, note: &str, gate_seconds: f32, velocity: f32, sample_rate: u32) -> Vec<f32> {
    let mut rng = Rng::new(1, 2, 3, 4);
    let gate = ((gate_seconds * sample_rate as f32) as usize).max(1);
    voice(inst, parse_note(note).unwrap_or(60), gate, velocity, sample_rate, &mut rng).iter().map(|f| (f[0] + f[1]) * 0.5).collect()
}

/// Run `f` over `buf` in place; a loop is first run over its last `warm` frames (output discarded) so
/// the effect's state at sample 0 is what it would be after a previous pass.
fn run_fx(buf: &mut [Frame], looped: bool, warm: usize, mut f: impl FnMut(usize, Frame) -> Frame) {
    let n = buf.len();
    if looped {
        for i in n - warm.min(n)..n {
            f(i, buf[i]);
        }
    }
    for i in 0..n {
        buf[i] = f(i, buf[i]);
    }
}

fn highpass(buf: &mut [Frame], fc: f32, sr: u32, looped: bool) {
    let mut f = [Svf::new(fc, 0.707, sr as f32); 2];
    run_fx(buf, looped, 8192, |_, x| [f[0].tick(x[0]).2, f[1].tick(x[1]).2]);
}

/// Stereo chorus: one modulated tap per side, a quarter cycle apart. In a loop the LFO completes a whole
/// number of cycles and the tap reads around the loop point.
fn chorus(buf: &mut [Frame], depth: f32, sr: u32, looped: bool) {
    let n = buf.len();
    let dry = buf.to_vec();
    let secs = n as f32 / sr as f32;
    let cycles = (0.55 * secs).round().max(1.0);
    let (centre, swing) = (0.014 * sr as f32, 0.005 * sr as f32);
    for (i, o) in buf.iter_mut().enumerate() {
        let ph = cycles * i as f32 / n as f32;
        for (c, off) in [0.0f32, 0.25].into_iter().enumerate() {
            let d = centre + swing * (TAU * (ph + off)).sin();
            let p = i as f32 - d;
            let (j, t) = (p.floor(), p - p.floor());
            let at = |k: f32| -> f32 {
                let k = k as i64;
                if looped {
                    dry[k.rem_euclid(n as i64) as usize][c]
                } else if k >= 0 && (k as usize) < n {
                    dry[k as usize][c]
                } else {
                    0.0
                }
            };
            o[c] = dry[i][c] * (1.0 - 0.3 * depth) + (at(j) + (at(j + 1.0) - at(j)) * t) * depth * 0.8;
        }
    }
}

/// Sidechain-style pumping: the gain dips by `depth` at each trigger and recovers exponentially.
fn duck(buf: &mut [Frame], d: Duck, triggers: &[usize], step: usize, sr: u32, looped: bool) {
    let Some(&last) = triggers.last() else { return };
    let n = buf.len();
    let k = (-1.0 / (d.release * step as f32)).exp();
    let ramp = (0.004 * sr as f32).max(1.0);
    // `e` is the current trigger's dip (1 at the trigger), `old` the one it replaced, cross-faded over
    // `ramp` so the gain never jumps.
    let (mut e, mut old, mut since) = (0.0f32, 0.0f32, ramp);
    if looped {
        e = k.powf((n - last) as f32);
    }
    let mut next = 0usize;
    for (i, o) in buf.iter_mut().enumerate() {
        if next < triggers.len() && triggers[next] == i {
            old = e * (since / ramp).min(1.0) + old * (1.0 - (since / ramp).min(1.0));
            e = 1.0;
            since = 0.0;
            next += 1;
        }
        let x = (since / ramp).min(1.0);
        let g = 1.0 - d.depth * (e * x + old * (1.0 - x));
        o[0] *= g;
        o[1] *= g;
        e *= k;
        old *= k;
        since += 1.0;
    }
}

/// Feedback echo. A loop runs twice so the repeats of one pass ring into the start of the next.
fn delay(buf: &mut [Frame], d: Delay, step: usize, looped: bool) {
    let ds = d.steps as usize * step;
    let mut line = vec![[0.0f32; 2]; ds];
    let mut lp = [0.0f32; 2];
    let mut c = 0usize;
    let n = buf.len();
    let passes = if looped { 2 } else { 1 };
    let mut wet = vec![[0.0f32; 2]; n];
    for pass in 0..passes {
        for i in 0..n {
            let x = buf[i];
            let mut y = line[c];
            for ch in 0..2 {
                lp[ch] += (1.0 - d.damp) * (y[ch] - lp[ch]);
                y[ch] = lp[ch];
            }
            line[c] = if d.pingpong { [(x[0] + x[1]) * 0.5 + y[1] * d.feedback, y[0] * d.feedback] } else { [x[0] + y[0] * d.feedback, x[1] + y[1] * d.feedback] };
            if pass + 1 == passes {
                wet[i] = y;
            }
            c = (c + 1) % ds;
        }
    }
    for (o, w) in buf.iter_mut().zip(&wet) {
        o[0] += w[0] * d.mix;
        o[1] += w[1] * d.mix;
    }
}

/// The song's reverb: six damped combs and three allpasses per side (Freeverb layout, the right side a
/// few samples longer), fed with the mono sum of the send bus. Returns the wet signal only.
fn reverb(send: &[Frame], cfg: Reverb, sr: u32, looped: bool) -> Vec<Frame> {
    const COMBS: [usize; 6] = [1116, 1188, 1277, 1356, 1422, 1491];
    const ALLPASS: [usize; 3] = [556, 441, 341];
    const SPREAD: usize = 23;
    struct Comb {
        buf: Vec<f32>,
        i: usize,
        fb: f32,
        lp: f32,
    }
    struct Side {
        combs: Vec<Comb>,
        aps: Vec<(Vec<f32>, usize)>,
        hp: Svf,
    }
    let scale = sr as f32 / 44_100.0;
    let side = |extra: usize| Side {
        combs: COMBS
            .iter()
            .map(|&l| {
                let len = ((l + extra) as f32 * scale) as usize;
                Comb { buf: vec![0.0; len], i: 0, fb: 10f32.powf(-3.0 * len as f32 / (sr as f32 * cfg.decay)), lp: 0.0 }
            })
            .collect(),
        aps: ALLPASS.iter().map(|&l| (vec![0.0; ((l + extra) as f32 * scale) as usize], 0)).collect(),
        hp: Svf::new(220.0, 0.707, sr as f32),
    };
    let mut sides = [side(0), side(SPREAD)];
    // Unity-ish wet level for a send of 1: undo the combs' resonant gain.
    let fb = sides[0].combs[2].fb;
    let gain = ((1.0 - fb * fb) / COMBS.len() as f32).sqrt();
    let damp = cfg.damp;
    let mut tick = |x: Frame| -> Frame {
        let input = (x[0] + x[1]) * 0.5 * gain;
        let mut out = [0.0f32; 2];
        for (c, s) in sides.iter_mut().enumerate() {
            let mut acc = 0.0;
            for cb in &mut s.combs {
                let y = cb.buf[cb.i];
                cb.lp = y * (1.0 - damp) + cb.lp * damp;
                cb.buf[cb.i] = input + cb.lp * cb.fb;
                cb.i = (cb.i + 1) % cb.buf.len();
                acc += y;
            }
            for (buf, i) in &mut s.aps {
                let b = buf[*i];
                buf[*i] = acc + b * 0.5;
                *i = (*i + 1) % buf.len();
                acc = b - acc;
            }
            out[c] = s.hp.tick(acc).2;
        }
        out
    };
    let n = send.len();
    if looped {
        let warm = ((cfg.decay * 1.2 * sr as f32) as usize).min(n);
        for x in &send[n - warm..] {
            tick(*x);
        }
    }
    send.iter().map(|x| tick(*x)).collect()
}

/// Sample positions of the stem's kick hits (what `duck.every == 0` follows).
fn kick_triggers(song: &Song, stem: &Stem, step: usize) -> Vec<usize> {
    let mut v: Vec<usize> = stem
        .tracks
        .iter()
        .filter(|t| matches!(song.instruments[&t.instrument], Instrument::Kick { .. }))
        .flat_map(|t| t.events.iter().map(|e| e.step() as usize * step))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// One track through its effects, at its gain and pan.
fn render_track(song: &Song, kind: StemKind, stem: &Stem, ti: usize, t: &Track, n: usize, looped: bool) -> Vec<Frame> {
    let sr = song.sample_rate;
    let step = samples_per_step(sr);
    let inst = &song.instruments[&t.instrument];
    let mut buf = vec![[0.0f32; 2]; n];
    // Identical notes are synthesised once per track.
    let mut cache: HashMap<(i32, usize, u32), Vec<Frame>> = HashMap::new();
    for e in &t.events {
        let midi = parse_note(e.note()).unwrap_or(60);
        let gate = ((e.len_steps() * step as f32) as usize).max(1);
        let key = (if matches!(inst, Instrument::Synth { .. } | Instrument::Tom { .. }) { midi } else { 0 }, gate, e.velocity().to_bits());
        let v = cache.entry(key).or_insert_with(|| {
            let mut rng = Rng::new(song.seed, kind.seed_id(), ti as u64, (key.0 as u64) << 40 ^ (gate as u64) << 8 ^ key.2 as u64);
            voice(inst, midi, gate, e.velocity(), sr, &mut rng)
        });
        let pos = e.step() as usize * step;
        for (i, s) in v.iter().enumerate() {
            let idx = pos + i;
            let o = if looped {
                &mut buf[idx % n]
            } else if idx < n {
                &mut buf[idx]
            } else {
                break;
            };
            o[0] += s[0];
            o[1] += s[1];
        }
    }
    if t.hp > 0.0 {
        highpass(&mut buf, t.hp, sr, looped);
    }
    if t.chorus > 0.0 {
        chorus(&mut buf, t.chorus, sr, looped);
    }
    if let Some(d) = t.duck {
        let triggers: Vec<usize> = if d.every > 0 { (0..n / step).step_by(d.every as usize).map(|s| s * step).collect() } else { kick_triggers(song, stem, step) };
        duck(&mut buf, d, &triggers, step, sr, looped);
    }
    if let Some(d) = t.delay {
        delay(&mut buf, d, step, looped);
    }
    // Constant-power pan (a balance control for stereo material).
    let ang = (t.pan + 1.0) * std::f32::consts::FRAC_PI_4;
    let (gl, gr) = (ang.cos() * t.gain, ang.sin() * t.gain);
    for o in &mut buf {
        o[0] *= gl;
        o[1] *= gr;
    }
    buf
}

pub fn render_stem(song: &Song, kind: StemKind, stem: &Stem) -> RenderedStem {
    let sr = song.sample_rate;
    let n = (stem.bars * FRAMES_PER_BAR * (sr / FPS)) as usize;
    debug_assert_eq!(n, stem.bars as usize * STEPS_PER_BAR as usize * samples_per_step(sr));
    let looped = kind.is_loop();
    let mut bus = vec![[0.0f32; 2]; n];
    let mut send: Option<Vec<Frame>> = None;
    for (ti, t) in stem.tracks.iter().enumerate() {
        let track = render_track(song, kind, stem, ti, t, n, looped);
        for (o, s) in bus.iter_mut().zip(&track) {
            o[0] += s[0];
            o[1] += s[1];
        }
        if t.reverb > 0.0 {
            let send = send.get_or_insert_with(|| vec![[0.0f32; 2]; n]);
            for (o, s) in send.iter_mut().zip(&track) {
                o[0] += s[0] * t.reverb;
                o[1] += s[1] * t.reverb;
            }
        }
    }
    if let Some(send) = send {
        for (o, w) in bus.iter_mut().zip(reverb(&send, song.reverb, sr, looped)) {
            o[0] += w[0];
            o[1] += w[1];
        }
    }
    let mut samples = Vec::with_capacity(n * 2);
    for f in &bus {
        samples.push((f[0] * song.master).tanh());
        samples.push((f[1] * song.master).tanh());
    }
    if !looped {
        // Cut tails fade over 10 ms so one-shots never click at the end.
        let fade = (sr / 100) as usize;
        for i in 0..fade.min(n) {
            let g = i as f32 / fade as f32;
            samples[2 * (n - 1 - i)] *= g;
            samples[2 * (n - 1 - i) + 1] *= g;
        }
    }
    RenderedStem { kind, sample_rate: sr, samples, looped }
}
