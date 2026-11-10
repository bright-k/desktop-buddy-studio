use crate::{Host, Mode};
use serde::Deserialize;
use std::{
    sync::{atomic::Ordering, mpsc, Arc},
    time::Duration,
};
use tauri::WebviewWindow;

const MAX_EDGE: usize = 192;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitMask {
    columns: usize,
    rows: usize,
    bits: Vec<u32>,
    // Canvas origin and extent as fractions of the live client area.
    offset_x: f64,
    offset_y: f64,
    fraction_x: f64,
    fraction_y: f64,
}

impl Default for HitMask {
    fn default() -> Self {
        Self {
            columns: 0,
            rows: 0,
            bits: Vec::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            fraction_x: 1.0,
            fraction_y: 1.0,
        }
    }
}

impl HitMask {
    pub fn valid(&self) -> bool {
        self.columns <= MAX_EDGE
            && self.rows <= MAX_EDGE
            && (self.columns == 0) == (self.rows == 0)
            && self.bits.len() == (self.columns * self.rows).div_ceil(32)
            && (-1.0..=1.0).contains(&self.offset_x)
            && (-1.0..=1.0).contains(&self.offset_y)
            && self.fraction_x > 0.0
            && self.fraction_x <= 1.0
            && self.fraction_y > 0.0
            && self.fraction_y <= 1.0
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        let inside = |x| (0.0..1.0).contains(&x);
        if !inside(x) || !inside(y) || self.columns == 0 {
            return false;
        }
        let x = (x - self.offset_x) / self.fraction_x;
        let y = (y - self.offset_y) / self.fraction_y;
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
                let unavailable = host.controller.lock().unwrap().mode == Mode::Hidden
                    || !host.overlay_ready.load(Ordering::Acquire);
                if unavailable {
                    host.track_follow_cursor(None);
                    if ignored {
                        host.hit_test_wake.wait(None);
                        continue;
                    }
                }
                let (sender, receiver) = mpsc::sync_channel(1);
                let overlay = window.clone();
                let poll_host = host.clone();
                if window
                    .run_on_main_thread(move || {
                        let host = poll_host;
                        let (mode, follow) = {
                            let controller = host.controller.lock().unwrap();
                            (
                                controller.mode,
                                controller.cursor_follow
                                    && !controller.paused
                                    && !controller.settings_open
                                    && !host.quick_timer.lock().unwrap().is_open(),
                            )
                        };
                        let available =
                            mode != Mode::Hidden && host.overlay_ready.load(Ordering::Acquire);
                        let button_down = available && buddy_platform::mouse_button_down();
                        let menu_open = host.menu_open.load(Ordering::Acquire);
                        let cursor =
                            if available && mode == Mode::Normal && !button_down && !menu_open {
                                overlay.cursor_position().ok()
                            } else {
                                None
                            };
                        host.track_follow_cursor(if follow { cursor } else { None });
                        let (mut next, delay) = if !available {
                            (true, 0)
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
                host.hit_test_wake
                    .wait((!unavailable).then_some(Duration::from_millis(delay)));
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
            fraction_x: 1.0,
            fraction_y: 1.0,
        };
        assert!(mask.valid());
        assert!(mask.contains(0.1, 0.1));
        assert!(!mask.contains(0.4, 0.1));
        assert!(mask.contains(0.4, 0.8));
        assert!(!mask.contains(-0.1, 0.1));
    }

    #[test]
    fn hit_testing_excludes_extra_space_in_a_mismatched_viewport() {
        let mask = HitMask {
            columns: 3,
            rows: 3,
            bits: vec![259],
            offset_x: -1.0 / 6.0,
            offset_y: 0.0,
            fraction_x: 0.5,
            fraction_y: 1.0,
        };
        assert!(mask.valid());
        assert!(mask.contains(0.1, 0.1));
        assert!(mask.contains(0.25, 0.8));
        assert!(!mask.contains(0.4, 0.8));
    }
}
