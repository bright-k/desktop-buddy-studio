#[cfg(target_os = "macos")]
mod macos;
mod queue;
mod wake;
pub use wake::WakeSignal;
#[cfg(target_os = "windows")]
mod windows;

use buddy_runtime::InputEvent;
pub use buddy_runtime::{CaptureState, InputSourceStatus, InputStatus, PermissionState};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::Instant,
};

/// Native callback receipt time, retained only in the bounded input queue.
pub struct CapturedInput {
    pub event: InputEvent,
    pub received_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct InputOptions {
    pub keyboard: bool,
    pub mouse: bool,
}
impl Default for InputOptions {
    fn default() -> Self {
        Self {
            keyboard: true,
            mouse: true,
        }
    }
}
impl InputOptions {
    pub fn status(
        self,
        permission: PermissionState,
        capture: CaptureState,
        reason: Option<String>,
    ) -> InputStatus {
        let source = |enabled| InputSourceStatus {
            permission,
            capture: if enabled {
                capture
            } else {
                CaptureState::Stopped
            },
            reason: if enabled {
                reason.clone()
            } else {
                Some("사용자 설정에서 감지를 껐습니다.".into())
            },
        };
        InputStatus {
            keyboard: source(self.keyboard),
            mouse: source(self.mouse),
        }
    }
}

pub struct InputSession {
    receiver: queue::EventReceiver,
    status: Arc<Mutex<InputStatus>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

/// Each overlay button belongs to the window that received the press. After
/// capture restarts, an unmatched release cannot create a global reaction.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
struct OverlayMouseRoute {
    overlay: Option<usize>,
    global_down: [bool; 2],
}
#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl OverlayMouseRoute {
    fn new(overlay: Option<usize>) -> Self {
        Self {
            overlay,
            global_down: [false; 2],
        }
    }

    fn reset(&mut self) {
        self.global_down = [false; 2];
    }

    fn routes(&self, button: buddy_project::MouseButton) -> bool {
        self.overlay.is_some() && button != buddy_project::MouseButton::Middle
    }

    fn forward(
        &mut self,
        button: buddy_project::MouseButton,
        pressed: bool,
        target: Option<usize>,
    ) -> bool {
        if !self.routes(button) {
            return true;
        }
        let index = usize::from(button == buddy_project::MouseButton::Secondary);
        if pressed {
            self.global_down[index] = target != self.overlay;
            self.global_down[index]
        } else {
            std::mem::take(&mut self.global_down[index])
        }
    }
}

impl InputSession {
    pub fn start(options: InputOptions) -> Result<Self, String> {
        Self::start_with_signal(options, WakeSignal::default())
    }
    pub fn start_with_signal(options: InputOptions, signal: WakeSignal) -> Result<Self, String> {
        Self::start_routed(options, signal, None)
    }
    /// `overlay` is the Player PID on macOS and the overlay HWND on Windows.
    pub fn start_for_overlay(
        options: InputOptions,
        signal: WakeSignal,
        overlay: usize,
    ) -> Result<Self, String> {
        Self::start_routed(options, signal, (overlay != 0).then_some(overlay))
    }
    fn start_routed(
        options: InputOptions,
        signal: WakeSignal,
        overlay: Option<usize>,
    ) -> Result<Self, String> {
        let (sender, receiver) = queue::bounded_with_signal(1024, signal);
        let status = Arc::new(Mutex::new(options.status(
            permission_state(),
            CaptureState::Stopped,
            None,
        )));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_status = status.clone();
        let worker_stop = stop.clone();
        let worker = std::thread::Builder::new()
            .name("buddy-native-input".into())
            .spawn(move || {
                #[cfg(target_os = "macos")]
                macos::run(
                    options,
                    sender,
                    worker_status,
                    worker_stop,
                    ready_tx,
                    overlay,
                );
                #[cfg(target_os = "windows")]
                windows::run(
                    options,
                    sender,
                    worker_status,
                    worker_stop,
                    ready_tx,
                    overlay,
                );
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    let _ = (options, sender, worker_status, worker_stop, overlay);
                    let _ = ready_tx.send(());
                }
            })
            .map_err(|e| e.to_string())?;
        if ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_err()
        {
            stop.store(true, Ordering::Release);
            return Err("native input startup timed out".into());
        }
        Ok(Self {
            receiver,
            status,
            stop,
            worker: Some(worker),
        })
    }
    pub fn status(&self) -> InputStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn poll(&mut self) -> Option<InputEvent> {
        self.poll_captured().map(|captured| captured.event)
    }
    pub fn poll_captured(&mut self) -> Option<CapturedInput> {
        self.receiver.poll_captured()
    }
}
impl Drop for InputSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub fn permission_state() -> PermissionState {
    #[cfg(target_os = "macos")]
    {
        macos::permission_state()
    }
    #[cfg(target_os = "windows")]
    {
        PermissionState::NotRequired
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PermissionState::Denied
    }
}
/// Query button state without starting the global input capture session.
pub fn mouse_button_down() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::mouse_button_down()
    }
    #[cfg(target_os = "windows")]
    {
        windows::mouse_button_down()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}
/// Must only be called from an explicit user request (settings or CLI flag).
pub fn request_input_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::request_permission()
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[cfg(test)]
mod overlay_mouse_tests {
    use super::OverlayMouseRoute;
    use buddy_project::MouseButton::{Middle, Primary, Secondary};

    #[test]
    fn the_press_target_owns_the_release_even_across_window_edges() {
        let mut route = OverlayMouseRoute::new(Some(10));
        assert!(!route.forward(Primary, true, Some(10)));
        assert!(route.forward(Secondary, true, Some(20)));
        assert!(!route.forward(Primary, false, Some(20)));
        assert!(route.forward(Secondary, false, Some(10)));

        assert!(!route.forward(Secondary, true, Some(10)));
        assert!(!route.forward(Secondary, false, Some(20)));
        assert!(route.forward(Middle, true, Some(10)));

        assert!(route.forward(Secondary, true, Some(20)));
        route.reset();
        assert!(!route.forward(Secondary, false, Some(20)));
    }
}
