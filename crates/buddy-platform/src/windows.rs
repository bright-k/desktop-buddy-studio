use crate::{
    queue::EventSender, CaptureState, InputOptions, InputStatus, OverlayMouseRoute, PermissionState,
};
use buddy_project::{MouseButton, WheelDirection};
use buddy_runtime::InputEvent;
use std::{
    collections::BTreeMap,
    mem::size_of,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, RemoteDesktop::*},
    UI::{
        Input::{
            KeyboardAndMouse::{GetAsyncKeyState, MapVirtualKeyW, MAPVK_VSC_TO_VK_EX},
            *,
        },
        WindowsAndMessaging::*,
    },
};

struct Context {
    sender: EventSender,
    devices: BTreeMap<usize, BTreeMap<String, u16>>,
    stop: Arc<AtomicBool>,
    last_motion: Instant,
    previous: Instant,
    locked: bool,
    options: InputOptions,
    status: Arc<Mutex<InputStatus>>,
    rearm_keys: BTreeMap<String, u16>,
    overlay_mouse: OverlayMouseRoute,
    queue_epoch: u64,
}
impl Context {
    unsafe fn rearm(&mut self) {
        self.overlay_mouse.reset();
        self.devices.clear();
        self.rearm_keys.clear();
        for scan in 0..128 {
            for flags in [0, 2] {
                if let Some(code) = scan_code(scan, flags) {
                    let raw = u32::from(scan) | if flags == 2 { 0xe000 } else { 0 };
                    let vk = MapVirtualKeyW(raw, MAPVK_VSC_TO_VK_EX) as u16;
                    if vk != 0 && GetAsyncKeyState(i32::from(vk)) < 0 {
                        self.rearm_keys.insert(code.into(), vk);
                    }
                }
            }
        }
        let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
        let buttons = [
            (
                1,
                if swapped {
                    MouseButton::Secondary
                } else {
                    MouseButton::Primary
                },
            ),
            (
                2,
                if swapped {
                    MouseButton::Primary
                } else {
                    MouseButton::Secondary
                },
            ),
            (4, MouseButton::Middle),
        ]
        .into_iter()
        .filter(|(_, button)| !self.overlay_mouse.routes(*button))
        .filter_map(|(vk, button)| (GetAsyncKeyState(vk) < 0).then_some(button))
        .collect();
        self.queue_epoch = self.sender.epoch();
        self.sender.send(InputEvent::Rearm {
            keys: self.rearm_keys.keys().cloned().collect(),
            buttons,
        });
    }
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let received_at = Instant::now();
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
    if !pointer.is_null() {
        let context = &mut *pointer;
        match message {
            WM_INPUT if !context.locked && context.queue_epoch == context.sender.epoch() => {
                let mut size = 0u32;
                if GetRawInputData(
                    lparam as HRAWINPUT,
                    RID_INPUT,
                    ptr::null_mut(),
                    &mut size,
                    size_of::<RAWINPUTHEADER>() as u32,
                ) == u32::MAX
                    || size > 65536
                    || size < (size_of::<RAWINPUTHEADER>() as u32)
                {
                    context.sender.reset();
                } else {
                    let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
                    let length = GetRawInputData(
                        lparam as HRAWINPUT,
                        RID_INPUT,
                        storage.as_mut_ptr().cast(),
                        &mut size,
                        size_of::<RAWINPUTHEADER>() as u32,
                    );
                    if length != u32::MAX && length as usize <= storage.len() * size_of::<usize>() {
                        let bytes = std::slice::from_raw_parts(
                            storage.as_ptr().cast::<u8>(),
                            length as usize,
                        );
                        match parse_packet(bytes) {
                            Some(RawPacket::Keyboard { device, key }) => {
                                keyboard(context, device, key, received_at)
                            }
                            Some(RawPacket::Mouse(data)) => mouse(context, data, received_at),
                            None => context.sender.reset(),
                        }
                    } else {
                        context.sender.reset();
                    }
                }
            }
            WM_INPUT_DEVICE_CHANGE if wparam == 2 => {
                context.devices.remove(&(lparam as usize));
                context.sender.reset();
                context.rearm();
            }
            WM_WTSSESSION_CHANGE if wparam == 7 || wparam == 8 => {
                context.locked = wparam == 7;
                context.devices.clear();
                context.sender.reset();
                *context.status.lock().unwrap() = context.options.status(
                    PermissionState::NotRequired,
                    if context.locked {
                        CaptureState::Suspended
                    } else {
                        CaptureState::Running
                    },
                    context
                        .locked
                        .then(|| "사용자 세션이 잠겨 있습니다.".into()),
                );
                if !context.locked {
                    context.rearm();
                }
            }
            WM_TIMER => {
                if context.stop.load(Ordering::Acquire) {
                    PostQuitMessage(0);
                }
                if context.previous.elapsed() > Duration::from_secs(1) {
                    context.devices.clear();
                    context.sender.reset();
                }
                if context.queue_epoch != context.sender.epoch() && !context.locked {
                    context.rearm();
                }
                context.previous = Instant::now();
                let held: BTreeMap<String, u16> = context
                    .devices
                    .values()
                    .flat_map(|keys| keys.iter().map(|(code, vk)| (code.clone(), *vk)))
                    .chain(
                        context
                            .rearm_keys
                            .iter()
                            .map(|(code, vk)| (code.clone(), *vk)),
                    )
                    .collect();
                for (code, vk) in held {
                    if GetAsyncKeyState(i32::from(vk)) >= 0 {
                        for keys in context.devices.values_mut() {
                            keys.remove(&code);
                        }
                        context.rearm_keys.remove(&code);
                        context.sender.send(InputEvent::KeyUp { code });
                    }
                }
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}
unsafe fn keyboard(context: &mut Context, device: usize, key: RAWKEYBOARD, received_at: Instant) {
    if key.VKey == 255 {
        return;
    }
    let code = scan_code(key.MakeCode, key.Flags)
        .unwrap_or("Unknown")
        .to_string();
    let pressed = key.Flags & 1 == 0;
    let keys = context.devices.entry(device).or_default();
    if pressed {
        let repeat = keys.contains_key(&code);
        let virtual_key = match code.as_str() {
            "ShiftLeft" => 0xa0,
            "ShiftRight" => 0xa1,
            "ControlLeft" => 0xa2,
            "ControlRight" => 0xa3,
            "AltLeft" => 0xa4,
            "AltRight" => 0xa5,
            _ => key.VKey,
        };
        keys.insert(code.clone(), virtual_key);
        context
            .sender
            .send_at(InputEvent::KeyDown { code, repeat }, received_at);
    } else {
        keys.remove(&code);
        if !context
            .devices
            .values()
            .any(|keys| keys.contains_key(&code))
        {
            context.rearm_keys.remove(&code);
            context
                .sender
                .send_at(InputEvent::KeyUp { code }, received_at);
        }
    }
}
unsafe fn mouse(context: &mut Context, mouse: RAWMOUSE, received_at: Instant) {
    let data = mouse.Anonymous.Anonymous;
    let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
    for (mask, pressed, physical) in [
        (1, true, 0),
        (2, false, 0),
        (4, true, 1),
        (8, false, 1),
        (16, true, 2),
        (32, false, 2),
    ] {
        if data.usButtonFlags & mask != 0 {
            let button = match (physical, swapped) {
                (0, false) | (1, true) => MouseButton::Primary,
                (0, true) | (1, false) => MouseButton::Secondary,
                _ => MouseButton::Middle,
            };
            if context.overlay_mouse.routes(button) {
                let target = if pressed {
                    let position = GetMessagePos();
                    let point = POINT {
                        x: i32::from(position as i16),
                        y: i32::from((position >> 16) as i16),
                    };
                    let hit = WindowFromPoint(point);
                    (!hit.is_null()).then(|| GetAncestor(hit, GA_ROOT) as usize)
                } else {
                    None
                };
                if !context.overlay_mouse.forward(button, pressed, target) {
                    continue;
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
    }
    let delta = data.usButtonData as i16;
    if data.usButtonFlags & 0x800 != 0 && delta != 0 {
        context.sender.send_at(
            InputEvent::Wheel {
                direction: if delta > 0 {
                    WheelDirection::Right
                } else {
                    WheelDirection::Left
                },
            },
            received_at,
        );
    }
    if data.usButtonFlags & 0x400 != 0 && delta != 0 {
        context.sender.send_at(
            InputEvent::Wheel {
                direction: if delta > 0 {
                    WheelDirection::Up
                } else {
                    WheelDirection::Down
                },
            },
            received_at,
        );
    }
    if (mouse.lLastX != 0 || mouse.lLastY != 0)
        && context.last_motion.elapsed() >= Duration::from_millis(34)
    {
        context.last_motion = Instant::now();
        let mut point = POINT::default();
        if GetCursorPos(&mut point) != 0 {
            context.sender.send_at(
                InputEvent::CursorMove {
                    x: point.x as f64,
                    y: point.y as f64,
                },
                received_at,
            );
        }
    }
}
pub(crate) fn mouse_button_down() -> bool {
    [1, 2, 4, 5, 6]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(key) < 0 })
}

pub(crate) fn run(
    options: InputOptions,
    sender: EventSender,
    status: Arc<Mutex<InputStatus>>,
    stop: Arc<AtomicBool>,
    ready: SyncSender<()>,
    overlay: Option<usize>,
) {
    if !options.keyboard && !options.mouse {
        let _ = ready.send(());
        return;
    }
    let mut context = Box::new(Context {
        sender,
        devices: BTreeMap::new(),
        stop,
        last_motion: Instant::now() - Duration::from_secs(1),
        previous: Instant::now(),
        locked: false,
        options,
        status: status.clone(),
        rearm_keys: BTreeMap::new(),
        overlay_mouse: OverlayMouseRoute::new(overlay),
        queue_epoch: 0,
    });
    unsafe {
        let class: Vec<u16> = "DesktopBuddyRawInput\0".encode_utf16().collect();
        let instance = GetModuleHandleW(ptr::null());
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        RegisterClassW(&definition);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            (&mut *context as *mut Context).cast(),
        );
        if window.is_null() {
            *status.lock().unwrap() = options.status(
                PermissionState::NotRequired,
                CaptureState::Failed,
                Some("Raw Input message window creation failed".into()),
            );
            let _ = ready.send(());
            return;
        }
        let usages: Vec<u16> = [(options.keyboard, 6), (options.mouse, 2)]
            .into_iter()
            .filter_map(|(enabled, usage)| enabled.then_some(usage))
            .collect();
        let devices: Vec<_> = usages
            .iter()
            .map(|usage| RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: *usage,
                dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
                hwndTarget: window,
            })
            .collect();
        if RegisterRawInputDevices(
            devices.as_ptr(),
            devices.len() as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        ) == 0
            || SetTimer(window, 1, 250, None) == 0
        {
            *status.lock().unwrap() = options.status(
                PermissionState::NotRequired,
                CaptureState::Failed,
                Some("Raw Input registration or watchdog timer failed".into()),
            );
            DestroyWindow(window);
            let _ = ready.send(());
            return;
        }
        WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION);
        context.rearm();
        *status.lock().unwrap() =
            options.status(PermissionState::NotRequired, CaptureState::Running, None);
        let _ = ready.send(());
        let mut message = MSG::default();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let removals: Vec<_> = usages
            .iter()
            .map(|usage| RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: *usage,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: ptr::null_mut(),
            })
            .collect();
        RegisterRawInputDevices(
            removals.as_ptr(),
            removals.len() as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        );
        WTSUnRegisterSessionNotification(window);
        KillTimer(window, 1);
        DestroyWindow(window);
    }
}

enum RawPacket {
    Keyboard { device: usize, key: RAWKEYBOARD },
    Mouse(RAWMOUSE),
}
fn parse_packet(bytes: &[u8]) -> Option<RawPacket> {
    let offset = std::mem::offset_of!(RAWINPUT, data);
    if bytes.len() < size_of::<RAWINPUTHEADER>() {
        return None;
    }
    // A keyboard packet is smaller than the RAWINPUT union (40 vs 48 bytes on
    // x64). Never form a full RAWINPUT reference over a keyboard-sized buffer.
    let header = unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<RAWINPUTHEADER>()) };
    if header.dwSize as usize > bytes.len() {
        return None;
    }
    match header.dwType {
        RIM_TYPEKEYBOARD if bytes.len() >= offset + size_of::<RAWKEYBOARD>() => {
            Some(RawPacket::Keyboard {
                device: header.hDevice as usize,
                key: unsafe {
                    ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<RAWKEYBOARD>())
                },
            })
        }
        RIM_TYPEMOUSE if bytes.len() >= offset + size_of::<RAWMOUSE>() => {
            Some(RawPacket::Mouse(unsafe {
                ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<RAWMOUSE>())
            }))
        }
        _ => None,
    }
}

fn scan_code(scan: u16, flags: u16) -> Option<&'static str> {
    if flags & 4 != 0 {
        return None;
    }
    if flags & 2 != 0 {
        return Some(match scan {
            0x1d => "ControlRight",
            0x38 => "AltRight",
            0x47 => "Home",
            0x48 => "ArrowUp",
            0x49 => "PageUp",
            0x4b => "ArrowLeft",
            0x4d => "ArrowRight",
            0x4f => "End",
            0x50 => "ArrowDown",
            0x51 => "PageDown",
            0x53 => "Delete",
            0x5b => "MetaLeft",
            0x5c => "MetaRight",
            _ => return None,
        });
    }
    Some(match scan {
        0x01 => "Escape",
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0a => "Digit9",
        0x0b => "Digit0",
        0x0c => "Minus",
        0x0d => "Equal",
        0x0e => "Backspace",
        0x0f => "Tab",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1a => "BracketLeft",
        0x1b => "BracketRight",
        0x1c => "Enter",
        0x1d => "ControlLeft",
        0x1e => "KeyA",
        0x1f => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2a => "ShiftLeft",
        0x2b => "Backslash",
        0x2c => "KeyZ",
        0x2d => "KeyX",
        0x2e => "KeyC",
        0x2f => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x36 => "ShiftRight",
        0x38 => "AltLeft",
        0x39 => "Space",
        0x3b => "F1",
        0x3c => "F2",
        0x3d => "F3",
        0x3e => "F4",
        0x3f => "F5",
        0x40 => "F6",
        0x41 => "F7",
        0x42 => "F8",
        0x43 => "F9",
        0x44 => "F10",
        0x57 => "F11",
        0x58 => "F12",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_sized_packet_is_valid_and_truncation_is_rejected() {
        let offset = std::mem::offset_of!(RAWINPUT, data);
        let mut bytes = vec![0u8; offset + size_of::<RAWKEYBOARD>()];
        let header = RAWINPUTHEADER {
            dwType: RIM_TYPEKEYBOARD,
            dwSize: bytes.len() as u32,
            ..Default::default()
        };
        let keyboard = RAWKEYBOARD {
            MakeCode: 0x1e,
            VKey: b'A' as u16,
            ..Default::default()
        };
        unsafe {
            ptr::write_unaligned(bytes.as_mut_ptr().cast::<RAWINPUTHEADER>(), header);
            ptr::write_unaligned(
                bytes.as_mut_ptr().add(offset).cast::<RAWKEYBOARD>(),
                keyboard,
            );
        }
        assert!(bytes.len() < size_of::<RAWINPUT>());
        assert!(
            matches!(parse_packet(&bytes), Some(RawPacket::Keyboard { key, .. }) if key.MakeCode == 0x1e)
        );
        assert!(parse_packet(&bytes[..bytes.len() - 1]).is_none());
    }
}
