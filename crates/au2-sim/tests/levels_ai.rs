//! Runs the shipped levels through the sim with the AI driving every faction (levels run in parallel threads).

use au2_map::*;
use au2_sim::*;
use std::path::PathBuf;

fn levels_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels")
}

/// Run `f` on every shipped level in parallel; results in index order.
fn each_level<T: Send>(f: impl Fn(&LevelEntry, &Map) -> T + Sync) -> Vec<(String, T)> {
    let dir = levels_dir();
    let entries = load_index(&dir).unwrap();
    std::thread::scope(|sc| {
        let hs: Vec<_> = entries
            .iter()
            .map(|e| {
                let (dir, f) = (dir.clone(), &f);
                sc.spawn(move || (e.stem.clone(), f(e, &load_level(&dir, e).unwrap())))
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Plays until decided or `max_ticks`; returns (ticks, status, sun counts per faction, state hash).
fn play(map: &Map, cfg: SimConfig, max_ticks: u64) -> (u64, Status, Vec<usize>, u64) {
    let mut sim = Sim::new(map, cfg);
    while sim.tick < max_ticks && sim.status == Status::Playing {
        sim.step(&[]);
        sim.take_events();
    }
    let suns = (0..sim.faction_count()).map(|f| sim.sun_count(f as u8)).collect();
    (sim.tick, sim.status, suns, sim.state_hash())
}

#[test]
fn every_shipped_level_runs_with_ai_and_is_deterministic() {
    let res = each_level(|e, map| {
        let cfg = || SimConfig { seed: 42, ai_difficulty: e.difficulty, human_ai: true, enable_ai: true };
        (play(map, cfg(), 60 * 60 * 2), play(map, cfg(), 60 * 60 * 2))
    });
    for (stem, (a, b)) in res {
        assert_eq!(a, b, "{stem} not deterministic");
    }
}

/// The AI must be able to finish every shipped level: against a human who never issues an order (but whose suns
/// still defend themselves), the AI factions eliminate the human within a generous limit of game time.
#[test]
fn ai_finishes_every_level_against_an_idle_human() {
    let res = each_level(|e, map| play(map, SimConfig { seed: 7, ai_difficulty: e.difficulty.max(1), human_ai: false, enable_ai: true }, 60 * 60 * 20));
    let mut unfinished = Vec::new();
    for (stem, (ticks, status, _, _)) in res {
        println!("{:<24} {:>6.1}s {:?}", stem, ticks as f64 / 60.0, status);
        if status != Status::Lost {
            unfinished.push(stem);
        }
    }
    assert!(unfinished.is_empty(), "AI did not finish: {unfinished:?}");
}

/// Mirror matches (the hardest AI on every faction) are allowed to stall, but should mostly be decided.
#[test]
fn ai_mirror_matches_mostly_finish() {
    let res = each_level(|_, map| play(map, SimConfig { seed: 7, ai_difficulty: 3, human_ai: true, enable_ai: true }, 60 * 60 * 20));
    let total = res.len();
    let mut done = 0;
    for (stem, (ticks, status, suns, _)) in res {
        println!("{:<24} {:>6.1}s {:?} suns {:?}", stem, ticks as f64 / 60.0, status, suns);
        if status != Status::Playing {
            done += 1;
        }
    }
    assert!(done * 10 >= total * 8, "only {done}/{total} mirror matches finished");
}
