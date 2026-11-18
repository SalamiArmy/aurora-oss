//! Diagnostic: play a shipped level headless and print a timeline.
//! `cargo run --release -p au2-sim --example play -- <stem> [minutes] [difficulty] [seed] [idle]`
//! `idle` = the human never moves (otherwise the AI plays every faction).

use au2_map::{load_index, load_level};
use au2_sim::*;
use std::path::PathBuf;

fn all(minutes: u64, idle: bool, seed: u64) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    let entries = load_index(&dir).unwrap();
    let t0 = std::time::Instant::now();
    let res: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = entries
            .iter()
            .map(|e| {
                let dir = dir.clone();
                sc.spawn(move || {
                    let map = load_level(&dir, e).unwrap();
                    let d = if idle { e.difficulty.max(1) } else { 3 };
                    let mut sim = Sim::new(&map, SimConfig { seed, ai_difficulty: d, human_ai: !idle, enable_ai: true });
                    let mut peak = 0;
                    while sim.tick < minutes * 3600 && sim.status == Status::Playing {
                        sim.step(&[]);
                        sim.take_events();
                        peak = peak.max(sim.ships.len());
                    }
                    let suns: Vec<usize> = (0..sim.faction_count()).map(|f| sim.sun_count(f as u8)).collect();
                    format!("{:<22} d{} {:>6.0}s {:?} suns {:?} peak {}", e.stem, d, sim.tick as f64 / 60.0, sim.status, suns, peak)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in res {
        println!("{r}");
    }
    println!("{:.1}s wall", t0.elapsed().as_secs_f64());
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().is_some_and(|s| s == "all") {
        let minutes = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
        return all(minutes, a.get(2).is_some_and(|s| s == "idle"), a.get(3).and_then(|s| s.parse().ok()).unwrap_or(7));
    }
    let stem = a.first().cloned().unwrap_or("dawn_twin".into());
    let minutes: u64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    let e = load_index(&dir).unwrap().into_iter().find(|e| e.stem == stem).expect("no such level");
    let diff: u8 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(e.difficulty);
    let seed: u64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let idle = a.get(4).is_some_and(|s| s == "idle");
    let map = load_level(&dir, &e).unwrap();
    let mut sim = Sim::new(&map, SimConfig { seed, ai_difficulty: diff, human_ai: !idle, enable_ai: true });
    let t0 = std::time::Instant::now();
    while sim.tick < minutes * 3600 && sim.status == Status::Playing {
        sim.step(&[]);
        sim.take_events();
        if sim.tick % 1800 == 0 {
            let row: Vec<String> = (0..sim.faction_count())
                .map(|f| format!("f{f}: {:>2} suns {:>5} str {:>4} dots {:>3}/s", sim.sun_count(f as u8), sim.strength(f as u8), sim.ship_count(f as u8), sim.production(f as u8)))
                .collect();
            println!("{:>5.0}s  {}", sim.tick as f64 / 60.0, row.join(" | "));
        }
    }
    if std::env::var("DUMP").is_ok() { dump(&sim); }
    println!("{stem}: {:?} at {:.0}s ({} ships, {:.1}s wall)", sim.status, sim.tick as f64 / 60.0, sim.ships.len(), t0.elapsed().as_secs_f64());
}
#[allow(dead_code)]
fn dump(sim: &Sim) {
    use std::collections::BTreeMap;
    let mut m: BTreeMap<(u8, String), (u32, f64)> = BTreeMap::new();
    for s in &sim.ships {
        let d = sim.target_pos(s.target).map(|p| p.dist(s.pos)).unwrap_or(0.0);
        let e = m.entry((s.faction, format!("{:?}/{:?}", s.target, s.mode))).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += d;
    }
    for ((f, k), (n, d)) in m {
        if n > 3 {
            println!("f{f} {k}: {n} ships, mean dist {:.0}", d / n as f64);
        }
    }
}
