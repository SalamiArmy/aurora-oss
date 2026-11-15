//! Aurora RTS desktop front-end: command line, repository-root discovery and the headless snapshot mode. The game,
//! renderer and window loop live in the `au2_app` library (shared with the Android entry crate).

use au2_app::game::{self, Game, Input};
use au2_app::gfx::Canvas;
use au2_app::{png, render, App, FsSource, LevelSource};
use au2_sim::SimConfig;
use std::path::{Path, PathBuf};
use winit::event_loop::EventLoop;

const USAGE: &str = "\
au2-app [LEVEL] [options]

  LEVEL            file stem from levels/levels.txt (e.g. dawn_twin); omit to get the level menu
  --root DIR       repository root containing levels/ (default: search upwards from the working dir / executable)
  --seed N         simulation seed (default: from the clock)
  --snapshot FILE  headless: run the level for --ticks steps (AI plays both sides), write a PNG, exit
  --ticks N        steps for --snapshot (default 1800)
  --size WxH       snapshot size (default 1280x800)
  -h, --help       this text
";

// ---------------------------------------------------------------------------------------------- args and paths
struct Args {
    level: Option<String>,
    root: Option<PathBuf>,
    seed: Option<u64>,
    snapshot: Option<PathBuf>,
    ticks: u64,
    size: (i32, i32),
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { level: None, root: None, seed: None, snapshot: None, ticks: 1800, size: (1280, 800) };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--root" => a.root = Some(PathBuf::from(val("--root")?)),
            "--seed" => a.seed = Some(val("--seed")?.parse().map_err(|_| "bad --seed".to_string())?),
            "--snapshot" => a.snapshot = Some(PathBuf::from(val("--snapshot")?)),
            "--ticks" => a.ticks = val("--ticks")?.parse().map_err(|_| "bad --ticks".to_string())?,
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size wants WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n\n{USAGE}")),
            s => a.level = Some(s.to_string()),
        }
    }
    Ok(a)
}

/// Find the repository root: the first directory (cwd, executable, and their parents) holding `levels/levels.txt`.
fn find_root(explicit: &Option<PathBuf>) -> Option<PathBuf> {
    let ok = |p: &Path| p.join("levels").join("levels.txt").is_file();
    if let Some(r) = explicit {
        return ok(r).then(|| r.clone());
    }
    if let Ok(r) = std::env::var("AU2_ROOT") {
        let r = PathBuf::from(r);
        if ok(&r) {
            return Some(r);
        }
    }
    let mut starts = Vec::new();
    if let Ok(d) = std::env::current_dir() {
        starts.push(d);
    }
    if let Ok(e) = std::env::current_exe() {
        if let Some(d) = e.parent() {
            starts.push(d.to_path_buf());
        }
    }
    for s in starts {
        let mut cur = Some(s.as_path());
        while let Some(d) = cur {
            if ok(d) {
                return Some(d.to_path_buf());
            }
            cur = d.parent();
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------- headless snapshot
fn snapshot(source: &FsSource, args: &Args, out: &Path) -> Result<(), String> {
    let entries = source.index()?;
    let stem = args.level.clone().unwrap_or_else(|| entries[0].stem.clone());
    let (i, e) = entries.iter().enumerate().find(|(_, e)| e.stem == stem).ok_or(format!("no such level '{stem}'"))?;
    let map = source.level(e)?;
    let (w, h) = args.size;
    let mut g = Game::new(e.clone(), i, map.clone(), args.seed.unwrap_or(1), (w as f32, h as f32));
    // AI plays both sides so there is something to look at
    g.sim = au2_sim::Sim::new(&map, SimConfig { seed: args.seed.unwrap_or(1), ai_difficulty: e.difficulty.max(1), human_ai: true, enable_ai: true });
    for _ in 0..args.ticks {
        g.sim.step(&[]);
        for ev in g.sim.take_events() {
            if let au2_sim::Event::Combat { pos, a, .. } = ev {
                g.sparks.push(game::Spark { pos, age: 0.2, color_faction: a });
            }
        }
        g.sparks.retain(|s| s.age < 0.5);
    }
    g.selection = g.own_sun_ids().into_iter().take(2).collect();
    g.clock = args.ticks as f32 / 60.0;
    let mut c = Canvas::new(w, h);
    let input = Input { mouse: (w as f32 / 2.0, h as f32 / 2.0), ..Default::default() };
    render::draw_game(&mut c, &g, &input);
    render::draw_ui(&mut c, &mut g, &Input::default());
    std::fs::write(out, png::encode(w as u32, h as u32, &c.buf)).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    println!("wrote {} ({} ticks, {} ships, status {:?})", out.display(), g.sim.tick, g.sim.ships.len(), g.sim.status);
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let Some(root) = find_root(&args.root) else {
        eprintln!("cannot find levels/levels.txt - run from the repository, or pass --root DIR (or set AU2_ROOT)");
        std::process::exit(1);
    };
    let source = FsSource { root };
    if let Some(out) = &args.snapshot {
        if let Err(e) = snapshot(&source, &args, out) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let app = match App::new(Box::new(source), args.seed, args.level.clone(), false) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let event_loop = EventLoop::new().expect("event loop");
    au2_app::run(event_loop, app).expect("run");
}
