//! Getting the software canvas onto the window. Desktop: softbuffer (nearest-neighbour upscale by an integer factor).
//! Android: straight into the window's `ANativeWindow` buffer; the buffer is the canvas size and the compositor scales
//! it to the screen, so no CPU upscaling happens there.

use crate::gfx::Canvas;
use std::rc::Rc;
use winit::window::Window;

#[cfg(not(target_os = "android"))]
pub struct Presenter {
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

#[cfg(not(target_os = "android"))]
impl Presenter {
    pub fn new(win: &Rc<Window>) -> Result<Presenter, String> {
        let ctx = softbuffer::Context::new(win.clone()).map_err(|e| e.to_string())?;
        let surface = softbuffer::Surface::new(&ctx, win.clone()).map_err(|e| e.to_string())?;
        Ok(Presenter { surface })
    }

    /// `pw` x `ph` is the window's physical size; the canvas is drawn `scale` times larger.
    pub fn present(&mut self, canvas: &Canvas, pw: i32, ph: i32, scale: i32) {
        use std::num::NonZeroU32;
        let (Some(a), Some(b)) = (NonZeroU32::new(pw as u32), NonZeroU32::new(ph as u32)) else { return };
        if self.surface.resize(a, b).is_err() {
            return;
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        let s = scale.max(1) as usize;
        let (cw, ch) = (canvas.w as usize, canvas.h as usize);
        if s == 1 {
            if buf.len() == canvas.buf.len() {
                buf.copy_from_slice(&canvas.buf);
            }
        } else if buf.len() == (pw * ph) as usize {
            let pw = pw as usize;
            for y in 0..ph as usize {
                let src = &canvas.buf[(y / s).min(ch - 1) * cw..][..cw];
                let dst = &mut buf[y * pw..][..pw];
                for (x, d) in dst.iter_mut().enumerate() {
                    *d = src[(x / s).min(cw - 1)];
                }
            }
        }
        let _ = buf.present();
    }
}

#[cfg(target_os = "android")]
pub struct Presenter {
    window: ndk::native_window::NativeWindow,
    size: (i32, i32),
}

#[cfg(target_os = "android")]
impl Presenter {
    pub fn new(win: &Rc<Window>) -> Result<Presenter, String> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = win.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::AndroidNdk(h) = handle.as_raw() else { return Err("not an Android NDK window".into()) };
        // SAFETY: the pointer is a live ANativeWindow owned by the activity; `clone_from_ptr` takes its own reference.
        let window = unsafe { ndk::native_window::NativeWindow::clone_from_ptr(h.a_native_window.cast()) };
        Ok(Presenter { window, size: (0, 0) })
    }

    pub fn present(&mut self, canvas: &Canvas, _pw: i32, _ph: i32, _scale: i32) {
        use ndk::hardware_buffer_format::HardwareBufferFormat;
        if self.size != (canvas.w, canvas.h) {
            // RGBX/RGBA 8888 buffer of canvas size; the system scales it to the window
            if self.window.set_buffers_geometry(canvas.w, canvas.h, Some(HardwareBufferFormat::R8G8B8A8_UNORM)).is_err() {
                return;
            }
            self.size = (canvas.w, canvas.h);
        }
        let Ok(mut guard) = self.window.lock(None) else { return };
        let (w, h, stride) = (guard.width() as usize, guard.height() as usize, guard.stride() as usize);
        let Some(bytes) = guard.bytes() else { return };
        if w != canvas.w as usize || h != canvas.h as usize || bytes.len() < stride * h * 4 {
            return;
        }
        for y in 0..h {
            let src = &canvas.buf[y * w..][..w];
            let dst = &mut bytes[y * stride * 4..][..w * 4];
            for (x, &p) in src.iter().enumerate() {
                dst[x * 4].write((p >> 16) as u8);
                dst[x * 4 + 1].write((p >> 8) as u8);
                dst[x * 4 + 2].write(p as u8);
                dst[x * 4 + 3].write(255);
            }
        }
        // dropping the guard unlocks and posts the buffer
    }
}
