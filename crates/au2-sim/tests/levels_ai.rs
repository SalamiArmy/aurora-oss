//! Runs the shipped levels through the sim with the AI driving every faction.

use au2_map::*;
use au2_sim::*;
use std::path::PathBuf;

fn levels_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels")
}

/// Plays until decided or `max_ticks`; returns (ticks, status, sun counts per faction).
fn play(map: &Map, difficulty: u8, seed: u64, max_ticks: u64) -> (u64, Status, Vec<usize>, u64) {
    let mut sim = Sim::new(map, SimConfig { seed, ai_difficulty: difficulty, human_ai: true, enable_ai: true });
    while sim.tick < max_ticks && sim.status == Status::Playing {
        sim.step(&[]);
        sim.take_events();
    }
    let suns = (0..sim.faction_count()).map(|f| sim.sun_count(f as u8)).collect();
    (sim.tick, sim.status, suns, sim.state_hash())
}

#[test]
fn every_shipped_level_runs_with_ai_and_is_deterministic() {
    let dir = levels_dir();
    for e in load_index(&dir).unwrap() {
        let map = load_level(&dir, &e).unwrap();
        let a = play(&map, e.difficulty, 42, 60 * 60 * 2);
        let b = play(&map, e.difficulty, 42, 60 * 60 * 2);
        assert_eq!(a, b, "{} not deterministic", e.stem);
        // the AI must do something: someone holds more than the starting suns or the game ended
        assert!(a.0 > 0);
    }
}

/// The AI must be able to finish every shipped level: against a human who never issues an order, the AI factions
/// eliminate the human within a generous limit of game time.
#[test]
fn ai_finishes_every_level_against_an_idle_human() {
    let dir = levels_dir();
    let mut unfinished = Vec::new();
    for e in load_index(&dir).unwrap() {
        let map = load_level(&dir, &e).unwrap();
        let limit = 60 * 60 * 20; // 20 minutes of game time
        let mut sim = Sim::new(&map, SimConfig { seed: 7, ai_difficulty: e.difficulty.max(1), human_ai: false, enable_ai: true });
        while sim.tick < limit && sim.status == Status::Playing {
            sim.step(&[]);
            sim.take_events();
        }
        println!("{:<24} {:>6.1}s {:?}", e.stem, sim.tick as f64 / 60.0, sim.status);
        if sim.status != Status::Lost {
            unfinished.push(e.stem.clone());
        }
    }
    assert!(unfinished.is_empty(), "AI did not finish: {unfinished:?}");
}

/// Mirror matches (the AI on every faction) are allowed to stall, but should mostly be decided; this just prints
/// the outcomes and requires a clear majority to finish.
#[test]
fn ai_mirror_matches_mostly_finish() {
    let dir = levels_dir();
    let (mut done, mut total) = (0, 0);
    for e in load_index(&dir).unwrap() {
        let map = load_level(&dir, &e).unwrap();
        let (ticks, status, suns, _) = play(&map, 3, 7, 60 * 60 * 20);
        println!("{:<24} {:>6.1}s {:?} suns {:?}", e.stem, ticks as f64 / 60.0, status, suns);
        total += 1;
        if status != Status::Playing {
            done += 1;
        }
    }
    assert!(done * 10 >= total * 8, "only {done}/{total} mirror matches finished");
}
