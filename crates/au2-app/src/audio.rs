//! Sound effects from `assets/*.ogg`.
//!
//! * Combat plays notes of one instrument; the note follows a 16-step bar (one step = 15 ticks = a quarter second),
//!   so fights sound rhythmic. The sequence differs when the player is winning, losing or roughly even.
//! * Upgrades play a rising tone, reaching the maximum level a bigger swell; orders, repeat orders, hits and captures
//!   have their own sounds.
//!
//! Missing files and missing audio devices are never fatal: the game just stays quiet.

use au2_sim::{Event, Sim};

/// -1 losing, 0 even, 1 winning for the human, from ship counts.
pub fn mood(sim: &Sim) -> i8 {
    let mine = sim.ship_count(0) as f32;
    let theirs: f32 = (1..sim.faction_count()).map(|f| sim.ship_count(f as u8) as f32).sum();
    if mine > theirs * 1.3 + 3.0 {
        1
    } else if theirs > mine * 1.3 + 3.0 {
        -1
    } else {
        0
    }
}

const NOTES: [&str; 11] = ["c1", "d1", "e1", "g1", "a2", "c2", "d2", "e2", "g2", "a3", "c3"];
const EVEN: [usize; 16] = [0, 2, 4, 2, 5, 4, 2, 4, 0, 2, 4, 5, 7, 5, 4, 2];
const WINNING: [usize; 16] = [4, 5, 7, 8, 9, 8, 7, 5, 4, 5, 7, 9, 10, 9, 7, 5];
const LOSING: [usize; 16] = [4, 2, 1, 0, 4, 2, 1, 0, 5, 4, 2, 1, 4, 2, 1, 0];
const INSTRUMENTS: [&str; 7] = ["synthetic", "acoustic", "percussion", "analog", "popcorn", "lunar", "voyager"];

/// Which file (without extension) to play for a combat step.
pub fn combat_note(instrument: &str, step: u64, mood: i8) -> String {
    let seq = match mood {
        1 => &WINNING,
        -1 => &LOSING,
        _ => &EVEN,
    };
    format!("sfx_{}_{}", instrument, NOTES[seq[(step % 16) as usize]])
}

pub const STEP_TICKS: u64 = 15;

#[cfg(feature = "audio")]
mod imp {
    use super::*;
    use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub struct Audio {
        _stream: Option<OutputStream>,
        handle: Option<OutputStreamHandle>,
        dir: PathBuf,
        cache: HashMap<String, Option<Arc<[u8]>>>,
        instrument: &'static str,
        last_step: u64,
        last_hit: u64,
        pub muted: bool,
        played_this_frame: u32,
    }

    impl Audio {
        pub fn new(assets: PathBuf) -> Audio {
            let (stream, handle) = match OutputStream::try_default() {
                Ok((s, h)) => (Some(s), Some(h)),
                Err(_) => (None, None),
            };
            Audio { _stream: stream, handle, dir: assets, cache: HashMap::new(), instrument: INSTRUMENTS[0], last_step: u64::MAX, last_hit: 0, muted: false, played_this_frame: 0 }
        }

        pub fn available(&self) -> bool {
            self.handle.is_some()
        }

        /// Pick the combat instrument for a level (by constellation).
        pub fn set_instrument(&mut self, index: usize) {
            self.instrument = INSTRUMENTS[index % INSTRUMENTS.len()];
            self.last_step = u64::MAX;
        }

        fn bytes(&mut self, name: &str) -> Option<Arc<[u8]>> {
            if let Some(c) = self.cache.get(name) {
                return c.clone();
            }
            let loaded = std::fs::read(self.dir.join(format!("{name}.ogg"))).ok().map(|v| Arc::from(v.into_boxed_slice()));
            self.cache.insert(name.to_string(), loaded.clone());
            loaded
        }

        pub fn play(&mut self, name: &str, volume: f32) {
            if self.muted || self.played_this_frame >= 8 {
                return;
            }
            let Some(data) = self.bytes(name) else { return };
            let Some(handle) = &self.handle else { return };
            if let Ok(dec) = Decoder::new(Cursor::new(data)) {
                let _ = handle.play_raw(dec.convert_samples::<f32>().amplify(volume));
                self.played_this_frame += 1;
            }
        }

        pub fn begin_frame(&mut self) {
            self.played_this_frame = 0;
        }

        /// React to this frame's sim events. `owner0_suns` tells which suns belong to the human (for upgrade sounds).
        pub fn handle(&mut self, events: &[Event], tick: u64, mood: i8, is_human_sun: &dyn Fn(usize) -> bool) {
            let mut combats = 0u32;
            for e in events {
                match e {
                    Event::Combat { .. } => combats += 1,
                    Event::Launch { faction: 0, .. } => {}
                    Event::SunHit { sun } if tick >= self.last_hit + 8 => {
                        self.last_hit = tick;
                        let name = if is_human_sun(*sun) { "sfx_synthetic_hitsun" } else { "sfx_percussion_hitsun" };
                        self.play(name, 0.35);
                    }
                    Event::SunUpgraded { sun, .. } if is_human_sun(*sun) => self.play("sfx_percussion_upgrade", 0.6),
                    Event::SunMaxed { sun } if is_human_sun(*sun) => {
                        self.play("sfx_percussion_upgrade", 0.9);
                        self.play("sfx_synthetic_c3", 0.7);
                    }
                    Event::SunCaptured { faction: 0, .. } => self.play("sfx_synthetic_healsun", 0.6),
                    _ => {}
                }
            }
            if combats > 0 {
                let step = tick / STEP_TICKS;
                if step != self.last_step {
                    self.last_step = step;
                    let name = combat_note(self.instrument, step, mood);
                    self.play(&name, 0.25 + 0.05 * combats.min(6) as f32);
                }
            }
        }
    }
}

#[cfg(not(feature = "audio"))]
mod imp {
    use super::*;
    use std::path::PathBuf;
    pub struct Audio {
        pub muted: bool,
    }
    impl Audio {
        pub fn new(_assets: PathBuf) -> Audio {
            Audio { muted: false }
        }
        pub fn available(&self) -> bool {
            false
        }
        pub fn set_instrument(&mut self, _i: usize) {}
        pub fn play(&mut self, _n: &str, _v: f32) {}
        pub fn begin_frame(&mut self) {}
        pub fn handle(&mut self, _e: &[Event], _t: u64, _m: i8, _h: &dyn Fn(usize) -> bool) {}
    }
}

pub use imp::Audio;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combat_notes_follow_the_bar_and_change_with_mood() {
        let a = combat_note("synthetic", 0, 0);
        assert_eq!(a, "sfx_synthetic_c1");
        assert_eq!(combat_note("synthetic", 16, 0), a, "the sequence repeats every bar (16 steps)");
        assert_ne!(combat_note("synthetic", 3, 1), combat_note("synthetic", 3, -1));
        assert_ne!(combat_note("synthetic", 3, 1), combat_note("synthetic", 3, 0));
    }

    #[test]
    fn every_note_in_every_sequence_exists_in_assets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        for inst in INSTRUMENTS {
            for mood in [-1, 0, 1] {
                for step in 0..16 {
                    let f = dir.join(format!("{}.ogg", combat_note(inst, step, mood)));
                    assert!(f.is_file(), "missing {}", f.display());
                }
            }
        }
    }
}
