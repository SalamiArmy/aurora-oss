//! Game session state and input handling (no drawing here).

use au2_map::{LevelEntry, Map, Object};
use au2_sim::math::Vec2;
use au2_sim::rules::TICKS_PER_SEC;
use au2_sim::{Command, Event, Obj, OrderError, Sim, SimConfig, Status, Target};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub const BAR_H: f32 = 44.0;

#[derive(Default)]
pub struct Input {
    pub mouse: (f32, f32),
    pub left_down: bool,
    pub right_down: bool,
    pub middle_down: bool,
    pub left_pressed: bool,
    pub left_released: bool,
    pub right_pressed: bool,
    pub wheel: f32,
    pub pressed: Vec<KeyCode>,
    pub down: HashSet<KeyCode>,
    /// Touch front end: bigger hit targets, no hover cursor.
    pub touch_mode: bool,
    /// Pinch zoom factor accumulated this frame (0 = none), anchored at `mouse`.
    pub pinch: f32,
    /// Camera pan accumulated this frame, in screen pixels (finger movement).
    pub pan_px: (f32, f32),
    /// A second finger landed: abandon the lasso in progress.
    pub cancel_drag: bool,
}

impl Input {
    /// Movement (px) beyond which a press-release is a lasso rather than a click.
    pub fn drag_slop(&self) -> f32 {
        if self.touch_mode {
            14.0
        } else {
            8.0
        }
    }
    pub fn key(&self, k: KeyCode) -> bool {
        self.pressed.contains(&k)
    }
    pub fn held(&self, k: KeyCode) -> bool {
        self.down.contains(&k)
    }
    pub fn shift(&self) -> bool {
        self.held(KeyCode::ShiftLeft) || self.held(KeyCode::ShiftRight)
    }
    pub fn ctrl(&self) -> bool {
        self.held(KeyCode::ControlLeft) || self.held(KeyCode::ControlRight)
    }
    pub fn end_frame(&mut self) {
        self.left_pressed = false;
        self.left_released = false;
        self.right_pressed = false;
        self.wheel = 0.0;
        self.pinch = 0.0;
        self.pan_px = (0.0, 0.0);
        self.cancel_drag = false;
        self.pressed.clear();
    }
}

pub struct Spark {
    pub pos: Vec2,
    pub age: f32,
    pub color_faction: u8,
}

pub struct Ping {
    pub pos: Vec2,
    pub age: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Pause,
}

pub struct Game {
    pub entry: LevelEntry,
    pub level_index: usize,
    pub map: Map,
    pub sim: Sim,
    pub seed: u64,
    pub cam: [f32; 2],
    pub zoom: f32,
    pub selection: Vec<usize>,
    pub percent: u8,
    pub repeat: bool,
    pub speed: u32,
    pub paused: bool,
    pub overlay: Overlay,
    pub sparks: Vec<Spark>,
    pub pings: Vec<Ping>,
    pub toast: Option<(String, f32)>,
    pub drag: Option<(f32, f32)>,
    pan: Option<((f32, f32), [f32; 2])>,
    acc: f32,
    pub clock: f32,
    pub cmds: Vec<Command>,
    /// Events produced this frame (for audio); drained by the caller.
    pub events: Vec<Event>,
    pub orders_sent: u32,
    /// Height of the bottom bar in canvas pixels (taller for touch).
    pub bar_h: f32,
}

fn cfg(seed: u64, entry: &LevelEntry) -> SimConfig {
    SimConfig { seed, ai_difficulty: entry.difficulty, human_ai: false, enable_ai: true }
}

impl Game {
    pub fn new(entry: LevelEntry, level_index: usize, map: Map, seed: u64, view: (f32, f32)) -> Game {
        let sim = Sim::new(&map, cfg(seed, &entry));
        let mut g = Game {
            entry,
            level_index,
            map,
            sim,
            seed,
            cam: [0.0, 0.0],
            zoom: 0.05,
            selection: Vec::new(),
            percent: 50,
            repeat: false,
            speed: 1,
            paused: false,
            overlay: Overlay::None,
            sparks: Vec::new(),
            pings: Vec::new(),
            toast: None,
            drag: None,
            pan: None,
            acc: 0.0,
            clock: 0.0,
            cmds: Vec::new(),
            events: Vec::new(),
            orders_sent: 0,
            bar_h: BAR_H,
        };
        g.fit(view);
        g
    }

    pub fn restart(&mut self) {
        self.sim = Sim::new(&self.map, cfg(self.seed, &self.entry));
        self.selection.clear();
        self.sparks.clear();
        self.pings.clear();
        self.cmds.clear();
        self.paused = false;
        self.overlay = Overlay::None;
        self.acc = 0.0;
        self.orders_sent = 0;
    }

    /// Frame the whole board.
    pub fn fit(&mut self, view: (f32, f32)) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut add = |x: f64, y: f64, r: f64| {
            x0 = x0.min((x - r) as f32);
            x1 = x1.max((x + r) as f32);
            y0 = y0.min((y - r) as f32);
            y1 = y1.max((y + r) as f32);
        };
        for o in &self.map.objects {
            match o {
                Object::Sun(s) => add(s.pos[0], s.pos[1], 600.0),
                Object::OrbitingSun(s, or) => add(s.pos[0], s.pos[1], or.rx.max(or.ry) + 600.0),
                Object::FragileSun(f) => add(f.sun.pos[0], f.sun.pos[1], f.orbit.rx.max(f.orbit.ry) + 600.0),
                Object::Wormhole(w) => add(w.pos[0], w.pos[1], w.size + 200.0),
                Object::Supernova(s) => add(s.pos[0], s.pos[1], 600.0),
                Object::WhiteHole(s) => add(s.pos[0], s.pos[1], 600.0),
                _ => {}
            }
        }
        if x0 > x1 {
            (x0, x1, y0, y1) = (-5000.0, 5000.0, -5000.0, 5000.0);
        }
        self.cam = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
        let (vw, vh) = (view.0.max(100.0), (view.1 - self.bar_h).max(100.0));
        self.zoom = (vw / (x1 - x0)).min(vh / (y1 - y0)) * 0.95;
    }

    pub fn to_screen(&self, p: Vec2, view: (f32, f32)) -> (f32, f32) {
        (view.0 / 2.0 + (p.x as f32 - self.cam[0]) * self.zoom, (view.1 - self.bar_h) / 2.0 - (p.y as f32 - self.cam[1]) * self.zoom)
    }

    pub fn to_world(&self, s: (f32, f32), view: (f32, f32)) -> Vec2 {
        Vec2::new(((s.0 - view.0 / 2.0) / self.zoom + self.cam[0]) as f64, (((view.1 - self.bar_h) / 2.0 - s.1) / self.zoom + self.cam[1]) as f64)
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), 2.5));
    }

    pub fn own_sun_ids(&self) -> Vec<usize> {
        self.sim.suns().filter(|(_, s)| s.faction == 0 && s.present()).map(|(i, _)| i).collect()
    }

    pub fn in_bar(&self, mouse: (f32, f32), view: (f32, f32)) -> bool {
        mouse.1 >= view.1 - self.bar_h
    }

    /// Zoom by `factor` keeping the world point under screen position `at` fixed.
    pub fn zoom_at(&mut self, at: (f32, f32), factor: f32, view: (f32, f32)) {
        let before = self.to_world(at, view);
        self.zoom = (self.zoom * factor).clamp(0.01, 0.6);
        let after = self.to_world(at, view);
        self.cam[0] += (before.x - after.x) as f32;
        self.cam[1] += (before.y - after.y) as f32;
    }

    pub fn issue_order(&mut self, target: Target) {
        let sources: Vec<usize> = self.selection.iter().copied().filter(|&i| self.sim.sun(i).map(|s| s.faction == 0 && s.present()).unwrap_or(false)).collect();
        if sources.is_empty() {
            return;
        }
        let cmd = Command::Order { faction: 0, sources, target, then: None, percent: self.percent, repeat: self.repeat };
        // validate right now so the player gets immediate feedback; the sim re-validates when it applies it
        let mut probe_ok = true;
        if let Command::Order { sources, .. } = &cmd {
            if sources.len() > self.sim.factions[0].max_sources as usize {
                self.toast(format!("at most {} suns per order", self.sim.factions[0].max_sources));
                probe_ok = false;
            }
        }
        if probe_ok {
            self.cmds.push(cmd);
            self.orders_sent += 1;
            if let Some(p) = self.sim.target_pos(target) {
                self.pings.push(Ping { pos: p, age: 0.0 });
            }
        }
    }

    /// Process one frame of input. Returns true if the world consumed a click (used by the caller for UI gating).
    pub fn update(&mut self, input: &Input, dt: f32, view: (f32, f32)) {
        self.clock += dt;
        for s in self.sparks.iter_mut() {
            s.age += dt;
        }
        self.sparks.retain(|s| s.age < 0.5);
        for p in self.pings.iter_mut() {
            p.age += dt;
        }
        self.pings.retain(|p| p.age < 0.8);
        if let Some((_, t)) = &mut self.toast {
            *t -= dt;
            if *t <= 0.0 {
                self.toast = None;
            }
        }

        // ---- keyboard (always available)
        if input.key(KeyCode::Escape) {
            self.overlay = if self.overlay == Overlay::Pause { Overlay::None } else { Overlay::Pause };
        }
        let decided = self.sim.status != Status::Playing;
        if self.overlay == Overlay::None {
            if input.key(KeyCode::Space) || input.key(KeyCode::KeyP) {
                self.paused = !self.paused;
            }
            if input.key(KeyCode::Digit1) {
                self.percent = 25;
            }
            if input.key(KeyCode::Digit2) {
                self.percent = 50;
            }
            if input.key(KeyCode::Digit3) {
                self.percent = 100;
            }
            if input.key(KeyCode::KeyT) {
                self.repeat = !self.repeat;
                self.toast(if self.repeat { "repeat orders ON" } else { "repeat orders off" });
            }
            if input.key(KeyCode::KeyX) {
                let sel = self.selection.clone();
                if !sel.is_empty() {
                    self.cmds.push(Command::Cancel { faction: 0, sources: sel });
                    self.toast("standing orders cancelled");
                }
            }
            if input.key(KeyCode::KeyA) && input.ctrl() {
                self.selection = self.own_sun_ids();
            }
            if input.key(KeyCode::KeyF) || input.key(KeyCode::Home) {
                self.fit(view);
            }
            if input.key(KeyCode::Equal) || input.key(KeyCode::NumpadAdd) {
                self.speed = (self.speed * 2).min(8);
            }
            if input.key(KeyCode::Minus) || input.key(KeyCode::NumpadSubtract) {
                self.speed = (self.speed / 2).max(1);
            }
            // camera pan
            let pan = 900.0 / self.zoom * dt;
            let (mut dx, mut dy) = (0.0, 0.0);
            if input.held(KeyCode::ArrowLeft) || input.held(KeyCode::KeyA) && !input.ctrl() {
                dx -= pan;
            }
            if input.held(KeyCode::ArrowRight) || input.held(KeyCode::KeyD) {
                dx += pan;
            }
            if input.held(KeyCode::ArrowUp) || input.held(KeyCode::KeyW) {
                dy += pan;
            }
            if input.held(KeyCode::ArrowDown) || input.held(KeyCode::KeyS) {
                dy -= pan;
            }
            self.cam[0] += dx;
            self.cam[1] += dy;
        }

        // ---- mouse (world)
        let modal = self.overlay != Overlay::None || decided;
        let over_bar = self.in_bar(input.mouse, view);
        if !modal {
            // zoom around the cursor
            if input.wheel != 0.0 && !over_bar {
                self.zoom_at(input.mouse, 1.0 + 0.15 * input.wheel.clamp(-3.0, 3.0), view);
            }
            // touch: pinch zoom and finger pan
            if input.pinch > 0.0 {
                self.zoom_at(input.mouse, input.pinch, view);
            }
            if input.pan_px != (0.0, 0.0) {
                self.cam[0] -= input.pan_px.0 / self.zoom;
                self.cam[1] += input.pan_px.1 / self.zoom;
            }
            if input.cancel_drag {
                self.drag = None;
            }
            // middle-drag pans
            if input.middle_down {
                match self.pan {
                    None => self.pan = Some((input.mouse, self.cam)),
                    Some((m0, c0)) => {
                        self.cam[0] = c0[0] - (input.mouse.0 - m0.0) / self.zoom;
                        self.cam[1] = c0[1] + (input.mouse.1 - m0.1) / self.zoom;
                    }
                }
            } else {
                self.pan = None;
            }

            let wp = self.to_world(input.mouse, view);
            // fingers are fat: a more generous pick radius for touch
            let slack = ((if input.touch_mode { 22.0 } else { 6.0 }) / self.zoom) as f64;
            // right click = order
            if input.right_pressed && !over_bar {
                let t = match self.sim.pick(wp, slack) {
                    Some(i) => Target::Obj(i),
                    None => Target::Point(wp),
                };
                self.issue_order(t);
            }
            // left press begins a click / lasso
            if input.left_pressed && !over_bar {
                self.drag = Some(input.mouse);
            }
            if input.left_released {
                if let Some(start) = self.drag.take() {
                    let moved = (input.mouse.0 - start.0).abs() + (input.mouse.1 - start.1).abs();
                    if moved > input.drag_slop() {
                        // lasso: own suns inside the rectangle
                        let (x0, x1) = (start.0.min(input.mouse.0), start.0.max(input.mouse.0));
                        let (y0, y1) = (start.1.min(input.mouse.1), start.1.max(input.mouse.1));
                        let inside: Vec<usize> = self
                            .own_sun_ids()
                            .into_iter()
                            .filter(|&i| {
                                let p = self.to_screen(self.sim.sun(i).unwrap().pos, view);
                                p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
                            })
                            .collect();
                        if input.shift() {
                            for i in inside {
                                if !self.selection.contains(&i) {
                                    self.selection.push(i);
                                }
                            }
                        } else {
                            self.selection = inside;
                        }
                    } else {
                        let wp0 = self.to_world(start, view);
                        match self.sim.pick(wp0, slack) {
                            // touch has no right button: tapping a sun that is already selected sends the selection
                            // to it (the way to upgrade / reinforce a sun from its own stock)
                            Some(i) if input.touch_mode && self.selection.contains(&i) && self.sim.sun(i).map(|s| s.faction == 0 && s.present()).unwrap_or(false) => {
                                self.issue_order(Target::Obj(i))
                            }
                            Some(i) if self.sim.sun(i).map(|s| s.faction == 0 && s.present()).unwrap_or(false) => {
                                if input.shift() {
                                    if let Some(k) = self.selection.iter().position(|&x| x == i) {
                                        self.selection.remove(k);
                                    } else {
                                        self.selection.push(i);
                                    }
                                } else {
                                    self.selection = vec![i];
                                }
                            }
                            Some(i) if !self.selection.is_empty() => self.issue_order(Target::Obj(i)),
                            None if !self.selection.is_empty() => self.issue_order(Target::Point(wp0)),
                            _ => {}
                        }
                    }
                }
            }
        } else {
            self.drag = None;
        }

        // ---- simulation stepping
        self.selection.retain(|&i| self.sim.sun(i).map(|s| s.faction == 0 && s.present()).unwrap_or(false));
        if !self.paused && self.overlay == Overlay::None && self.sim.status == Status::Playing {
            self.acc += dt.min(0.1) * self.speed as f32;
            let tick = 1.0 / TICKS_PER_SEC as f32;
            let mut n = 0;
            while self.acc >= tick && n < 16 && self.sim.status == Status::Playing {
                let cmds = std::mem::take(&mut self.cmds);
                self.sim.step(&cmds);
                self.acc -= tick;
                n += 1;
                for ev in self.sim.take_events() {
                    match &ev {
                        Event::Combat { pos, a, .. } => {
                            if self.sparks.len() < 400 {
                                self.sparks.push(Spark { pos: *pos, age: 0.0, color_faction: *a });
                            }
                        }
                        Event::OrderRejected { faction: 0, why } => {
                            let m = match why {
                                OrderError::TooManyOrders => "too many orders at once - wait a moment",
                                OrderError::TooManyRepeats => "too many standing repeat orders",
                                OrderError::NoShips => "no ships to send",
                                OrderError::TooManySources => "too many suns in one order",
                                _ => "order not possible",
                            };
                            self.toast = Some((m.to_string(), 2.0));
                        }
                        _ => {}
                    }
                    self.events.push(ev);
                }
            }
            if self.acc > tick * 16.0 {
                self.acc = 0.0;
            }
        }
        // orders given while paused still apply when the game resumes (kept in `cmds`)
        let _ = Obj::Fog; // keep the import used when features change
    }

    pub fn seconds(&self) -> f32 {
        self.sim.tick as f32 / TICKS_PER_SEC as f32
    }
}
