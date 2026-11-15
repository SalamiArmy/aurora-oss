//! Aurora RTS front-end: loads a level, runs the deterministic sim, renders it in software and turns mouse/keyboard
//! input into orders. Window/event handling is winit + softbuffer so it builds on the stock GNU toolchain.

mod game;
mod gfx;
mod png;
mod render;

use au2_map::{load_index, load_level, LevelEntry};
use au2_sim::{Command, SimConfig, Status};
use game::{Game, Input, Overlay};
use gfx::Canvas;
use render::{button, UiAction, FACTION_COLORS, WHITE};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

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

fn clock_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

// ---------------------------------------------------------------------------------------------- app state
enum State {
    Menu { sel: usize, error: Option<String> },
    Play(Box<Game>),
}

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    canvas: Canvas,
    input: Input,
    state: State,
    root: PathBuf,
    entries: Vec<LevelEntry>,
    seed: Option<u64>,
    last: Instant,
    next_frame: Instant,
    help: bool,
    start_level: Option<String>,
}

impl App {
    fn start_level(&mut self, index: usize) {
        let e = self.entries[index].clone();
        match load_level(&self.root.join("levels"), &e) {
            Ok(map) => {
                let view = (self.canvas.w as f32, self.canvas.h as f32);
                let seed = self.seed.unwrap_or_else(clock_seed);
                self.state = State::Play(Box::new(Game::new(e, index, map, seed, view)));
            }
            Err(err) => {
                self.state = State::Menu { sel: index, error: Some(format!("cannot load level: {err}")) };
            }
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let Some(win) = self.window.clone() else { return };
        let size = win.inner_size();
        let (w, h) = (size.width as i32, size.height as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        self.canvas.resize(w, h);
        if self.input.key(KeyCode::F1) {
            self.help = !self.help;
        }

        let mut next: Option<Transition> = None;
        match &mut self.state {
            State::Menu { sel, error } => {
                if let Some(t) = draw_menu(&mut self.canvas, &self.entries, sel, error.as_deref(), &self.input) {
                    next = Some(t);
                }
            }
            State::Play(g) => {
                let view = (w as f32, h as f32);
                g.update(&self.input, dt, view);
                render::draw_game(&mut self.canvas, g, &self.input);
                match render::draw_ui(&mut self.canvas, g, &self.input) {
                    Some(UiAction::Resume) => g.overlay = Overlay::None,
                    Some(UiAction::Restart) => g.restart(),
                    Some(UiAction::Surrender) => {
                        g.cmds.push(Command::Surrender { faction: 0 });
                        g.overlay = Overlay::None;
                        g.paused = false;
                    }
                    Some(UiAction::Levels) => next = Some(Transition::Menu(g.level_index)),
                    Some(UiAction::Next) => next = Some(Transition::Start(g.level_index + 1)),
                    None => {}
                }
                if g.sim.status == Status::Playing && g.overlay == Overlay::None && self.input.key(KeyCode::F5) {
                    g.restart();
                }
            }
        }
        if self.help {
            render::draw_help(&mut self.canvas);
        }
        match next {
            Some(Transition::Start(i)) if i < self.entries.len() => self.start_level(i),
            Some(Transition::Start(_)) => self.state = State::Menu { sel: 0, error: None },
            Some(Transition::Menu(i)) => self.state = State::Menu { sel: i, error: None },
            None => {}
        }

        if let Some(surface) = &mut self.surface {
            if let (Some(a), Some(b)) = (NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) {
                if surface.resize(a, b).is_ok() {
                    if let Ok(mut buf) = surface.buffer_mut() {
                        if buf.len() == self.canvas.buf.len() {
                            buf.copy_from_slice(&self.canvas.buf);
                        }
                        let _ = buf.present();
                    }
                }
            }
        }
        self.input.end_frame();
    }
}

enum Transition {
    Start(usize),
    Menu(usize),
}

/// The level-select screen. Returns a transition when a level is chosen.
fn draw_menu(c: &mut Canvas, entries: &[LevelEntry], sel: &mut usize, error: Option<&str>, input: &Input) -> Option<Transition> {
    render::background(c);
    let w = c.w as f32;
    c.text_centered(w / 2.0, 24.0, "AURORA", 6, [120, 200, 255], 1.0);
    c.text_centered(w / 2.0, 80.0, "choose a level", 2, [150, 160, 190], 1.0);

    // group by constellation, keeping the file's order
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        match groups.iter_mut().find(|g| g.0 == e.constellation) {
            Some(g) => g.1.push(i),
            None => groups.push((e.constellation.clone(), vec![i])),
        }
    }
    // keyboard navigation
    let pos_of = |idx: usize| -> (usize, usize) {
        for (gi, g) in groups.iter().enumerate() {
            if let Some(r) = g.1.iter().position(|&x| x == idx) {
                return (gi, r);
            }
        }
        (0, 0)
    };
    let (mut gi, mut ri) = pos_of(*sel);
    if input.key(KeyCode::ArrowRight) && gi + 1 < groups.len() {
        gi += 1;
        ri = ri.min(groups[gi].1.len() - 1);
    }
    if input.key(KeyCode::ArrowLeft) && gi > 0 {
        gi -= 1;
        ri = ri.min(groups[gi].1.len() - 1);
    }
    if input.key(KeyCode::ArrowDown) && ri + 1 < groups[gi].1.len() {
        ri += 1;
    }
    if input.key(KeyCode::ArrowUp) && ri > 0 {
        ri -= 1;
    }
    *sel = groups[gi].1[ri];
    if input.key(KeyCode::Enter) || input.key(KeyCode::NumpadEnter) {
        return Some(Transition::Start(*sel));
    }

    let cols = groups.len() as f32;
    let col_w = ((w - 40.0) / cols).min(220.0);
    let x0 = (w - col_w * cols) / 2.0;
    let mut chosen = None;
    for (g_i, (name, idxs)) in groups.iter().enumerate() {
        let x = x0 + g_i as f32 * col_w;
        c.text(x + 8.0, 130.0, &name.to_uppercase(), 2, FACTION_COLORS[g_i % 4], 1.0);
        for (r, &li) in idxs.iter().enumerate() {
            let e = &entries[li];
            let y = 164.0 + r as f32 * 44.0;
            let active = li == *sel;
            if button(c, input, x + 4.0, y, col_w - 12.0, 36.0, &e.name, active) {
                chosen = Some(li);
            }
            for d in 0..4 {
                let col = if d <= e.difficulty { [255, 200, 90] } else { [50, 55, 75] };
                c.disc(x + 14.0 + d as f32 * 9.0, y + 29.0, 2.5, col, 1.0);
            }
        }
    }
    let hint = "click a level, or arrows + Enter     F1: controls     in game: Esc = menu";
    c.text_centered(w / 2.0, c.h as f32 - 28.0, hint, 1, [130, 140, 170], 1.0);
    if let Some(e) = error {
        c.rect(20.0, c.h as f32 - 90.0, w - 40.0, 40.0, [60, 14, 14], 0.9);
        c.text(30.0, c.h as f32 - 76.0, e, 1, [255, 160, 150], 1.0);
    }
    let _ = WHITE;
    chosen.map(Transition::Start)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Aurora").with_inner_size(LogicalSize::new(1280.0, 800.0));
        let win = Rc::new(el.create_window(attrs).expect("create window"));
        let ctx = softbuffer::Context::new(win.clone()).expect("graphics context");
        self.surface = Some(softbuffer::Surface::new(&ctx, win.clone()).expect("surface"));
        let s = win.inner_size();
        self.canvas.resize(s.width as i32, s.height as i32);
        self.window = Some(win);
        if let Some(stem) = self.start_level.take() {
            match self.entries.iter().position(|e| e.stem == stem) {
                Some(i) => self.start_level(i),
                None => self.state = State::Menu { sel: 0, error: Some(format!("no such level '{stem}' in levels.txt")) },
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Focused(false) => self.input.down.clear(),
            WindowEvent::CursorMoved { position, .. } => self.input.mouse = (position.x as f32, position.y as f32),
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        self.input.left_down = down;
                        if down {
                            self.input.left_pressed = true;
                        } else {
                            self.input.left_released = true;
                        }
                    }
                    MouseButton::Right => {
                        self.input.right_down = down;
                        if down {
                            self.input.right_pressed = true;
                        }
                    }
                    MouseButton::Middle => self.input.middle_down = down,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if !event.repeat {
                            self.input.pressed.push(code);
                        }
                        self.input.down.insert(code);
                    } else {
                        self.input.down.remove(&code);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            self.next_frame = now + Duration::from_millis(16);
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

// ---------------------------------------------------------------------------------------------- headless snapshot
fn snapshot(root: &Path, args: &Args, out: &Path) -> Result<(), String> {
    let entries = load_index(&root.join("levels")).map_err(|e| e.to_string())?;
    let stem = args.level.clone().unwrap_or_else(|| entries[0].stem.clone());
    let (i, e) = entries.iter().enumerate().find(|(_, e)| e.stem == stem).ok_or(format!("no such level '{stem}'"))?;
    let map = load_level(&root.join("levels"), e).map_err(|e| e.to_string())?;
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
    if let Some(out) = &args.snapshot {
        if let Err(e) = snapshot(&root, &args, out) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let entries = match load_index(&root.join("levels")) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mut app = App {
        window: None,
        surface: None,
        canvas: Canvas::new(1280, 800),
        input: Input::default(),
        state: State::Menu { sel: 0, error: None },
        root,
        entries,
        seed: args.seed,
        last: Instant::now(),
        next_frame: Instant::now(),
        help: false,
        start_level: args.level.clone(),
    };
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut app).expect("run");
}
