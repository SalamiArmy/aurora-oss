use au2_music::format::parse_note;
use au2_music::{render_song, Song, StemKind};

const DAWN: &str = include_str!("../songs/dawn.song.json");

fn dawn() -> Song {
    Song::from_json(DAWN).expect("dawn validates")
}

#[test]
fn notes_parse() {
    assert_eq!(parse_note("A4"), Some(69));
    assert_eq!(parse_note("C#4"), Some(61));
    assert_eq!(parse_note("Bb2"), Some(46));
    assert_eq!(parse_note("H4"), None);
    assert_eq!(parse_note("A"), None);
}

#[test]
fn stems_are_frame_exact() {
    let stems = render_song(&dawn()).unwrap();
    assert_eq!(stems.len(), 10, "intro, two main loops, three pads, four fills");
    for s in &stems {
        // Whole bars of 128 game frames, and an exact whole number of samples per frame.
        assert_eq!(s.game_frames() % 128, 0, "{:?}", s.kind);
        assert_eq!(s.frames(), s.game_frames() as usize * 735, "{:?}", s.kind);
    }
    assert_eq!(stems[1].game_frames(), 1024, "8 bars = 17.0667 s");
}

#[test]
fn rendering_is_deterministic_and_bounded() {
    let a = render_song(&dawn()).unwrap();
    let b = render_song(&dawn()).unwrap();
    assert_eq!(a, b);
    for s in &a {
        assert!(s.samples.iter().any(|v| v.abs() > 0.01), "{:?} is silent", s.kind);
        assert!(s.samples.iter().all(|v| v.abs() <= 1.0), "{:?} exceeds full scale", s.kind);
    }
}

#[test]
fn loop_tails_wrap_but_one_shots_end_silent() {
    // One long bell on the last step of a one-bar stem: a loop stem wraps its tail onto the start.
    let mut song = dawn();
    for st in [&mut song.stems.beat, &mut song.stems.beathigh] {
        st.bars = 1;
        st.tracks.truncate(1);
        st.tracks[0].instrument = "bell".into();
        st.tracks[0].delay = None;
        st.tracks[0].events = vec![au2_music::format::Event(15, "A4".into(), 1.0, 1.0)];
    }
    song.stems.intro = song.stems.beat.clone();
    let r = render_song(&song).unwrap();
    let beat = &r[1];
    let head: f32 = beat.samples[..2000].iter().map(|v| v.abs()).sum();
    assert!(head > 1.0, "loop tail should wrap to the start");
    let intro = &r[0];
    let head: f32 = intro.samples[..2000].iter().map(|v| v.abs()).sum();
    assert_eq!(head, 0.0, "one-shot tail must not wrap");
    assert_eq!(*intro.samples.last().unwrap(), 0.0, "one-shot ends faded to zero");
}

#[test]
fn invalid_songs_are_rejected() {
    let mut s = dawn();
    s.stems.beathigh.bars = 4;
    assert!(s.validate().unwrap_err().0.contains("same number of bars"));

    let mut s = dawn();
    s.stems.beat.tracks[0].instrument = "nope".into();
    assert!(s.validate().unwrap_err().0.contains("unknown instrument"));

    let mut s = dawn();
    s.stems.beat.tracks[0].events[0].0 = 9999;
    assert!(s.validate().unwrap_err().0.contains("outside"));

    let mut s = dawn();
    s.stems.beat.tracks[0].events[0].1 = "X9".into();
    assert!(s.validate().unwrap_err().0.contains("bad note"));

    let mut s = dawn();
    s.format = "aurora-song/2".into();
    assert!(s.validate().is_err());
}

#[test]
fn json_round_trips() {
    let s = dawn();
    let again: Song = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, again);
}

const SONGS: [&str; 8] = [
    DAWN,
    include_str!("../songs/tide.song.json"),
    include_str!("../songs/ember.song.json"),
    include_str!("../songs/lumen.song.json"),
    include_str!("../songs/vesper.song.json"),
    include_str!("../songs/halcyon.song.json"),
    include_str!("../songs/nadir.song.json"),
    include_str!("../songs/zenith.song.json"),
];

/// Units spawn on bar lines, half bars or beats depending on game speed, and the spawn points pulse
/// there: both main loops of every song must hit a kick on every beat, hardest on the bar line.
#[test]
fn every_song_has_a_kick_on_every_beat_and_the_hardest_on_the_bar() {
    use au2_music::format::Instrument;
    for json in SONGS {
        let song = Song::from_json(json).unwrap();
        for (label, stem) in [("beat", &song.stems.beat), ("beathigh", &song.stems.beathigh)] {
            let kicks: Vec<_> = stem.tracks.iter().filter(|t| matches!(song.instruments[&t.instrument], Instrument::Kick { .. })).flat_map(|t| &t.events).collect();
            for bar in 0..stem.bars {
                let vel = |step: u32| kicks.iter().find(|e| e.step() == bar * 16 + step).map(|e| e.velocity());
                let down = vel(0).unwrap_or_else(|| panic!("{} {label}: no kick on bar {bar}", song.name));
                for beat in [4, 8, 12] {
                    let v = vel(beat).unwrap_or_else(|| panic!("{} {label}: no kick on bar {bar} step {beat}", song.name));
                    assert!(v <= down, "{} {label} bar {bar}: beat {beat} ({v}) is louder than the downbeat ({down})", song.name);
                }
            }
        }
    }
}

/// Low band (one-pole low-pass at 150 Hz) energy of the mono sum over `[from, to)` sample frames.
fn low_energy(s: &au2_music::RenderedStem, from: usize, to: usize) -> f32 {
    let k = (-std::f32::consts::TAU * 150.0 / s.sample_rate as f32).exp();
    let mut lp = 0.0f32;
    let mut e = 0.0;
    // Start a little early so the filter has settled by `from`.
    for i in from.saturating_sub(2000)..to {
        lp = lp * k + (s.samples[2 * i] + s.samples[2 * i + 1]) * 0.5 * (1.0 - k);
        if i >= from {
            e += lp * lp;
        }
    }
    e
}

/// The same thing measured in the audio: in every song's main loops the low end jumps on each beat, on
/// the exact sample the game's frame grid puts it (32 frames = 23 520 samples at 44.1 kHz).
#[test]
fn the_rendered_beat_lands_on_the_frame_grid() {
    for json in SONGS {
        let song = Song::from_json(json).unwrap();
        for kind in [au2_music::StemKind::Beat, au2_music::StemKind::BeatHigh] {
            let stem = if kind == au2_music::StemKind::Beat { &song.stems.beat } else { &song.stems.beathigh };
            let r = au2_music::render_stem(&song, kind, stem);
            let beat = 32 * 735;
            let win = 1764; // 40 ms
            let (mut after, mut before) = (0.0, 0.0);
            for b in 1..stem.bars as usize * 4 {
                after += low_energy(&r, b * beat, b * beat + win);
                before += low_energy(&r, b * beat - win, b * beat);
            }
            assert!(after > 4.0 * before, "{} {kind:?}: the beat does not stand out (after {after:.3}, before {before:.3})", song.name);
        }
    }
}

#[test]
fn main_loops_are_stereo_and_leave_headroom() {
    for json in SONGS {
        let song = Song::from_json(json).unwrap();
        let r = au2_music::render_stem(&song, au2_music::StemKind::BeatHigh, &song.stems.beathigh);
        let (mut mid, mut side, mut peak) = (0.0f32, 0.0f32, 0.0f32);
        for f in r.samples.chunks_exact(2) {
            mid += (f[0] + f[1]).powi(2);
            side += (f[0] - f[1]).powi(2);
            peak = peak.max(f[0].abs()).max(f[1].abs());
        }
        assert!((side / mid).sqrt() > 0.25, "{}: too narrow ({:.2})", song.name, (side / mid).sqrt());
        assert!(peak < 0.75, "{}: peak {peak:.2} leaves no room for the pads and sound effects", song.name);
        let rms = (mid / 4.0 / r.frames() as f32).sqrt();
        assert!((0.04..0.12).contains(&rms), "{}: main loop RMS {rms:.3}", song.name);
    }
}

fn one_track_song(mut edit: impl FnMut(&mut au2_music::format::Track)) -> au2_music::RenderedStem {
    let mut song = dawn();
    let st = &mut song.stems.pad_normal;
    st.bars = 1;
    st.tracks.truncate(1);
    let t = &mut st.tracks[0];
    t.instrument = "bell".into();
    (t.delay, t.duck, t.reverb, t.chorus, t.hp) = (None, None, 0.0, 0.0, 0.0);
    t.events = vec![au2_music::format::Event(15, "A4".into(), 1.0, 1.0)];
    edit(t);
    au2_music::render_stem(&song, au2_music::StemKind::PadNormal, &song.stems.pad_normal)
}

/// Mean absolute level over `from..to` seconds.
fn level(s: &au2_music::RenderedStem, from: f32, to: f32) -> f32 {
    let at = |t: f32| (t * s.sample_rate as f32) as usize * 2;
    let part = &s.samples[at(from)..at(to)];
    part.iter().map(|v| v.abs()).sum::<f32>() / part.len() as f32
}

#[test]
fn reverb_and_echo_tails_wrap_around_the_loop() {
    // A note on the last 16th of a one-bar loop: 1.2 s into the loop its own release is over...
    let dry = one_track_song(|_| {});
    // ...but its reverb tail and its echoes are still sounding there.
    let wet = one_track_song(|t| t.reverb = 1.0);
    let echo = one_track_song(|t| t.delay = Some(au2_music::format::Delay { steps: 3, feedback: 0.6, mix: 0.6, pingpong: true, damp: 0.2 }));
    let d = level(&dry, 1.2, 1.6);
    assert!(level(&wet, 1.2, 1.6) > 5.0 * d + 1e-4, "no reverb tail after the loop point");
    assert!(level(&echo, 1.2, 1.6) > 5.0 * d + 1e-4, "no echo after the loop point");
}

#[test]
fn ducking_dips_on_the_beat_and_recovers() {
    // A held pad chord, pumping on every beat.
    let held = |duck: Option<au2_music::format::Duck>| {
        one_track_song(|t| {
            t.instrument = "pad".into();
            t.events = vec![au2_music::format::Event(0, "A3".into(), 16.0, 1.0)];
            t.duck = duck;
        })
    };
    let flat = held(None);
    let pumped = held(Some(au2_music::format::Duck { depth: 0.8, release: 1.0, every: 4 }));
    let beat = 32.0 / 60.0;
    // Just after the third beat the pumped pad is far below the plain one; just before the fourth it is back.
    let dip = level(&pumped, 2.0 * beat + 0.01, 2.0 * beat + 0.04) / level(&flat, 2.0 * beat + 0.01, 2.0 * beat + 0.04);
    let back = level(&pumped, 3.0 * beat - 0.05, 3.0 * beat - 0.01) / level(&flat, 3.0 * beat - 0.05, 3.0 * beat - 0.01);
    assert!(dip < 0.45, "gain right after the beat: {dip:.2}");
    assert!(back > 0.9, "gain before the next beat: {back:.2}");
}

#[test]
fn old_songs_without_the_new_fields_still_load() {
    let json = r#"{"format":"aurora-song/1","name":"old","instruments":{
        "pad":{"type":"synth","osc":[{"wave":"saw"}],"attack":0.1,"decay":0.5,"sustain":0.7,"release":0.3,"lowpass":{"cutoff":800}},
        "kick":{"type":"kick"},"hat":{"type":"hat"},"snare":{"type":"snare"}},
        "stems":{"intro":{"bars":1,"tracks":[{"instrument":"kick","events":[[0,"C2",1,1]]}]},
        "beat":{"bars":1,"tracks":[{"instrument":"pad","delay":{"steps":3,"feedback":0.5,"mix":0.4},"events":[[0,"A3",4,0.8]]}]},
        "beathigh":{"bars":1,"tracks":[{"instrument":"hat","events":[[2,"C2",1,0.5]]}]},
        "pad_normal":{"bars":1,"tracks":[{"instrument":"pad","events":[[0,"A3",8,0.5]]}]},
        "pad_winning":{"bars":1,"tracks":[{"instrument":"pad","events":[[0,"C4",8,0.5]]}]},
        "pad_losing":{"bars":1,"tracks":[{"instrument":"pad","events":[[0,"A2",8,0.5]]}]},
        "fills":[{"bars":1,"tracks":[{"instrument":"snare","events":[[0,"C2",1,0.5]]}]}]}}"#;
    let stems = render_song(&Song::from_json(json).unwrap()).unwrap();
    assert_eq!(stems.len(), 7);
    assert!(stems.iter().all(|s| s.samples.iter().any(|v| v.abs() > 0.001)));
}

/// The title piece (`menu.song.json`, written by `menugen.py`) carries a `menu` stem; the other stems are empty
/// placeholders. It loops on the bar grid like every loop stem.
#[test]
fn the_menu_song_renders_a_long_loop_menu_stem() {
    let song = Song::from_json(include_str!("../songs/menu.song.json")).expect("menu song validates");
    let stems = render_song(&song).expect("renders");
    let menu = stems.iter().find(|s| s.kind == StemKind::Menu).expect("menu stem");
    assert!(menu.looped);
    assert_eq!(menu.game_frames(), 32 * 128);
    assert!(menu.seconds() > 60.0, "drawn out: {} s", menu.seconds());
    assert_eq!(StemKind::Menu.file_suffix(), "menu");
}
