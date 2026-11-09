use crate::{Host, Mode};
use serde::Deserialize;
use std::{
    sync::{atomic::Ordering, mpsc, Arc},
    time::Duration,
};
use tauri::WebviewWindow;

const MAX_EDGE: usize = 192;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitMask {
    columns: usize,
    rows: usize,
    bits: Vec<u32>,
    offset_x: f64,
    offset_y: f64,
}

impl HitMask {
    pub fn valid(&self) -> bool {
        self.columns <= MAX_EDGE
            && self.rows <= MAX_EDGE
            && (self.columns == 0) == (self.rows == 0)
            && self.bits.len() == (self.columns * self.rows).div_ceil(32)
            && (-1.0..=1.0).contains(&self.offset_x)
            && (-1.0..=1.0).contains(&self.offset_y)
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        let inside = |x| (0.0..1.0).contains(&x);
        if !inside(x) || !inside(y) || self.columns == 0 {
            return false;
        }
        let x = x - self.offset_x;
        let y = y - self.offset_y;
        if !inside(x) || !inside(y) {
            return false;
        }
        let column = (x * self.columns as f64) as usize;
        let row = (y * self.rows as f64) as usize;
        let pixel = row * self.columns + column;
        self.bits[pixel / 32] & (1 << (pixel % 32)) != 0
    }
}

// Use the live client area. On macOS Tao scales desktop cursor coordinates by
// the primary screen and the window origin by the window's own screen.
fn cursor_in_window(
    window: &WebviewWindow,
    cursor: tauri::PhysicalPosition<f64>,
) -> Option<(f64, f64)> {
    let origin = window.inner_position().ok()?;
    let size = window.inner_size().ok()?;
    let (x, y) = if cfg!(target_os = "macos") {
        let primary = window.primary_monitor().ok()??.scale_factor();
        let factor = window.scale_factor().ok()? / primary;
        (cursor.x * factor, cursor.y * factor)
    } else {
        (cursor.x, cursor.y)
    };
    Some((
        (x - f64::from(origin.x)) / f64::from(size.width),
        (y - f64::from(origin.y)) / f64::from(size.height),
    ))
}

pub fn start(window: WebviewWindow, host: Arc<Host>) {
    std::thread::Builder::new()
        .name("buddy-overlay-hit-test".into())
        .spawn(move || {
            // Only one poll can be queued at a time. The native cursor remains
            // observable while the WebView window passes through mouse events.
            let mut ignored = true;
            while !host.exiting.load(Ordering::Acquire) {
                let (sender, receiver) = mpsc::sync_channel(1);
                let overlay = window.clone();
                let host = host.clone();
                if window
                    .run_on_main_thread(move || {
                        let (mode, follow) = {
                            let controller = host.controller.lock().unwrap();
                            (
                                controller.mode,
                                controller.cursor_follow
                                    && !controller.paused
                                    && !controller.settings_open,
                            )
                        };
                        let button_down =
                            mode != Mode::Hidden && buddy_platform::mouse_button_down();
                        let menu_open = host.menu_open.load(Ordering::Acquire);
                        let cursor = if mode == Mode::Normal && !button_down && !menu_open {
                            overlay.cursor_position().ok()
                        } else {
                            None
                        };
                        host.track_follow_cursor(if follow { cursor } else { None });
                        let (mut next, delay) = if mode == Mode::Hidden {
                            (true, 80)
                        } else if mode == Mode::Editing || button_down || menu_open {
                            // Preserve whichever window received the press until
                            // release, even as the frame or the cursor moves.
                            (ignored, 8)
                        } else if let Some((x, y)) =
                            cursor.and_then(|cursor| cursor_in_window(&overlay, cursor))
                        {
                            let mask = host.hit_mask.lock().unwrap();
                            let nearby = (-0.2..1.2).contains(&x) && (-0.2..1.2).contains(&y);
                            (!mask.contains(x, y), if nearby { 8 } else { 16 })
                        } else {
                            (true, 16)
                        };
                        if next != ignored && overlay.set_ignore_cursor_events(next).is_err() {
                            next = ignored;
                        }
                        let _ = sender.send((next, delay));
                    })
                    .is_err()
                {
                    break;
                }
                let Ok((next, delay)) = receiver.recv() else {
                    break;
                };
                ignored = next;
                std::thread::sleep(Duration::from_millis(delay));
            }
        })
        .expect("overlay hit test worker creation");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_testing_tracks_the_canvas_at_the_work_area_edge() {
        let mask = HitMask {
            columns: 3,
            rows: 3,
            bits: vec![259],
            offset_x: -1.0 / 3.0,
            offset_y: 0.0,
        };
        assert!(mask.valid());
        assert!(mask.contains(0.1, 0.1));
        assert!(!mask.contains(0.4, 0.1));
        assert!(mask.contains(0.4, 0.8));
        assert!(!mask.contains(-0.1, 0.1));
    }
}
