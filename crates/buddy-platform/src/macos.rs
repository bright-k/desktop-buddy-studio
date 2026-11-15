use crate::{
    queue::EventSender, CaptureState, InputOptions, InputStatus, OverlayMouseRoute, PermissionState,
};
use buddy_project::{MouseButton, WheelDirection};
use buddy_runtime::InputEvent;
use std::{
    collections::BTreeSet,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

type CFRef = *const c_void;
type EventRef = *mut c_void;
static ACCESS_REQUESTED_OR_REVOKED: AtomicBool = AtomicBool::new(false);
const SESSION_EVENT_TAP: u32 = 1;
const ANNOTATED_SESSION_EVENT_TAP: u32 = 2;
const EVENT_TARGET_PID: u32 = 40;
#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}
type Callback = unsafe extern "C" fn(*mut c_void, u32, EventRef, *mut c_void) -> EventRef;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: Callback,
        info: *mut c_void,
    ) -> CFRef;
    fn CGEventTapEnable(tap: CFRef, enable: bool);
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
    fn CGEventGetLocation(event: EventRef) -> CGPoint;
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGSessionCopyCurrentDictionary() -> CFRef;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFRef;
    static kCFBooleanTrue: CFRef;
    fn CFStringCreateWithCString(
        allocator: CFRef,
        string: *const std::ffi::c_char,
        encoding: u32,
    ) -> CFRef;
    fn CFDictionaryGetValue(dictionary: CFRef, key: CFRef) -> CFRef;
    fn CFMachPortCreateRunLoopSource(allocator: CFRef, port: CFRef, order: isize) -> CFRef;
    fn CFRunLoopGetCurrent() -> CFRef;
    fn CFRunLoopAddSource(run_loop: CFRef, source: CFRef, mode: CFRef);
    fn CFRunLoopRemoveSource(run_loop: CFRef, source: CFRef, mode: CFRef);
    fn CFRunLoopRunInMode(mode: CFRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFMachPortInvalidate(port: CFRef);
    fn CFRelease(object: CFRef);
}

pub(crate) fn desktop_available() -> bool {
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return false;
        }
        // The lock screen may still have an awake display and an active console session.
        let key = CFStringCreateWithCString(
            ptr::null(),
            c"CGSSessionScreenIsLocked".as_ptr(),
            0x0800_0100,
        );
        let available = !key.is_null()
            && CFDictionaryGetValue(session, key) != kCFBooleanTrue
            && CGDisplayIsAsleep(CGMainDisplayID()) == 0;
        if !key.is_null() {
            CFRelease(key);
        }
        CFRelease(session);
        available
    }
}
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

pub fn permission_state() -> PermissionState {
    if unsafe { CGPreflightListenEventAccess() } {
        PermissionState::Granted
    } else {
        if ACCESS_REQUESTED_OR_REVOKED.load(Ordering::Acquire) {
            PermissionState::Denied
        } else {
            PermissionState::NeedsRequest
        }
    }
}
pub fn request_permission() -> bool {
    ACCESS_REQUESTED_OR_REVOKED.store(true, Ordering::Release);
    unsafe { CGRequestListenEventAccess() }
}

pub(crate) fn mouse_button_down() -> bool {
    // kCGEventSourceStateCombinedSessionState includes the current user's devices.
    (0..5).any(|button| unsafe { CGEventSourceButtonState(0, button) })
}

struct Context {
    sender: EventSender,
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u32>,
    overlay_mouse: OverlayMouseRoute,
    last_motion: Instant,
    tap: CFRef,
    interrupted: bool,
    queue_epoch: u64,
}
impl Context {
    unsafe fn rearm(&mut self) {
        self.overlay_mouse.reset();
        self.keys = (0..128)
            .filter(|key| key_code(*key).is_some() && CGEventSourceKeyState(0, *key))
            .collect();
        self.buttons = (0..3)
            .filter(|button| CGEventSourceButtonState(0, *button))
            .collect();
        self.queue_epoch = self.sender.epoch();
        self.sender.send(InputEvent::Rearm {
            keys: self
                .keys
                .iter()
                .filter_map(|key| key_code(*key).map(str::to_owned))
                .collect(),
            buttons: self
                .buttons
                .iter()
                .map(|raw| match raw {
                    0 => MouseButton::Primary,
                    1 => MouseButton::Secondary,
                    _ => MouseButton::Middle,
                })
                .filter(|button| !self.overlay_mouse.routes(*button))
                .collect(),
        });
        self.interrupted = false;
    }
}
unsafe extern "C" fn callback(
    _: *mut c_void,
    kind: u32,
    event: EventRef,
    info: *mut c_void,
) -> EventRef {
    let received_at = Instant::now();
    if info.is_null() {
        return event;
    }
    let context = &mut *(info as *mut Context);
    if kind >= 0xffff_fffe {
        context.sender.reset();
        context.keys.clear();
        context.buttons.clear();
        context.interrupted = true;
        if !context.tap.is_null() {
            CGEventTapEnable(context.tap, true);
        }
        return event;
    }
    if event.is_null() || context.interrupted || context.queue_epoch != context.sender.epoch() {
        return event;
    }
    if matches!(kind, 10..=12) && IsSecureEventInputEnabled() != 0 {
        return event;
    }
    match kind {
        10..=12 => {
            let key = CGEventGetIntegerValueField(event, 9) as u16;
            let code = key_code(key).unwrap_or("Unknown").to_string();
            let pressed = if kind == 12 {
                CGEventSourceKeyState(0, key)
            } else {
                kind == 10
            };
            if pressed {
                let repeat =
                    CGEventGetIntegerValueField(event, 8) != 0 || context.keys.contains(&key);
                context.keys.insert(key);
                context
                    .sender
                    .send_at(InputEvent::KeyDown { code, repeat }, received_at);
            } else {
                context.keys.remove(&key);
                context
                    .sender
                    .send_at(InputEvent::KeyUp { code }, received_at);
            }
        }
        1..=4 | 25 | 26 => {
            let raw = match kind {
                1 | 2 => 0,
                3 | 4 => 1,
                _ => CGEventGetIntegerValueField(event, 3) as u32,
            };
            let button = match raw {
                0 => MouseButton::Primary,
                1 => MouseButton::Secondary,
                2 => MouseButton::Middle,
                _ => return event,
            };
            let pressed = matches!(kind, 1 | 3 | 25);
            if pressed {
                context.buttons.insert(raw);
            } else {
                context.buttons.remove(&raw);
            }
            if context.overlay_mouse.routes(button) {
                // Global capture pauses for the settings window. While it is
                // active, a routed press delivered to this PID is the overlay's.
                let target =
                    pressed.then(|| CGEventGetIntegerValueField(event, EVENT_TARGET_PID) as usize);
                if !context.overlay_mouse.forward(button, pressed, target) {
                    return event;
                }
            }
            context.sender.send_at(
                if pressed {
                    InputEvent::MouseDown { button }
                } else {
                    InputEvent::MouseUp { button }
                },
                received_at,
            );
        }
        5 | 6 | 7 | 27 => {
            if context.last_motion.elapsed() >= Duration::from_millis(34) {
                context.last_motion = Instant::now();
                let point = CGEventGetLocation(event);
                context.sender.send_at(
                    InputEvent::CursorMove {
                        x: point.x,
                        y: point.y,
                    },
                    received_at,
                );
            }
        }
        22 => {
            let x = CGEventGetIntegerValueField(event, 12);
            let y = CGEventGetIntegerValueField(event, 11);
            let scroll = match (x, y) {
                (0, 0) => None,
                (x, 0) => Some(InputEvent::Wheel {
                    direction: if x > 0 {
                        WheelDirection::Left
                    } else {
                        WheelDirection::Right
                    },
                }),
                (0, y) => Some(InputEvent::Wheel {
                    direction: if y > 0 {
                        WheelDirection::Up
                    } else {
                        WheelDirection::Down
                    },
                }),
                (x, y) => Some(InputEvent::DiagonalWheel {
                    left: x > 0,
                    up: y > 0,
                }),
            };
            if let Some(scroll) = scroll {
                context.sender.send_at(scroll, received_at);
            }
        }
        _ => {}
    }
    event // listen-only: never replace or suppress the original event.
}

pub(crate) fn run(
    options: InputOptions,
    sender: EventSender,
    status: Arc<Mutex<InputStatus>>,
    stop: Arc<AtomicBool>,
    ready: SyncSender<()>,
    targets: Vec<usize>,
) {
    if !options.keyboard && !options.mouse {
        let _ = ready.send(());
        return;
    }
    if !unsafe { CGPreflightListenEventAccess() } {
        *status.lock().unwrap() = options.status(
            permission_state(),
            CaptureState::Stopped,
            Some("Input Monitoring permission has not been granted".into()),
        );
        let _ = ready.send(());
        return;
    }
    let mut kinds = Vec::new();
    if options.keyboard {
        kinds.extend([10, 11, 12]);
    }
    if options.mouse {
        kinds.extend([1, 2, 3, 4, 5, 6, 7, 22, 25, 26, 27]);
    }
    let mask = kinds
        .into_iter()
        .fold(0u64, |mask, kind| mask | (1 << kind));
    let routed = !targets.is_empty();
    let mut context = Box::new(Context {
        sender,
        keys: BTreeSet::new(),
        buttons: BTreeSet::new(),
        overlay_mouse: OverlayMouseRoute::new(targets),
        last_motion: Instant::now() - Duration::from_secs(1),
        tap: ptr::null(),
        interrupted: false,
        queue_epoch: 0,
    });
    unsafe {
        let tap = CGEventTapCreate(
            if routed && options.mouse {
                ANNOTATED_SESSION_EVENT_TAP
            } else {
                SESSION_EVENT_TAP
            },
            0,
            1,
            mask,
            callback,
            (&mut *context as *mut Context).cast(),
        );
        if tap.is_null() {
            *status.lock().unwrap() = options.status(
                permission_state(),
                CaptureState::Failed,
                Some("listen-only session event tap could not be created".into()),
            );
            let _ = ready.send(());
            return;
        }
        context.tap = tap;
        let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
        if source.is_null() {
            CFMachPortInvalidate(tap);
            CFRelease(tap);
            *status.lock().unwrap() = options.status(
                permission_state(),
                CaptureState::Failed,
                Some("event tap run-loop source could not be created".into()),
            );
            let _ = ready.send(());
            return;
        }
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        context.rearm();
        CGEventTapEnable(tap, true);
        *status.lock().unwrap() =
            options.status(PermissionState::Granted, CaptureState::Running, None);
        let _ = ready.send(());
        let mut previous = Instant::now();
        let mut secure = false;
        while !stop.load(Ordering::Acquire) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, false);
            let now_secure = IsSecureEventInputEnabled() != 0;
            let granted = CGPreflightListenEventAccess();
            if previous.elapsed() > Duration::from_secs(1)
                || now_secure != secure
                || context.interrupted
                || context.queue_epoch != context.sender.epoch()
                || !granted
            {
                context.sender.reset();
                context.rearm();
            }
            if !granted {
                ACCESS_REQUESTED_OR_REVOKED.store(true, Ordering::Release);
                *status.lock().unwrap() = options.status(
                    PermissionState::Denied,
                    CaptureState::Suspended,
                    Some("Input Monitoring permission was revoked".into()),
                );
                break;
            }
            if now_secure != secure {
                secure = now_secure;
                let mut current =
                    options.status(PermissionState::Granted, CaptureState::Running, None);
                if secure && options.keyboard {
                    current.keyboard.capture = CaptureState::Suspended;
                    current.keyboard.reason =
                        Some("보안 입력으로 키보드 감지가 중지됐습니다.".into());
                }
                *status.lock().unwrap() = current;
            }
            if !secure {
                let released: Vec<_> = context
                    .keys
                    .iter()
                    .copied()
                    .filter(|key| !CGEventSourceKeyState(0, *key))
                    .collect();
                for key in released {
                    context.keys.remove(&key);
                    if let Some(code) = key_code(key) {
                        context.sender.send(InputEvent::KeyUp { code: code.into() });
                    }
                }
            }
            {
                let released: Vec<_> = context
                    .buttons
                    .iter()
                    .copied()
                    .filter(|button| !CGEventSourceButtonState(0, *button))
                    .collect();
                for raw in released {
                    context.buttons.remove(&raw);
                    let button = match raw {
                        0 => MouseButton::Primary,
                        1 => MouseButton::Secondary,
                        _ => MouseButton::Middle,
                    };
                    if context.overlay_mouse.forward(button, false, None) {
                        context.sender.send(InputEvent::MouseUp { button });
                    }
                }
            }
            previous = Instant::now();
        }
        CGEventTapEnable(tap, false);
        CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
        CFMachPortInvalidate(tap);
        CFRelease(source);
        CFRelease(tap);
    }
}

fn key_code(key: u16) -> Option<&'static str> {
    Some(match key {
        0 => "KeyA",
        1 => "KeyS",
        2 => "KeyD",
        3 => "KeyF",
        4 => "KeyH",
        5 => "KeyG",
        6 => "KeyZ",
        7 => "KeyX",
        8 => "KeyC",
        9 => "KeyV",
        11 => "KeyB",
        12 => "KeyQ",
        13 => "KeyW",
        14 => "KeyE",
        15 => "KeyR",
        16 => "KeyY",
        17 => "KeyT",
        18 => "Digit1",
        19 => "Digit2",
        20 => "Digit3",
        21 => "Digit4",
        22 => "Digit6",
        23 => "Digit5",
        24 => "Equal",
        25 => "Digit9",
        26 => "Digit7",
        27 => "Minus",
        28 => "Digit8",
        29 => "Digit0",
        30 => "BracketRight",
        31 => "KeyO",
        32 => "KeyU",
        33 => "BracketLeft",
        34 => "KeyI",
        35 => "KeyP",
        36 => "Enter",
        37 => "KeyL",
        38 => "KeyJ",
        39 => "Quote",
        40 => "KeyK",
        41 => "Semicolon",
        42 => "Backslash",
        43 => "Comma",
        44 => "Slash",
        45 => "KeyN",
        46 => "KeyM",
        47 => "Period",
        48 => "Tab",
        49 => "Space",
        50 => "Backquote",
        51 => "Backspace",
        53 => "Escape",
        54 => "MetaRight",
        55 => "MetaLeft",
        56 => "ShiftLeft",
        58 => "AltLeft",
        59 => "ControlLeft",
        60 => "ShiftRight",
        61 => "AltRight",
        62 => "ControlRight",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        99 => "F3",
        100 => "F8",
        101 => "F9",
        103 => "F11",
        109 => "F10",
        111 => "F12",
        115 => "Home",
        116 => "PageUp",
        117 => "Delete",
        118 => "F4",
        119 => "End",
        120 => "F2",
        121 => "PageDown",
        122 => "F1",
        123 => "ArrowLeft",
        124 => "ArrowRight",
        125 => "ArrowDown",
        126 => "ArrowUp",
        _ => return None,
    })
}
