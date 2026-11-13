use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub const FORMAT_ID: &str = "aurora-song/1";

fn one() -> f32 {
    1.0
}
fn default_rate() -> u32 {
    44_100
}
fn default_master() -> f32 {
    0.8
}

/// A whole song: shared instruments and the stems the music system layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    /// Must be [`FORMAT_ID`].
    pub format: String,
    pub name: String,
    /// 44100 or 48000 (both give a whole number of samples per game frame).
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Seeds every noise source; same song + seed renders bit-identically.
    #[serde(default)]
    pub seed: u64,
    /// Output gain applied before the final soft clip.
    #[serde(default = "default_master")]
    pub master: f32,
    /// The shared reverb every track can send to (`Track::reverb`).
    #[serde(default)]
    pub reverb: Reverb,
    pub instruments: BTreeMap<String, Instrument>,
    pub stems: Stems,
}

/// The song's one reverb (a stereo Schroeder/Freeverb tank), fed by each track's `reverb` send.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Reverb {
    /// Seconds for the tail to fall 60 dB (0.2..8).
    #[serde(default = "reverb_decay")]
    pub decay: f32,
    /// High-frequency damping of the tail, 0 (bright) .. 0.95 (dark).
    #[serde(default = "reverb_damp")]
    pub damp: f32,
}
fn reverb_decay() -> f32 {
    2.2
}
fn reverb_damp() -> f32 {
    0.35
}
impl Default for Reverb {
    fn default() -> Self {
        Reverb { decay: reverb_decay(), damp: reverb_damp() }
    }
}

/// The layers of one song, (intro, beat, beathigh, pads, fills).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stems {
    /// Plays once at the start of a game, then the music queues `beat`.
    pub intro: Stem,
    /// Main loop (calm). Same length as `beathigh`.
    pub beat: Stem,
    /// Main loop (intense); alternates with `beat` on the loop boundary.
    pub beathigh: Stem,
    /// Secondary loops layered over the main one by battle status.
    pub pad_normal: Stem,
    pub pad_winning: Stem,
    pub pad_losing: Stem,
    /// One-shots dropped over the music now and then (1 to 4).
    pub fills: Vec<Stem>,
    /// Title / main-menu music: one long free-running loop, not tied to the sim clock or the beat grid
    /// (a title-screen track is 150-230 s long). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Stem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemKind {
    Intro,
    Beat,
    BeatHigh,
    PadNormal,
    PadWinning,
    PadLosing,
    Fill(usize),
    Menu,
}

impl StemKind {
    /// Output file stem, named `mx_<song>_<this>`.
    pub fn file_suffix(self) -> String {
        match self {
            StemKind::Intro => "intro".into(),
            StemKind::Beat => "beat".into(),
            StemKind::BeatHigh => "beathigh".into(),
            StemKind::PadNormal => "pad_normal".into(),
            StemKind::PadWinning => "pad_winning".into(),
            StemKind::PadLosing => "pad_losing".into(),
            StemKind::Fill(i) => format!("fill{}", i + 1),
            StemKind::Menu => "menu".into(),
        }
    }
    /// Loops seamlessly (tails wrap around); intro and fills are one-shots (tails are cut).
    pub fn is_loop(self) -> bool {
        !matches!(self, StemKind::Intro | StemKind::Fill(_))
    }
    fn index(self) -> u64 {
        match self {
            StemKind::Intro => 0,
            StemKind::Beat => 1,
            StemKind::BeatHigh => 2,
            StemKind::PadNormal => 3,
            StemKind::PadWinning => 4,
            StemKind::PadLosing => 5,
            StemKind::Fill(i) => 6 + i as u64,
            StemKind::Menu => 10,
        }
    }
    /// Stable id mixed into the noise seed.
    pub fn seed_id(self) -> u64 {
        self.index()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stem {
    /// Length in bars (1 bar = 128 game frames = 16 steps). 1..=64.
    pub bars: u32,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Key into `Song::instruments`.
    pub instrument: String,
    #[serde(default = "one")]
    pub gain: f32,
    /// -1 (left) .. 1 (right).
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub delay: Option<Delay>,
    /// Send level into the song reverb, 0..1.
    #[serde(default)]
    pub reverb: f32,
    /// Stereo chorus depth, 0..1.
    #[serde(default)]
    pub chorus: f32,
    /// High-pass cutoff in Hz (0 = off); keeps pads and leads out of the kick's way.
    #[serde(default)]
    pub hp: f32,
    /// Sidechain-style gain dip on the beat, so the track pumps around the kick.
    #[serde(default)]
    pub duck: Option<Duck>,
    pub events: Vec<Event>,
}

/// A gain dip that recovers exponentially: the "pumping" of a sidechained track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Duck {
    /// How far the gain drops at the trigger, 0..1.
    pub depth: f32,
    /// Recovery time constant in 16th steps (0.25..16).
    #[serde(default = "duck_release")]
    pub release: f32,
    /// Trigger every this many steps from the stem start (4 = every beat). 0 = on this stem's kick events.
    #[serde(default)]
    pub every: u32,
}
fn duck_release() -> f32 {
    1.5
}

/// Feedback echo on a track (circular in loop stems, so it wraps seamlessly).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Delay {
    /// Echo spacing in 16th steps (1..=16).
    pub steps: u32,
    /// 0..0.95
    pub feedback: f32,
    /// Wet level, 0..1.
    pub mix: f32,
    /// Echoes alternate left / right.
    #[serde(default)]
    pub pingpong: bool,
    /// Each repeat loses this much treble, 0..0.95.
    #[serde(default)]
    pub damp: f32,
}

/// `[step, "A3", length_in_steps, velocity]`. Drums ignore the pitch; length chokes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event(pub u32, pub String, pub f32, pub f32);

impl Event {
    pub fn step(&self) -> u32 {
        self.0
    }
    pub fn note(&self) -> &str {
        &self.1
    }
    pub fn len_steps(&self) -> f32 {
        self.2
    }
    pub fn velocity(&self) -> f32 {
        self.3
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wave {
    Sine,
    Saw,
    Square,
    Tri,
    Noise,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Osc {
    pub wave: Wave,
    #[serde(default = "one")]
    pub level: f32,
    /// Octave shift.
    #[serde(default)]
    pub octave: i32,
    /// Fine tune in cents.
    #[serde(default)]
    pub detune: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lowpass {
    /// Base cutoff in Hz.
    pub cutoff: f32,
    /// Extra cutoff in Hz at note start, decaying with the amplitude envelope (or over `decay`).
    #[serde(default)]
    pub env: f32,
    /// Resonance (0.5 = none, 0.707 = flat, up to 8 = ringing).
    #[serde(default = "lp_q")]
    pub q: f32,
    /// Seconds for `env` to die away on its own; 0 = follow the amplitude envelope.
    #[serde(default)]
    pub decay: f32,
    /// Key tracking 0..1: how much the cutoff follows the note (relative to A3).
    #[serde(default)]
    pub track: f32,
}
fn lp_q() -> f32 {
    0.707
}

/// Phase modulation of every oscillator by one sine operator (bells, electric pianos, growls).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fm {
    /// Modulator frequency / note frequency.
    pub ratio: f32,
    /// Modulation depth in radians at note start.
    pub index: f32,
    /// Seconds for the depth to fall to 1/e; 0 = constant.
    #[serde(default)]
    pub decay: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vibrato {
    /// Hz.
    pub rate: f32,
    /// Cents.
    pub depth: f32,
    /// Seconds before it reaches full depth.
    #[serde(default)]
    pub delay: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Instrument {
    /// Subtractive / FM synth: oscillators (optionally a detuned stereo unison stack), ADSR in seconds,
    /// resonant lowpass with its own envelope, vibrato, saturation.
    Synth {
        osc: Vec<Osc>,
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        #[serde(default)]
        lowpass: Option<Lowpass>,
        /// Copies of the oscillator stack (1..=7), detuned across `spread` and panned across `width`.
        #[serde(default = "one_u")]
        unison: u32,
        /// Outermost unison detune in cents.
        #[serde(default)]
        spread: f32,
        /// Stereo width of the unison stack, 0..1.
        #[serde(default)]
        width: f32,
        #[serde(default)]
        fm: Option<Fm>,
        #[serde(default)]
        vibrato: Option<Vibrato>,
        /// Soft saturation amount, 0..8.
        #[serde(default)]
        drive: f32,
    },
    /// Sine sweeping from `start` to `end` Hz, with a beater click and saturation so it reads on small speakers.
    Kick {
        #[serde(default = "kick_start")]
        start: f32,
        #[serde(default = "kick_end")]
        end: f32,
        #[serde(default = "kick_decay")]
        decay: f32,
        /// Pitch sweep rate (1/s).
        #[serde(default = "kick_sweep")]
        sweep: f32,
        /// Level of the attack click, 0..1.
        #[serde(default = "kick_click")]
        click: f32,
        /// Saturation, 0..8 (adds the harmonics a phone speaker can reproduce).
        #[serde(default = "kick_drive")]
        drive: f32,
    },
    /// Two-partial drum tone plus band-limited noise.
    Snare {
        #[serde(default = "snare_tone")]
        tone: f32,
        #[serde(default = "snare_decay")]
        decay: f32,
        /// Noise share, 0..1.
        #[serde(default = "half")]
        noise: f32,
    },
    /// High-passed noise: closed hat (short decay), open hat / crash (long), shaker (with `attack`).
    Hat {
        #[serde(default = "hat_decay")]
        decay: f32,
        /// High-pass cutoff in Hz.
        #[serde(default = "hat_tone")]
        tone: f32,
        /// Fade-in in seconds (a shaker's soft front).
        #[serde(default)]
        attack: f32,
    },
    /// Hand clap: three quick noise bursts and a tail, band-passed around `tone`.
    Clap {
        #[serde(default = "clap_decay")]
        decay: f32,
        #[serde(default = "clap_tone")]
        tone: f32,
    },
    /// Pitched drum: a sine at the note's pitch, falling from `sweep` times that pitch.
    Tom {
        #[serde(default = "tom_decay")]
        decay: f32,
        #[serde(default = "tom_sweep")]
        sweep: f32,
    },
    /// Noise through a band-pass gliding `from` -> `to` Hz over the note: a riser (swelling in) when
    /// `to > from`, a falling wash (fading out) otherwise.
    Sweep {
        from: f32,
        to: f32,
        #[serde(default = "sweep_q")]
        q: f32,
    },
}
fn one_u() -> u32 {
    1
}
fn half() -> f32 {
    0.5
}
fn kick_start() -> f32 {
    150.0
}
fn kick_end() -> f32 {
    48.0
}
fn kick_decay() -> f32 {
    0.28
}
fn kick_sweep() -> f32 {
    32.0
}
fn kick_click() -> f32 {
    0.5
}
fn kick_drive() -> f32 {
    1.5
}
fn snare_tone() -> f32 {
    190.0
}
fn snare_decay() -> f32 {
    0.2
}
fn hat_decay() -> f32 {
    0.06
}
fn hat_tone() -> f32 {
    7000.0
}
fn clap_decay() -> f32 {
    0.2
}
fn clap_tone() -> f32 {
    1300.0
}
fn tom_decay() -> f32 {
    0.3
}
fn tom_sweep() -> f32 {
    1.7
}
fn sweep_q() -> f32 {
    1.5
}

impl Instrument {
    /// Kick, snare, hat, clap, tom or sweep: unpitched (or self-pitched) percussion.
    pub fn is_drum(&self) -> bool {
        !matches!(self, Instrument::Synth { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError(pub String);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ValidationError {}

/// MIDI note number of a name like `A3`, `C#4`, `Bb2` (A4 = 69 = 440 Hz).
pub fn parse_note(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let base = match b.first()?.to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut i = 1;
    let mut semi = base;
    match b.get(i) {
        Some(b'#') => {
            semi += 1;
            i += 1
        }
        Some(b'b') => {
            semi -= 1;
            i += 1
        }
        _ => {}
    }
    let oct: i32 = s.get(i..)?.parse().ok()?;
    if !(-1..=9).contains(&oct) {
        return None;
    }
    Some(12 * (oct + 1) + semi)
}

impl Song {
    pub fn from_json(s: &str) -> Result<Song, Box<dyn std::error::Error>> {
        let song: Song = serde_json::from_str(s)?;
        song.validate()?;
        Ok(song)
    }

    /// All stems in canonical order with their kinds.
    pub fn stem_list(&self) -> Vec<(StemKind, &Stem)> {
        let s = &self.stems;
        let mut v = vec![
            (StemKind::Intro, &s.intro),
            (StemKind::Beat, &s.beat),
            (StemKind::BeatHigh, &s.beathigh),
            (StemKind::PadNormal, &s.pad_normal),
            (StemKind::PadWinning, &s.pad_winning),
            (StemKind::PadLosing, &s.pad_losing),
        ];
        v.extend(s.fills.iter().enumerate().map(|(i, f)| (StemKind::Fill(i), f)));
        if let Some(m) = &s.menu {
            v.push((StemKind::Menu, m));
        }
        v
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        let err = |m: String| Err(ValidationError(m));
        if self.format != FORMAT_ID {
            return err(format!("format must be \"{FORMAT_ID}\", got \"{}\"", self.format));
        }
        if !matches!(self.sample_rate, 44_100 | 48_000) {
            return err("sample_rate must be 44100 or 48000".into());
        }
        if !(0.0..=2.0).contains(&self.master) {
            return err("master must be 0..2".into());
        }
        if self.stems.beat.bars != self.stems.beathigh.bars {
            return err("beat and beathigh must have the same number of bars (they swap on the loop boundary)".into());
        }
        if self.stems.fills.is_empty() || self.stems.fills.len() > 4 {
            return err("fills must hold 1 to 4 stems".into());
        }
        if !(0.2..=8.0).contains(&self.reverb.decay) || !(0.0..=0.95).contains(&self.reverb.damp) {
            return err("reverb: decay must be 0.2..8 s and damp 0..0.95".into());
        }
        for (name, inst) in &self.instruments {
            match inst {
                Instrument::Synth { osc, attack, decay, sustain, release, lowpass, unison, spread, width, fm, vibrato, drive } => {
                    if osc.is_empty() {
                        return err(format!("instrument {name}: needs at least one osc"));
                    }
                    if [*attack, *decay, *release].iter().any(|v| !(0.0..=30.0).contains(v)) || !(0.0..=1.0).contains(sustain) {
                        return err(format!("instrument {name}: bad envelope"));
                    }
                    if lowpass.is_some_and(|l| !(20.0..=20_000.0).contains(&l.cutoff)) {
                        return err(format!("instrument {name}: lowpass cutoff must be 20..20000 Hz"));
                    }
                    if lowpass.is_some_and(|l| !(0.5..=8.0).contains(&l.q) || !(0.0..=30.0).contains(&l.decay) || !(0.0..=1.0).contains(&l.track)) {
                        return err(format!("instrument {name}: lowpass q must be 0.5..8, decay 0..30 s, track 0..1"));
                    }
                    if !(1..=7).contains(unison) || !(0.0..=100.0).contains(spread) || !(0.0..=1.0).contains(width) {
                        return err(format!("instrument {name}: unison must be 1..7, spread 0..100 cents, width 0..1"));
                    }
                    if fm.is_some_and(|f| !(0.0..=32.0).contains(&f.ratio) || !(0.0..=20.0).contains(&f.index) || !(0.0..=30.0).contains(&f.decay)) {
                        return err(format!("instrument {name}: bad fm"));
                    }
                    if vibrato.is_some_and(|v| !(0.0..=20.0).contains(&v.rate) || !(0.0..=200.0).contains(&v.depth) || !(0.0..=10.0).contains(&v.delay)) {
                        return err(format!("instrument {name}: bad vibrato"));
                    }
                    if !(0.0..=8.0).contains(drive) {
                        return err(format!("instrument {name}: drive must be 0..8"));
                    }
                }
                Instrument::Kick { decay, .. } | Instrument::Snare { decay, .. } | Instrument::Hat { decay, .. } | Instrument::Clap { decay, .. } | Instrument::Tom { decay, .. } => {
                    if !(0.005..=5.0).contains(decay) {
                        return err(format!("instrument {name}: decay must be 0.005..5 s"));
                    }
                }
                Instrument::Sweep { from, to, q } => {
                    if [*from, *to].iter().any(|f| !(20.0..=20_000.0).contains(f)) || !(0.5..=8.0).contains(q) {
                        return err(format!("instrument {name}: sweep must stay in 20..20000 Hz with q 0.5..8"));
                    }
                }
            }
        }
        for (kind, stem) in self.stem_list() {
            let label = kind.file_suffix();
            if stem.bars == 0 || stem.bars > 64 {
                return err(format!("{label}: bars must be 1..=64"));
            }
            let steps = stem.bars * crate::STEPS_PER_BAR;
            for (ti, t) in stem.tracks.iter().enumerate() {
                if !self.instruments.contains_key(&t.instrument) {
                    return err(format!("{label} track {ti}: unknown instrument \"{}\"", t.instrument));
                }
                if !(-1.0..=1.0).contains(&t.pan) || !(0.0..=4.0).contains(&t.gain) {
                    return err(format!("{label} track {ti}: pan must be -1..1 and gain 0..4"));
                }
                if let Some(d) = t.delay {
                    if !(1..=16).contains(&d.steps) || !(0.0..=0.95).contains(&d.feedback) || !(0.0..=1.0).contains(&d.mix) || !(0.0..=0.95).contains(&d.damp) {
                        return err(format!("{label} track {ti}: bad delay"));
                    }
                }
                if !(0.0..=1.0).contains(&t.reverb) || !(0.0..=1.0).contains(&t.chorus) || !(0.0..=5000.0).contains(&t.hp) {
                    return err(format!("{label} track {ti}: reverb and chorus must be 0..1, hp 0..5000 Hz"));
                }
                if let Some(d) = t.duck {
                    if !(0.0..=1.0).contains(&d.depth) || !(0.25..=16.0).contains(&d.release) || d.every > 64 {
                        return err(format!("{label} track {ti}: bad duck"));
                    }
                }
                for (ei, e) in t.events.iter().enumerate() {
                    if e.step() >= steps {
                        return err(format!("{label} track {ti} event {ei}: step {} is outside {} steps", e.step(), steps));
                    }
                    if parse_note(e.note()).is_none() {
                        return err(format!("{label} track {ti} event {ei}: bad note \"{}\"", e.note()));
                    }
                    if !(e.len_steps() > 0.0 && e.len_steps() <= steps as f32) || !(0.0..=1.0).contains(&e.velocity()) {
                        return err(format!("{label} track {ti} event {ei}: bad length or velocity"));
                    }
                }
            }
        }
        Ok(())
    }
}
