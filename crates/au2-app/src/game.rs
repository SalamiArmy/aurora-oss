//! Game session state and input handling (no drawing here).
//!
//! Orders select **ships by area**: draw a circle (lasso) or tap a sun to select the area around it, then tap a target.
//! The selection is a set of ship ids (circling, hovering and flying ships alike); it is cleared once an order is sent.

use crate::view3d::{Camera, V3};
use au2_map::{LevelEntry, Map, Object};
use au2_sim::math::Vec2;
use au2_sim::rules::TICKS_PER_SEC;
use au2_sim::{Command, Event, OrderError, Sim, SimConfig, Status, Target};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub const BAR_H: f32 = 44.0;
/// Height of the info strip at the top that `fit` keeps the board out of.
pub const HUD_TOP: f32 = 76.0;

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

/// A supergiant (or any sun) dying: rays and debris in the owner's colour.
pub struct Burst {
    pub pos: Vec2,
    pub age: f32,
    pub faction: i8,
    pub big: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Pause,
}

/// Game speeds: each roughly twice the previous; normal is the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    Slow,
    Normal,
    Fast,
}

impl Speed {
    /// Simulation steps per real second.
    pub fn ticks_per_sec(self) -> f32 {
        match self {
            Speed::Slow => TICKS_PER_SEC as f32 / 2.0,
            Speed::Normal => TICKS_PER_SEC as f32,
            Speed::Fast => TICKS_PER_SEC as f32 * 2.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Speed::Slow => "Slow",
            Speed::Normal => "Normal",
            Speed::Fast => "Fast",
        }
    }
    pub fn next(self) -> Speed {
        match self {
            Speed::Slow => Speed::Normal,
            Speed::Normal => Speed::Fast,
            Speed::Fast => Speed::Slow,
        }
    }
    pub fn faster(self) -> Speed {
        if self == Speed::Slow { Speed::Normal } else { Speed::Fast }
    }
    pub fn slower(self) -> Speed {
        if self == Speed::Fast { Speed::Normal } else { Speed::Slow }
    }
}

/// Smallest-effort circle for a drawn lasso: centroid of the path and the mean distance of its points.
pub fn lasso_circle(path: &[Vec2]) -> Option<(Vec2, f64)> {
    if path.len() < 3 {
        return None;
    }
    let n = path.len() as f64;
    let c = path.iter().fold(Vec2::ZERO, |a, &p| a + p) * (1.0 / n);
    let r = path.iter().map(|p| p.dist(c)).sum::<f64>() / n;
    Some((c, r))
}

pub struct Game {
    pub entry: LevelEntry,
    pub level_index: usize,
    pub map: Map,
    pub sim: Sim,
    pub seed: u64,
    pub cam: [f32; 2],
    pub zoom: f32,
    /// Selected ship ids (sorted).
    pub selection: Vec<u32>,
    pub speed: Speed,
    pub paused: bool,
    pub overlay: Overlay,
    pub sparks: Vec<Spark>,
    pub pings: Vec<Ping>,
    pub bursts: Vec<Burst>,
    pub toast: Option<(String, f32)>,
    /// Screen position where the current press began.
    pub drag: Option<(f32, f32)>,
    /// The lasso being drawn (world points).
    pub lasso: Vec<Vec2>,
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
            speed: Speed::Normal,
            paused: false,
            overlay: Overlay::None,
            sparks: Vec::new(),
            pings: Vec::new(),
            bursts: Vec::new(),
            toast: None,
            drag: None,
            lasso: Vec::new(),
            pan: None,
            acc: 0.0,
            clock: 0.0,
            cmds: Vec::new(),
            events: Vec::new(),
            orders_sent: 0,
            bar_h: BAR_H,
        };
        g.fit(view);
        g.flash_all_suns();
        g
    }

    /// All suns flash when a level starts.
    fn flash_all_suns(&mut self) {
        let ps: Vec<Vec2> = self.sim.suns().map(|(_, s)| s.pos).collect();
        self.pings.extend(ps.into_iter().map(|pos| Ping { pos, age: 0.0 }));
    }

    pub fn restart(&mut self) {
        self.sim = Sim::new(&self.map, cfg(self.seed, &self.entry));
        self.selection.clear();
        self.sparks.clear();
        self.pings.clear();
        self.bursts.clear();
        self.cmds.clear();
        self.lasso.clear();
        self.drag = None;
        self.paused = false;
        self.overlay = Overlay::None;
        self.acc = 0.0;
        self.orders_sent = 0;
        self.flash_all_suns();
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
        // perspective: find the largest zoom at which the board's four corners are on screen, and centre their
        // projection (the near edge is drawn bigger than the far one, so the look-at point sits a little off-centre)
        let (vw, vh) = (view.0.max(100.0), (view.1 - self.bar_h).max(100.0));
        self.cam = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        let fits = |g: &Game, zoom: f32| -> Option<(f32, f32)> {
            let c = Camera::new(g.cam[0], g.cam[1], zoom, vw / 2.0, vh / 2.0, vh);
            let (mut top, mut bottom) = (f32::MAX, f32::MIN);
            for &(x, y) in &corners {
                let p = c.project(V3::new(x, y, 0.0))?;
                if p.x < vw * 0.03 || p.x > vw * 0.97 {
                    return None;
                }
                top = top.min(p.y);
                bottom = bottom.max(p.y);
            }
            // leave the top HUD strip clear
            (bottom - top <= vh * 0.97 - HUD_TOP).then_some((top, bottom))
        };
        for _ in 0..3 {
            let (mut lo, mut hi) = (0.0005f32, 2.0f32);
            for _ in 0..40 {
                let mid = (lo * hi).sqrt();
                if fits(self, mid).is_some() {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            self.zoom = lo;
            // shift the look-at point so the projected board is vertically centred
            if let Some((top, bottom)) = fits(self, lo) {
                let mid = (top + bottom) / 2.0 - (HUD_TOP + vh * 0.01) / 2.0;
                let c = self.camera(view);
                if let Some(w) = c.unproject(vw / 2.0, mid, 0.0) {
                    self.cam[1] += w.y - self.cam[1];
                }
            }
        }
    }

    /// The perspective camera for this frame.
    pub fn camera(&self, view: (f32, f32)) -> Camera {
        let vh = (view.1 - self.bar_h).max(1.0);
        Camera::new(self.cam[0], self.cam[1], self.zoom, view.0 / 2.0, vh / 2.0, vh)
    }

    /// Screen position of a ground point (off-screen far away if it is behind the camera).
    pub fn to_screen(&self, p: Vec2, view: (f32, f32)) -> (f32, f32) {
        match self.camera(view).project(V3::new(p.x as f32, p.y as f32, 0.0)) {
            Some(q) => (q.x, q.y),
            None => (-1.0e5, -1.0e5),
        }
    }

    /// Screen pixels per world unit at a ground point.
    pub fn scale_at(&self, p: Vec2, view: (f32, f32)) -> f32 {
        self.camera(view).project(V3::new(p.x as f32, p.y as f32, 0.0)).map(|q| q.scale).unwrap_or(0.0)
    }

    /// The ground point under a screen position.
    pub fn to_world(&self, s: (f32, f32), view: (f32, f32)) -> Vec2 {
        let c = self.camera(view);
        // above the horizon: clamp to just below it
        let w = c.unproject(s.0, s.1, 0.0).or_else(|| c.unproject(s.0, c.cy - c.f * 0.4, 0.0)).unwrap_or(V3::new(self.cam[0], self.cam[1], 0.0));
        Vec2::new(w.x as f64, w.y as f64)
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
        self.zoom = (self.zoom * factor).clamp(0.005, 0.6);
        for _ in 0..2 {
            let after = self.to_world(at, view);
            self.cam[0] += (before.x - after.x) as f32;
            self.cam[1] += (before.y - after.y) as f32;
        }
    }

    /// Move the camera so the ground point under screen `from` ends up under screen `to` (drag panning).
    pub fn drag_pan(&mut self, from: (f32, f32), to: (f32, f32), view: (f32, f32)) {
        let (a, b) = (self.to_world(from, view), self.to_world(to, view));
        self.cam[0] += (a.x - b.x) as f32;
        self.cam[1] += (a.y - b.y) as f32;
    }

    /// Is this ship id selected? (`selection` is sorted.)
    pub fn is_selected(&self, id: u32) -> bool {
        self.selection.binary_search(&id).is_ok()
    }

    /// Select (or, with `add`, add) the player's ships in a circle.
    pub fn select_circle(&mut self, c: Vec2, r: f64, add: bool) {
        let ids = self.sim.ships_in_circle(0, c, r);
        if add {
            self.selection.extend(ids);
            self.selection.sort_unstable();
            self.selection.dedup();
        } else {
            self.selection = ids;
        }
        if self.selection.is_empty() {
            self.toast("no ships of yours there");
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self.sim.all_ships(0);
    }

    pub fn issue_order(&mut self, target: Target) {
        if self.selection.is_empty() {
            return;
        }
        let ships = std::mem::take(&mut self.selection);
        self.cmds.push(Command::Order { faction: 0, ships, target });
        self.orders_sent += 1;
        if let Some(p) = self.sim.target_pos(target) {
            self.pings.push(Ping { pos: p, age: 0.0 });
        }
    }

    /// A click / tap at world point `wp` (no drag).
    fn click(&mut self, wp: Vec2, slack: f64, shift: bool) {
        let picked = self.sim.pick(wp, slack);
        if self.selection.is_empty() || shift {
            // select the area around a sun (of any owner)
            if let Some(i) = picked.filter(|&i| self.sim.sun(i).is_some()) {
                let (c, r) = (self.sim.sun(i).unwrap().pos, self.sim.sun_select_radius(i));
                self.select_circle(c, r, shift);
            }
            return;
        }
        match picked {
            Some(i) => self.issue_order(Target::Obj(i)),
            None => self.issue_order(Target::Point(wp)),
        }
    }

    /// Process one frame of input and advance the simulation.
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
        for b in self.bursts.iter_mut() {
            b.age += dt;
        }
        self.bursts.retain(|b| b.age < 1.6);
        if let Some((_, t)) = &mut self.toast {
            *t -= dt;
            if *t <= 0.0 {
                self.toast = None;
            }
        }

        // ---- keyboard (always available)
        if input.key(KeyCode::Escape) {
            if self.overlay == Overlay::None && (!self.selection.is_empty() || !self.lasso.is_empty()) {
                self.selection.clear();
                self.lasso.clear();
                self.drag = None;
            } else {
                self.overlay = if self.overlay == Overlay::Pause { Overlay::None } else { Overlay::Pause };
            }
        }
        let decided = self.sim.status != Status::Playing;
        if self.overlay == Overlay::None {
            if input.key(KeyCode::Space) || input.key(KeyCode::KeyP) {
                self.paused = !self.paused;
            }
            if input.key(KeyCode::KeyA) && input.ctrl() {
                self.select_all();
            }
            if input.key(KeyCode::KeyF) || input.key(KeyCode::Home) {
                self.fit(view);
            }
            if input.key(KeyCode::Equal) || input.key(KeyCode::NumpadAdd) {
                self.speed = self.speed.faster();
            }
            if input.key(KeyCode::Minus) || input.key(KeyCode::NumpadSubtract) {
                self.speed = self.speed.slower();
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

        // ---- mouse / touch (world)
        let modal = self.overlay != Overlay::None || decided;
        let over_bar = self.in_bar(input.mouse, view);
        if !modal {
            if input.wheel != 0.0 && !over_bar {
                self.zoom_at(input.mouse, 1.0 + 0.15 * input.wheel.clamp(-3.0, 3.0), view);
            }
            if input.pinch > 0.0 {
                self.zoom_at(input.mouse, input.pinch, view);
            }
            if input.pan_px != (0.0, 0.0) {
                let to = input.mouse;
                self.drag_pan((to.0 - input.pan_px.0, to.1 - input.pan_px.1), to, view);
            }
            if input.cancel_drag {
                self.drag = None;
                self.lasso.clear();
            }
            if input.middle_down {
                match self.pan {
                    None => self.pan = Some((input.mouse, self.cam)),
                    Some((m0, c0)) => {
                        self.cam = c0;
                        self.drag_pan(m0, input.mouse, view);
                    }
                }
            } else {
                self.pan = None;
            }

            let wp = self.to_world(input.mouse, view);
            let k = self.scale_at(wp, view).max(1e-4);
            let slack = ((if input.touch_mode { 22.0 } else { 6.0 }) / k) as f64;
            // right click = order (or nothing without a selection)
            if input.right_pressed && !over_bar {
                if self.selection.is_empty() {
                    self.click(wp, slack, false);
                } else {
                    let t = match self.sim.pick(wp, slack) {
                        Some(i) => Target::Obj(i),
                        None => Target::Point(wp),
                    };
                    self.issue_order(t);
                }
            }
            // left press begins a click or a lasso
            if input.left_pressed && !over_bar {
                self.drag = Some(input.mouse);
                self.lasso = vec![wp];
            }
            if self.drag.is_some() && input.left_down {
                if self.lasso.last().map(|l| l.dist(wp) * k as f64 > 4.0).unwrap_or(true) {
                    self.lasso.push(wp);
                }
            }
            if input.left_released {
                if let Some(start) = self.drag.take() {
                    // how far the drag ever got from where it began (a closed lasso ends where it started)
                    let path = std::mem::take(&mut self.lasso);
                    let reach = path.iter().map(|p| p.dist(path[0])).fold(0.0f64, f64::max) * k as f64;
                    let moved = ((input.mouse.0 - start.0).abs() + (input.mouse.1 - start.1).abs()).max(reach as f32);
                    if moved > input.drag_slop() {
                        let mut path = path;
                        path.push(wp);
                        if let Some((c, r)) = lasso_circle(&path) {
                            self.select_circle(c, r, input.shift());
                        }
                    } else {
                        let wp0 = self.to_world(start, view);
                        self.click(wp0, slack, input.shift());
                    }
                }
            }
        } else {
            self.drag = None;
            self.lasso.clear();
        }

        // ---- simulation stepping (the speed scales how many fixed steps run per real second)
        if !self.paused && self.overlay == Overlay::None && self.sim.status == Status::Playing {
            self.acc += dt.min(0.1) * self.speed.ticks_per_sec();
            let mut n = 0;
            while self.acc >= 1.0 && n < 16 && self.sim.status == Status::Playing {
                let cmds = std::mem::take(&mut self.cmds);
                self.sim.step(&cmds);
                self.acc -= 1.0;
                n += 1;
                let events = self.sim.take_events();
                self.handle_events(events);
            }
            if self.acc > 16.0 {
                self.acc = 0.0;
            }
            // ships that died drop out of the selection
            if !self.selection.is_empty() {
                let sim = &self.sim;
                self.selection.retain(|&id| sim.ship_index(id).is_some());
            }
        }
        // orders given while paused still apply when the game resumes (kept in `cmds`)
    }

    pub fn handle_events(&mut self, events: Vec<Event>) {
        for ev in events {
            match &ev {
                Event::Combat { pos, a, .. } => {
                    if self.sparks.len() < 400 {
                        self.sparks.push(Spark { pos: *pos, age: 0.0, color_faction: *a });
                    }
                }
                Event::SunDied { sun, faction, level } => {
                    if let Some(s) = self.sim.sun(*sun) {
                        self.bursts.push(Burst { pos: s.pos, age: 0.0, faction: *faction, big: *level >= 4 });
                    }
                }
                Event::OrderRejected { faction: 0, why } => {
                    let m = match why {
                        OrderError::Fogged => "you cannot see there",
                        OrderError::NoShips => "no ships selected",
                        OrderError::BadTarget => "not a valid target",
                        _ => "order not possible",
                    };
                    self.toast = Some((m.to_string(), 2.0));
                }
                _ => {}
            }
            self.events.push(ev);
        }
    }

    pub fn seconds(&self) -> f32 {
        self.sim.tick as f32 / TICKS_PER_SEC as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawn_lasso_becomes_its_circle() {
        let path: Vec<Vec2> = (0..36).map(|k| Vec2::new(100.0, 50.0) + au2_sim::math::dir_deg(k as f64 * 10.0) * 300.0).collect();
        let (c, r) = lasso_circle(&path).unwrap();
        assert!(c.dist(Vec2::new(100.0, 50.0)) < 1.0 && (r - 300.0).abs() < 1.0);
        assert!(lasso_circle(&path[..2]).is_none());
    }

    fn level(stem: &str) -> Game {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
        let e = au2_map::load_index(&dir).unwrap().into_iter().find(|e| e.stem == stem).unwrap();
        let map = au2_map::load_level(&dir, &e).unwrap();
        Game::new(e, 0, map, 1, (1280.0, 800.0))
    }

    /// One frame of input: press / move / release at screen points.
    fn frame(g: &mut Game, f: impl FnOnce(&mut Input)) {
        let mut i = Input::default();
        f(&mut i);
        g.update(&i, 0.0, (1280.0, 800.0));
    }

    #[test]
    fn lasso_selects_ships_and_a_click_sends_them() {
        let view = (1280.0, 800.0);
        let mut g = level("dawn_twin");
        let home = g.own_sun_ids()[0];
        let c = g.to_screen(g.sim.sun(home).unwrap().pos, view);
        let r = 70.0; // screen px, more than the cloud at the fitted zoom
        frame(&mut g, |i| {
            i.mouse = (c.0 + r, c.1);
            i.left_pressed = true;
            i.left_down = true;
        });
        for k in 1..=24 {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            frame(&mut g, |i| {
                i.mouse = (c.0 + r * a.cos(), c.1 + r * a.sin());
                i.left_down = true;
            });
        }
        frame(&mut g, |i| {
            i.mouse = (c.0 + r, c.1);
            i.left_released = true;
        });
        assert_eq!(g.selection.len(), 100, "the whole starting cloud");
        // click a neutral sun: the order is queued and the selection cleared
        let target = g.sim.suns().find(|(_, s)| s.faction < 0).map(|(i, _)| i).unwrap();
        let t = g.to_screen(g.sim.sun(target).unwrap().pos, view);
        frame(&mut g, |i| {
            i.mouse = t;
            i.left_pressed = true;
            i.left_released = true;
        });
        assert!(g.selection.is_empty());
        assert!(matches!(&g.cmds[..], [Command::Order { ships, target: Target::Obj(x), .. }] if ships.len() == 100 && *x == target));
    }

    #[test]
    fn clicking_a_sun_with_nothing_selected_selects_its_area() {
        let view = (1280.0, 800.0);
        let mut g = level("dawn_twin");
        let home = g.own_sun_ids()[0];
        let c = g.to_screen(g.sim.sun(home).unwrap().pos, view);
        frame(&mut g, |i| {
            i.mouse = c;
            i.left_pressed = true;
            i.left_released = true;
        });
        assert_eq!(g.selection.len(), 100);
        assert!(g.cmds.is_empty());
    }

    #[test]
    fn speeds_are_each_about_twice_the_previous() {
        assert_eq!(Speed::Normal.ticks_per_sec(), 60.0);
        assert_eq!(Speed::Slow.ticks_per_sec() * 2.0, Speed::Normal.ticks_per_sec());
        assert_eq!(Speed::Fast.ticks_per_sec(), Speed::Normal.ticks_per_sec() * 2.0);
    }
}
