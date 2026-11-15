//! The winit application: window/surface lifecycle (including Android suspend/resume), input translation (mouse,
//! keyboard, touch), the level menu and the frame loop. Rendering is software; see `render.rs`.

use crate::game::{Game, Input, Overlay};
use crate::gfx::Canvas;
use crate::render::{self, button, UiAction, FACTION_COLORS, WHITE};
use crate::source::LevelSource;
use crate::touch::{Phase, Touch};
use au2_map::LevelEntry;
use au2_sim::{Command, Status};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Canvas height (logical pixels) the mobile UI aims for: the integer scale is `floor(short side / this)`.
const MOBILE_TARGET_SHORT: f32 = 480.0;
/// Touch bottom bar height (canvas pixels).
const TOUCH_BAR_H: f32 = 60.0;

pub fn clock_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

enum State {
    Menu { sel: usize, error: Option<String> },
    Play(Box<Game>),
}

enum Transition {
    Start(usize),
    Menu(usize),
}

pub struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    canvas: Canvas,
    input: Input,
    touch: Touch,
    state: State,
    source: Box<dyn LevelSource>,
    entries: Vec<LevelEntry>,
    seed: Option<u64>,
    last: Instant,
    next_frame: Instant,
    help: bool,
    start_level: Option<String>,
    /// Mobile mode: touch UI from the first frame and the canvas is rendered at 1/scale of the surface, then upscaled.
    mobile: bool,
    scale: i32,
    last_landscape: Option<bool>,
}

impl App {
    /// `start_level`: a level stem to open straight away instead of the menu.
    pub fn new(source: Box<dyn LevelSource>, seed: Option<u64>, start_level: Option<String>, mobile: bool) -> Result<App, String> {
        let entries = source.index()?;
        let input = Input { touch_mode: mobile, ..Default::default() };
        Ok(App {
            window: None,
            surface: None,
            canvas: Canvas::new(1280, 800),
            input,
            touch: Touch::default(),
            state: State::Menu { sel: 0, error: None },
            source,
            entries,
            seed,
            last: Instant::now(),
            next_frame: Instant::now(),
            help: false,
            start_level,
            mobile,
            scale: 1,
            last_landscape: None,
        })
    }

    fn start_level(&mut self, index: usize) {
        let e = self.entries[index].clone();
        match self.source.level(&e) {
            Ok(map) => {
                let view = (self.canvas.w as f32, self.canvas.h as f32);
                let seed = self.seed.unwrap_or_else(clock_seed);
                let mut g = Game::new(e, index, map, seed, view);
                g.bar_h = self.bar_h();
                g.fit(view);
                self.state = State::Play(Box::new(g));
            }
            Err(err) => {
                self.state = State::Menu { sel: index, error: Some(format!("cannot load level: {err}")) };
            }
        }
    }

    fn bar_h(&self) -> f32 {
        if self.input.touch_mode {
            TOUCH_BAR_H
        } else {
            crate::game::BAR_H
        }
    }

    /// Integer UI scale for a surface of this size (always 1 on desktop).
    fn scale_for(&self, w: i32, h: i32) -> i32 {
        if self.mobile {
            ((w.min(h) as f32 / MOBILE_TARGET_SHORT).floor() as i32).max(1)
        } else {
            1
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let Some(win) = self.window.clone() else { return };
        let size = win.inner_size();
        let (pw, ph) = (size.width as i32, size.height as i32);
        if pw <= 0 || ph <= 0 {
            return;
        }
        self.scale = self.scale_for(pw, ph);
        let (w, h) = ((pw / self.scale).max(1), (ph / self.scale).max(1));
        self.canvas.resize(w, h);
        let view = (w as f32, h as f32);
        if self.input.key(KeyCode::F1) {
            self.help = !self.help;
        }
        // orientation change: re-frame the board
        let landscape = w >= h;
        let rotated = self.last_landscape.is_some_and(|l| l != landscape);
        self.last_landscape = Some(landscape);
        let bar_h = self.bar_h();

        let mut next: Option<Transition> = None;
        match &mut self.state {
            State::Menu { sel, error } => {
                if let Some(t) = draw_menu(&mut self.canvas, &self.entries, sel, error.as_deref(), &self.input) {
                    next = Some(t);
                }
            }
            State::Play(g) => {
                g.bar_h = bar_h;
                if rotated {
                    g.fit(view);
                }
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
        if self.help && !self.input.touch_mode {
            render::draw_help(&mut self.canvas);
        }
        match next {
            Some(Transition::Start(i)) if i < self.entries.len() => self.start_level(i),
            Some(Transition::Start(_)) => self.state = State::Menu { sel: 0, error: None },
            Some(Transition::Menu(i)) => self.state = State::Menu { sel: i, error: None },
            None => {}
        }

        self.present(pw, ph);
        self.input.end_frame();
        if self.input.touch_mode && !self.touch.active() {
            // no cursor on a touch screen: nothing may stay "hovered" after the finger lifts
            self.input.mouse = (-10_000.0, -10_000.0);
        }
    }

    /// Copy the canvas to the window surface, upscaling by `self.scale` (nearest neighbour).
    fn present(&mut self, pw: i32, ph: i32) {
        let Some(surface) = &mut self.surface else { return };
        let (Some(a), Some(b)) = (NonZeroU32::new(pw as u32), NonZeroU32::new(ph as u32)) else { return };
        if surface.resize(a, b).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let s = self.scale.max(1) as usize;
        let (cw, ch) = (self.canvas.w as usize, self.canvas.h as usize);
        if s == 1 {
            if buf.len() == self.canvas.buf.len() {
                buf.copy_from_slice(&self.canvas.buf);
            }
        } else if buf.len() == (pw * ph) as usize {
            let pw = pw as usize;
            for y in 0..ph as usize {
                let src = &self.canvas.buf[(y / s).min(ch - 1) * cw..][..cw];
                let dst = &mut buf[y * pw..][..pw];
                for (x, d) in dst.iter_mut().enumerate() {
                    *d = src[(x / s).min(cw - 1)];
                }
            }
        }
        let _ = buf.present();
    }

    fn create_window(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes().with_title("Aurora").with_inner_size(LogicalSize::new(1280.0, 800.0));
        let win = Rc::new(el.create_window(attrs).expect("create window"));
        let ctx = softbuffer::Context::new(win.clone()).expect("graphics context");
        self.surface = Some(softbuffer::Surface::new(&ctx, win.clone()).expect("surface"));
        let s = win.inner_size();
        self.scale = self.scale_for(s.width as i32, s.height as i32);
        self.canvas.resize((s.width as i32 / self.scale).max(1), (s.height as i32 / self.scale).max(1));
        self.window = Some(win);
        self.last = Instant::now();
    }

    /// Where does a new touch land? `(on one of your suns, on UI/menu/overlay)`
    fn touch_target(&self, p: (f32, f32)) -> (bool, bool) {
        let State::Play(g) = &self.state else { return (false, true) };
        let view = (self.canvas.w as f32, self.canvas.h as f32);
        let in_ui = g.in_bar(p, view) || g.overlay != Overlay::None || g.sim.status != Status::Playing;
        if in_ui {
            return (false, true);
        }
        let own = g.sim.pick(g.to_world(p, view), (22.0 / g.zoom) as f64).and_then(|i| g.sim.sun(i)).is_some_and(|s| s.faction == 0 && s.present());
        (own, false)
    }
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

    let (bh, pitch) = if input.touch_mode { (42.0, 50.0) } else { (36.0, 44.0) };
    let cols = groups.len() as f32;
    let col_w = ((w - 40.0) / cols).min(220.0);
    let x0 = (w - col_w * cols) / 2.0;
    let mut chosen = None;
    for (g_i, (name, idxs)) in groups.iter().enumerate() {
        let x = x0 + g_i as f32 * col_w;
        c.text(x + 8.0, 130.0, &name.to_uppercase(), 2, FACTION_COLORS[g_i % 4], 1.0);
        for (r, &li) in idxs.iter().enumerate() {
            let e = &entries[li];
            let y = 164.0 + r as f32 * pitch;
            let active = li == *sel;
            if button(c, input, x + 4.0, y, col_w - 12.0, bh, &e.name, active) {
                chosen = Some(li);
            }
            for d in 0..4 {
                let col = if d <= e.difficulty { [255, 200, 90] } else { [50, 55, 75] };
                c.disc(x + 14.0 + d as f32 * 9.0, y + bh - 7.0, 2.5, col, 1.0);
            }
        }
    }
    let hint = if input.touch_mode { "tap a level" } else { "click a level, or arrows + Enter     F1: controls     in game: Esc = menu" };
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
        self.create_window(el);
        if let Some(stem) = self.start_level.take() {
            match self.entries.iter().position(|e| e.stem == stem) {
                Some(i) => self.start_level(i),
                None => self.state = State::Menu { sel: 0, error: Some(format!("no such level '{stem}' in levels.txt")) },
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Android: the native window is destroyed when the app goes to the background. Drop the surface and window (the
    /// surface first, it borrows the window) and pause the game; `resumed` builds new ones.
    fn suspended(&mut self, _el: &ActiveEventLoop) {
        self.surface = None;
        self.window = None;
        self.touch = Touch::default();
        self.input.down.clear();
        if let State::Play(g) = &mut self.state {
            if g.sim.status == Status::Playing {
                g.overlay = Overlay::Pause;
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let s = self.scale.max(1) as f32;
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Focused(false) => self.input.down.clear(),
            WindowEvent::Touch(t) => {
                let pos = (t.location.x as f32 / s, t.location.y as f32 / s);
                let phase = match t.phase {
                    TouchPhase::Started => Phase::Started,
                    TouchPhase::Moved => Phase::Moved,
                    TouchPhase::Ended => Phase::Ended,
                    TouchPhase::Cancelled => Phase::Cancelled,
                };
                let (on_sun, in_ui) = if phase == Phase::Started { self.touch_target(pos) } else { (false, false) };
                self.touch.event(phase, t.id, pos, &mut self.input, on_sun, in_ui);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse = (position.x as f32 / s, position.y as f32 / s);
                if !self.mobile {
                    self.input.touch_mode = false;
                }
            }
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
                    // Android's Back button behaves like Esc
                    let code = if code == KeyCode::BrowserBack { KeyCode::Escape } else { code };
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

/// Run the app to completion on the given event loop.
pub fn run(event_loop: EventLoop<()>, mut app: App) -> Result<(), winit::error::EventLoopError> {
    event_loop.run_app(&mut app)
}
