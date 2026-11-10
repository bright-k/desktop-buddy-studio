use crate::{
    display::Display,
    require,
    timer::{self, Action, SurfaceSide, TimerView},
    timer_menu::PRESET_PREFIX,
    Host, Mode,
};
use buddy_project::{Point, TimerMode};
use serde::{Deserialize, Serialize};
use std::sync::{atomic::Ordering, Arc};
use tauri::{AppHandle, Emitter, Manager, State as Managed, WebviewWindow};

pub const LABEL: &str = "timer-quick";
const SIZE: Point = Point { x: 260.0, y: 198.0 };

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Content {
    Custom {
        seconds: u32,
    },
    Confirm {
        title: &'static str,
        detail: String,
        label: &'static str,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Popup {
    id: u32,
    content: Content,
    side: SurfaceSide,
    tail_x: f64,
}

#[derive(Default)]
pub struct State {
    current: Option<(Popup, Option<Action>)>,
    sequence: u32,
    pub closing: bool,
}

impl State {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub fn closed(&mut self) {
        self.current = None;
        self.closing = false;
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Decision {
    Dismiss,
    Start {
        seconds: u32,
        replace_revision: Option<u32>,
    },
    Confirm,
}

fn create_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App("index.html".into()))
        .on_navigation(crate::navigation::allow_player_navigation)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .initialization_script("window.location.hash = 'timer-quick';")
        .title("버디 타이머 빠른 메뉴")
        .inner_size(SIZE.x, SIZE.y)
        .decorations(false)
        .transparent(true)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible_on_all_workspaces(cfg!(target_os = "macos"))
        .shadow(false)
        .focused(false)
        .visible(false)
        .build()
}

fn open(
    app: &AppHandle,
    host: &Host,
    content: Content,
    action: Option<Action>,
) -> Result<(), String> {
    let window = app
        .get_webview_window(LABEL)
        .map_or_else(|| create_window(app), Ok)
        .map_err(|error| error.to_string())?;
    let (display, origin, size, side, tail_x) = placement(host);
    let popup = {
        let mut state = host.quick_timer.lock().unwrap();
        if state.closing {
            return Err("타이머 팝업을 닫고 있습니다. 다시 열어 주세요.".into());
        }
        state.sequence = state.sequence.wrapping_add(1);
        let popup = Popup {
            id: state.sequence,
            content,
            side,
            tail_x,
        };
        state.current = Some((popup.clone(), action));
        popup
    };
    host.track_follow_cursor(None);
    host.wake.wake();
    let result = position(&window, &display, origin, size)
        .and_then(|_| window.show())
        .and_then(|_| window.set_focus())
        .and_then(|_| window.emit("timer-quick", &popup));
    if result.is_err() {
        let _ = dismiss(&window, host, Some(popup.id));
    }
    result.map_err(|error| error.to_string())
}

fn placement(host: &Host) -> (Display, Point, Point, SurfaceSide, f64) {
    let c = host.controller.lock().unwrap().clone();
    let state = host.runtime.lock().unwrap().state();
    let display = host.display.lock().unwrap().clone();
    let size = display.fit_size(host.loaded.canvas_size(c.scale));
    let popup_size = Point {
        x: SIZE.x.min(display.bounds.width),
        y: SIZE.y.min(display.bounds.height),
    };
    let footprint = host.render_footprint(&state);
    let canvas = host.render_position(&display, size, &state);
    let near_buddy = c.mode != Mode::Hidden && host.overlay_ready.load(Ordering::Acquire);
    let placement = near_buddy
        .then(|| {
            timer::adjacent_position(display.bounds, canvas, size, footprint, popup_size, true)
        })
        .flatten();
    let (origin, side, tail_x) = if let Some(placement) = placement {
        let center = footprint.map_or(0.5, |(min, max)| (min.x + max.x) / 2.0);
        let inset = 24.0_f64.min(popup_size.x / 2.0);
        let tail_x =
            (canvas.x + center * size.x - placement.origin.x).clamp(inset, popup_size.x - inset);
        (placement.origin, placement.side, tail_x)
    } else {
        let bounds = display.bounds;
        let origin = bounds.clamp(
            bounds.x + bounds.width - popup_size.x - 8.0,
            bounds.y + 8.0,
            popup_size,
        );
        (origin, SurfaceSide::Side, popup_size.x / 2.0)
    };
    (display, origin, popup_size, side, tail_x)
}

fn position(
    window: &WebviewWindow,
    display: &Display,
    origin: Point,
    size: Point,
) -> tauri::Result<()> {
    if cfg!(target_os = "windows") {
        if window.outer_position().ok() != Some(display.physical_position(origin)) {
            display.position_window(window, origin)?;
        }
        display.sync_window_size(window, size)
    } else {
        display.sync_window_size(window, size)?;
        display.position_window(window, origin)
    }
}

pub fn refit(app: &AppHandle, host: &Host) {
    if !host.quick_timer.lock().unwrap().is_open() {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let (display, origin, size, side, tail_x) = placement(host);
    if position(&window, &display, origin, size).is_err() {
        return;
    }
    let popup = host
        .quick_timer
        .lock()
        .unwrap()
        .current
        .as_mut()
        .and_then(|(popup, _)| {
            if popup.side == side && popup.tail_x == tail_x {
                return None;
            }
            popup.side = side;
            popup.tail_x = tail_x;
            Some(popup.clone())
        });
    if let Some(popup) = popup {
        let _ = window.emit("timer-quick", &popup);
    }
}

pub fn dismiss(window: &WebviewWindow, host: &Host, id: Option<u32>) -> Result<(), String> {
    let mut state = host.quick_timer.lock().unwrap();
    if state
        .current
        .as_ref()
        .is_some_and(|(popup, _)| id.is_none_or(|id| id == popup.id))
    {
        window.destroy().map_err(|error| error.to_string())?;
        state.current = None;
        state.closing = true;
        host.wake.wake();
    }
    Ok(())
}

fn start(
    app: &AppHandle,
    host: &Host,
    view: &TimerView,
    mode: TimerMode,
    seconds: Option<u32>,
) -> Result<(), String> {
    let replace_revision = view.active_mode().map(|_| view.revision);
    let action = Action::Start {
        mode,
        seconds,
        replace_revision,
    };
    if replace_revision.is_none() {
        return host.apply_timer_action(action).map(|_| ());
    }
    let label = match mode {
        TimerMode::Countdown => format!(
            "일반 타이머 {}",
            timer::menu_time(seconds.unwrap_or(view.countdown_seconds()))
        ),
        TimerMode::Pomodoro => format!("집중 {}", timer::menu_time(view.focus_seconds())),
    };
    open(
        app,
        host,
        Content::Confirm {
            title: "타이머를 바꿀까요?",
            detail: format!("현재 타이머를 종료합니다. 새 타이머 · {label}"),
            label: "종료하고 시작",
        },
        Some(action),
    )
}

pub fn handle_menu(app: &AppHandle, host: &Host, id: &str, revision: u32) -> Result<(), String> {
    let view = host.timer_view()?;
    if view.storage_blocked {
        return host.open_timer(app);
    }
    if matches!(id, "timer-pomodoro" | "timer-toggle" | "timer-end") && view.revision != revision {
        return Err("타이머 상태가 바뀌었습니다. 메뉴를 다시 열어 주세요.".into());
    }
    match id {
        "timer-custom" => open(
            app,
            host,
            Content::Custom {
                seconds: view.countdown_seconds(),
            },
            None,
        ),
        "timer-pomodoro" if view.active_mode() != Some(TimerMode::Pomodoro) => {
            start(app, host, &view, TimerMode::Pomodoro, None)
        }
        "timer-pomodoro" | "timer-toggle" => host
            .apply_timer_action(Action::Toggle { revision })
            .map(|_| ()),
        "timer-end" => {
            let Some(mode) = view.active_mode() else {
                return Ok(());
            };
            let (title, detail) = if mode == TimerMode::Pomodoro {
                (
                    "뽀모도로를 종료할까요?",
                    "이번 세션의 집중 횟수가 초기화됩니다.",
                )
            } else {
                ("타이머를 종료할까요?", "남은 시간이 사라집니다.")
            };
            open(
                app,
                host,
                Content::Confirm {
                    title,
                    detail: detail.into(),
                    label: "종료",
                },
                Some(Action::End { revision }),
            )
        }
        _ => {
            let seconds = id
                .strip_prefix(PRESET_PREFIX)
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|&seconds| view.has_preset(seconds))
                .ok_or("바로가기 목록이 바뀌었습니다. 메뉴를 다시 열어 주세요.")?;
            start(app, host, &view, TimerMode::Countdown, Some(seconds))
        }
    }
}

#[tauri::command]
pub fn get_timer_quick(
    window: WebviewWindow,
    host: Managed<'_, Arc<Host>>,
) -> Result<Option<Popup>, String> {
    require(&window, LABEL)?;
    Ok(host
        .quick_timer
        .lock()
        .unwrap()
        .current
        .as_ref()
        .map(|(popup, _)| popup.clone()))
}

#[tauri::command]
pub fn finish_timer_quick(
    window: WebviewWindow,
    host: Managed<'_, Arc<Host>>,
    id: u32,
    decision: Decision,
) -> Result<(), String> {
    require(&window, LABEL)?;
    let action = {
        let state = host.quick_timer.lock().unwrap();
        let (popup, pending) = state
            .current
            .as_ref()
            .filter(|(popup, _)| popup.id == id)
            .ok_or("타이머 팝업이 닫혔습니다.")?;
        match (&popup.content, decision) {
            (_, Decision::Dismiss) => None,
            (
                Content::Custom { .. },
                Decision::Start {
                    seconds,
                    replace_revision,
                },
            ) => Some(Action::Start {
                mode: TimerMode::Countdown,
                seconds: Some(seconds),
                replace_revision,
            }),
            (Content::Confirm { .. }, Decision::Confirm) => pending.clone(),
            _ => return Err("이 타이머 팝업에서 실행할 수 없는 동작입니다.".into()),
        }
    };
    if let Some(action) = action {
        host.apply_timer_action(action)?;
    }
    dismiss(&window, &host, Some(id))
}
