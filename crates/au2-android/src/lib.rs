//! Android entry point. NativeActivity loads this library and calls `android_main`; everything else is `au2_app`.
//!
//! Levels are read from the APK's asset manager (`levels/levels.txt`, `levels/<stem>.au2`), which `cargo-apk` packs
//! from `apk-assets/` (staged by `tools/build-android.sh`). On every other target this crate is empty.

#![cfg(target_os = "android")]

use au2_app::{App, TextSource};
use std::ffi::CString;
use std::io::Read;
use winit::event_loop::EventLoop;
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::EventLoopBuilderExtAndroid;

fn read_asset(app: &AndroidApp, path: &str) -> Result<String, String> {
    let c = CString::new(path).map_err(|e| e.to_string())?;
    let mut asset = app.asset_manager().open(&c).ok_or_else(|| format!("asset '{path}' not found in the APK"))?;
    let mut text = String::new();
    asset.read_to_string(&mut text).map_err(|e| format!("asset '{path}': {e}"))?;
    Ok(text)
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("aurora"));
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));

    let assets = app.clone();
    let source = TextSource { read: move |p: &str| read_asset(&assets, p) };
    let game = match App::new(Box::new(source), None, None, true) {
        Ok(g) => g,
        Err(e) => {
            log::error!("cannot load levels: {e}");
            return;
        }
    };
    let event_loop = EventLoop::builder().with_android_app(app).build().expect("event loop");
    if let Err(e) = au2_app::run(event_loop, game) {
        log::error!("event loop ended with an error: {e}");
    }
}
