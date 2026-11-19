//! Aurora RTS front-end library: game session, software renderer, HUD and the winit application. The desktop binary
//! (`main.rs`) and the Android entry crate (`au2-android`) both drive this.

pub mod app;
pub mod draw3d;
pub mod game;
pub mod gfx;
pub mod png;
pub mod present;
pub mod render;
pub mod source;
pub mod touch;
pub mod view3d;

pub use app::{run, App};
pub use source::{FsSource, LevelSource, TextSource};
