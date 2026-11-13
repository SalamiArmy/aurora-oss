//! `au2-music check <song.json>` validates a song; `au2-music render <song.json> <out_dir>` writes
//! `mx_<name>_<stem>.wav` for every stem; `au2-music mix <song.json> <out.wav>` writes one audition mix-down of
//! the stems layered as in a game; `au2-music levels <song.json>` prints each track's level (used by
//! `songs/gen_all.py --balance`).
use au2_music::{render_song, wav, Song, StemKind};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args {
        [cmd, path] if cmd == "check" => {
            let song = Song::from_json(&std::fs::read_to_string(path)?)?;
            println!("{}: ok ({} stems)", song.name, song.stem_list().len());
            Ok(())
        }
        [cmd, path, out] if cmd == "render" => {
            let song = Song::from_json(&std::fs::read_to_string(path)?)?;
            std::fs::create_dir_all(out)?;
            for st in render_song(&song)? {
                let file = Path::new(out).join(format!("mx_{}_{}.wav", song.name, st.kind.file_suffix()));
                std::fs::write(&file, wav::to_wav_bytes(&st))?;
                println!("{}  {:.4} s  {} game frames{}", file.display(), st.seconds(), st.game_frames(), if st.looped { "  loop" } else { "" });
            }
            Ok(())
        }
        [cmd, path, out] if cmd == "mix" => {
            // An audition mix-down, layered the way the game plays the stems: the intro, then the main loop
            // alternating beat / beathigh with a pad for each battle status and a fill dropped on each pass.
            let song = Song::from_json(&std::fs::read_to_string(path)?)?;
            let stems = render_song(&song)?;
            let get = |k: StemKind| stems.iter().find(|s| s.kind == k).expect("stem");
            let (intro, beat, high) = (get(StemKind::Intro), get(StemKind::Beat), get(StemKind::BeatHigh));
            let pads = [StemKind::PadNormal, StemKind::PadWinning, StemKind::PadLosing, StemKind::PadNormal];
            let pass = beat.samples.len();
            let mut mix = vec![0.0f32; intro.samples.len() + pass * pads.len()];
            let mut add = |at: usize, src: &[f32], fade_in: bool| {
                for (i, v) in src.iter().enumerate() {
                    if let Some(o) = mix.get_mut(at + i) {
                        *o += v * if fade_in { (i as f32 / 132_300.0).min(1.0) } else { 1.0 };
                    }
                }
            };
            add(0, &intro.samples, false);
            for (k, pad) in pads.into_iter().enumerate() {
                let at = intro.samples.len() + k * pass;
                add(at, &if k % 2 == 0 { beat } else { high }.samples, false);
                add(at, &get(pad).samples, true);
                if let Some(fill) = stems.iter().find(|s| s.kind == StemKind::Fill(k)) {
                    // Two bars into the pass.
                    add(at + 2 * 128 * 2 * (song.sample_rate / 60) as usize, &fill.samples, false);
                }
            }
            // The game's music volume, and its mixer's ceiling.
            let samples: Vec<f32> = mix.iter().map(|v| (v * 0.7).clamp(-1.0, 1.0)).collect();
            let all = au2_music::RenderedStem { kind: StemKind::Beat, sample_rate: song.sample_rate, samples, looped: false };
            std::fs::write(out, wav::to_wav_bytes(&all))?;
            println!("{out}  {:.1} s", all.seconds());
            Ok(())
        }
        [cmd, path] if cmd == "levels" => {
            // Each track on its own (the others muted, so side-chains still see the kick): RMS and peak.
            let song = Song::from_json(&std::fs::read_to_string(path)?)?;
            for (kind, stem) in song.stem_list() {
                let all = au2_music::render_stem(&song, kind, stem);
                println!("{:<12} rms {:.4} peak {:.3}", kind.file_suffix(), rms(&all.samples), peak(&all.samples));
                for ti in 0..stem.tracks.len() {
                    let mut solo = stem.clone();
                    for (k, t) in solo.tracks.iter_mut().enumerate() {
                        if k != ti {
                            t.gain = 0.0;
                        }
                    }
                    let r = au2_music::render_stem(&song, kind, &solo);
                    println!("    {:<8} rms {:.4} peak {:.3}", stem.tracks[ti].instrument, rms(&r.samples), peak(&r.samples));
                }
            }
            Ok(())
        }
        _ => Err("usage: au2-music check <song.json> | render <song.json> <out_dir> | mix <song.json> <out.wav> | levels <song.json>".into()),
    }
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}
