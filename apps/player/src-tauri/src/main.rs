// Tauri's packaged builds can use debug assertions without needing a console.
#![cfg_attr(
    all(
        windows,
        any(not(debug_assertions), all(not(dev), not(feature = "preview")))
    ),
    windows_subsystem = "windows"
)]

mod display;
mod edit;
mod hit_test;
mod navigation;
mod preferences;
use buddy_platform::{InputOptions, InputSession, LaunchAtLogin, WakeSignal};
use buddy_project::{Anchor, LoadedProject, Point};
use buddy_runtime::*;
use display::Display;
use edit::EditSession;
use fs2::FileExt;
use preferences::PreferencesStore;
use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    image::Image,
    ipc::Channel,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State, WebviewUrl, WebviewWindow,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Editing,
    Hidden,
}
#[derive(Clone)]
struct Controller {
    timing: buddy_project::Timing,
    mode: Mode,
    settings_open: bool,
    paused: bool,
    keyboard: bool,
    mouse: bool,
    movement: bool,
    cursor_follow: bool,
    scale: f64,
    always_on_top: bool,
    revision: u64,
    edit: Option<EditSession>,
}
struct TrayControls {
    menu: Menu<tauri::Wry>,
    status: MenuItem<tauri::Wry>,
    visible: CheckMenuItem<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
    keyboard: CheckMenuItem<tauri::Wry>,
    mouse: CheckMenuItem<tauri::Wry>,
    movement: CheckMenuItem<tauri::Wry>,
    cursor_follow: CheckMenuItem<tauri::Wry>,
}
struct Host {
    loaded: LoadedProject,
    render_paths: std::collections::BTreeMap<String, String>,
    sound_paths: std::collections::BTreeMap<String, String>,
    runtime: Mutex<Runtime>,
    controller: Mutex<Controller>,
    preferences: Mutex<PreferenceOverrides>,
    store: Mutex<PreferencesStore>,
    launch_at_login: LaunchAtLogin,
    input_status: Mutex<InputStatus>,
    channel: Mutex<Option<Channel<WireState>>>,
    sequence: AtomicU32,
    wake: WakeSignal,
    started: Instant,
    data_directory: PathBuf,
    exiting: AtomicBool,
    display: Mutex<Display>,
    monitors: Mutex<Vec<Display>>,
    _instance_lock: File,
    edit_operation: Mutex<()>,
    buddy_clicks: Mutex<Vec<(u64, u64)>>,
    hit_mask: Mutex<hit_test::HitMask>,
    follow_target: Mutex<Option<Point>>,
    menu_open: AtomicBool,
}
impl Host {
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
    fn size(&self, scale: f64) -> Point {
        self.display
            .lock()
            .unwrap()
            .fit_size(self.loaded.canvas_size(scale))
    }
    fn base_position(&self, bounds: Rect, size: Point) -> Point {
        let defaults = &self.loaded.data.project.defaults.position;
        let bounds = display::placement_with_margin(
            bounds,
            size,
            self.placement_footprint(),
            defaults.margin_dip,
        );
        let x = match defaults.anchor {
            Anchor::TopLeft | Anchor::BottomLeft => bounds.x,
            _ => bounds.x + bounds.width - size.x,
        };
        let y = match defaults.anchor {
            Anchor::TopLeft | Anchor::TopRight => bounds.y,
            _ => bounds.y + bounds.height - size.y,
        };
        bounds.clamp(x, y, size)
    }
    fn placement_footprint(&self) -> Option<(Point, Point)> {
        self.loaded.footprints.idle.or(self.loaded.footprints.all)
    }
    fn placement_bounds(&self, display: &Display, size: Point) -> Rect {
        display::placement_bounds(display.bounds, size, self.placement_footprint())
    }
    fn render_position(&self, display: &Display, size: Point, state: &CharacterState) -> Point {
        let footprint = self
            .loaded
            .footprints
            .animations
            .get(&state.animation)
            .copied();
        display::placement_bounds(
            display.bounds,
            size,
            footprint.or(self.placement_footprint()),
        )
        .clamp(state.position.x, state.position.y, size)
    }
    fn place_on_display(
        &self,
        display: Display,
        position: Point,
        size: Point,
        drag_cursor: Option<Point>,
    ) {
        let mut runtime = self.runtime.lock().unwrap();
        runtime.place(
            self.now(),
            position,
            size,
            self.placement_bounds(&display, size),
        );
        if let Some(cursor) = drag_cursor {
            runtime.set_drag_cursor(cursor);
        }
        *self.display.lock().unwrap() = display;
        drop(runtime);
        self.wake.wake();
    }
    fn refresh_preferences(&self) {
        let defaults = &self.loaded.data.project.defaults;
        let prefs = self.preferences.lock().unwrap().clone();
        let mut c = self.controller.lock().unwrap();
        c.timing = prefs.timing.unwrap_or(defaults.timing);
        c.scale = prefs.scale.unwrap_or_else(|| self.loaded.default_scale());
        c.always_on_top = prefs.always_on_top.unwrap_or(defaults.always_on_top);
        c.keyboard = prefs.keyboard.unwrap_or(defaults.input.keyboard);
        c.mouse = prefs.mouse.unwrap_or(defaults.input.mouse);
        c.movement = prefs.movement_enabled.unwrap_or(defaults.movement_enabled);
        c.cursor_follow = self.cursor_follow_enabled(&prefs);
        c.revision += 1;
        drop(c);
        self.wake.wake();
    }
    fn cursor_follow_enabled(&self, prefs: &PreferenceOverrides) -> bool {
        prefs.cursor_follow.unwrap_or_else(|| {
            self.loaded
                .data
                .project
                .cursor_follow
                .as_ref()
                .is_some_and(|config| config.enabled)
        })
    }
    fn track_follow_cursor(&self, cursor: Option<tauri::PhysicalPosition<f64>>) {
        let target = cursor.and_then(|cursor| {
            let monitors = self.monitors.lock().unwrap();
            let macos = cfg!(target_os = "macos");
            let scale = if macos {
                monitors.iter().find(|m| m.primary)?.scale
            } else {
                1.0
            };
            let point = Point {
                x: cursor.x / scale,
                y: cursor.y / scale,
            };
            let monitor = display::at_cursor(&monitors, point)?;
            let current = self.display.lock().unwrap();
            (monitor.id == current.id).then(|| current.local_cursor(point.x, point.y, macos))
        });
        let mut previous = self.follow_target.lock().unwrap();
        if *previous != target {
            *previous = target;
            self.wake.wake();
        }
    }
    fn save(&self, prefs: PreferenceOverrides) -> Result<(), String> {
        self.store.lock().unwrap().save(&prefs)?;
        *self.preferences.lock().unwrap() = prefs;
        self.refresh_preferences();
        Ok(())
    }
    fn send_state(&self) {
        let c = self.controller.lock().unwrap().clone();
        let mut channel = self.channel.lock().unwrap();
        let Some(sender) = channel.as_ref() else {
            return;
        };
        let seq = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let (state, size, canvas_offset) = {
            let mut runtime = self.runtime.lock().unwrap();
            runtime.advance(self.now());
            let state = runtime.state();
            runtime.prepare_frame(seq, &state, c.mode != Mode::Hidden);
            let display = self.display.lock().unwrap();
            let size = display.fit_size(self.loaded.canvas_size(c.scale));
            let position = self.render_position(&display, size, &state);
            let origin = display.bounds.clamp(position.x, position.y, size);
            let offset = Point {
                x: position.x - origin.x,
                y: position.y - origin.y,
            };
            (state, size, offset)
        };
        let wire = WireState {
            seq,
            state,
            editing: c.mode == Mode::Editing,
            visible: c.mode != Mode::Hidden,
            edit_session_id: c.edit.as_ref().map(|session| session.id.clone()),
            width: size.x,
            height: size.y,
            canvas_offset,
        };
        if sender.send(wire).is_err() {
            self.runtime.lock().unwrap().frame_send_failed(seq);
            *channel = None;
        }
    }
    fn begin_edit(&self, grab: Point) -> Result<String, String> {
        let _operation = self.edit_operation.lock().unwrap();
        let position = self.runtime.lock().unwrap().state().position;
        let mut controller = self.controller.lock().unwrap();
        if controller.mode == Mode::Hidden {
            return Err("숨김을 해제한 뒤 이동하세요.".into());
        }
        if let Some(session) = &controller.edit {
            return Ok(session.id.clone());
        }
        let display = self.display.lock().unwrap();
        let size = display.fit_size(self.loaded.canvas_size(controller.scale));
        if !(0.0..=size.x).contains(&grab.x) || !(0.0..=size.y).contains(&grab.y) {
            return Err("버디 안에서 드래그를 시작하세요.".into());
        }
        let session = EditSession::new(position, display.id.clone(), grab, controller.scale);
        let id = session.id.clone();
        controller.edit = Some(session);
        controller.mode = Mode::Editing;
        controller.revision += 1;
        drop(controller);
        self.wake.wake();
        Ok(id)
    }
    fn drag_to_cursor(&self, session_id: &str, cursor: Point, macos: bool) -> Result<(), String> {
        let _operation = self.edit_operation.lock().unwrap();
        let c = self.controller.lock().unwrap().clone();
        let session = c.edit.as_ref().ok_or("유효한 편집 세션이 없습니다.")?;
        session.require(session_id)?;
        let monitors = self.monitors.lock().unwrap();
        // Tauri reports macOS cursor coordinates in the primary screen's pixels.
        let cursor_scale = if macos {
            monitors
                .iter()
                .find(|monitor| monitor.primary)
                .map_or(1.0, |monitor| monitor.scale)
        } else {
            1.0
        };
        let cursor = Point {
            x: cursor.x / cursor_scale,
            y: cursor.y / cursor_scale,
        };
        let Some(display) = display::at_cursor(&monitors, cursor).cloned() else {
            return Ok(());
        };
        drop(monitors);
        let cursor = display.local_cursor(cursor.x, cursor.y, macos);
        let size = display.fit_size(self.loaded.canvas_size(c.scale));
        let position = Point {
            x: cursor.x - session.grab.x,
            y: cursor.y - session.grab.y,
        };
        self.place_on_display(display, position, size, Some(cursor));
        Ok(())
    }
    fn finish_edit(&self, commit: bool, session_id: Option<&str>) -> Result<(), String> {
        let _operation = self.edit_operation.lock().unwrap();
        let c = self.controller.lock().unwrap().clone();
        if c.mode != Mode::Editing {
            return Err("유효한 편집 세션이 없습니다.".into());
        }
        let session = c.edit.as_ref().ok_or("유효한 편집 세션이 없습니다.")?;
        if let Some(id) = session_id {
            session.require(id)?;
        }
        let result = if commit {
            let mut prefs = self.preferences.lock().unwrap().clone();
            prefs.scale = Some(c.scale);
            let monitor_hint = self.display.lock().unwrap().id.clone();
            let position = self.runtime.lock().unwrap().state().position;
            prefs.base_placement = Some(BasePlacement {
                monitor_hint,
                position,
            });
            self.save(prefs)
        } else {
            Ok(())
        };
        if !commit || result.is_err() {
            self.controller.lock().unwrap().scale = session.scale;
            let display = display::choose(&self.monitors.lock().unwrap(), Some(&session.monitor));
            if let Some(display) = display {
                let size = display.fit_size(self.loaded.canvas_size(session.scale));
                let origin = if display.id == session.monitor {
                    session.origin
                } else {
                    self.base_position(display.bounds, size)
                };
                self.place_on_display(display, origin, size, None);
            }
        }
        if let Err(error) = &result {
            self.store.lock().unwrap().notice = Some(format!(
                "위치를 저장하지 못해 이전 위치로 돌아갔습니다. {error}"
            ));
        }
        {
            let mut c = self.controller.lock().unwrap();
            c.mode = Mode::Normal;
            c.edit = None;
            c.revision += 1;
        }
        self.wake.wake();
        result
    }
    fn reset_preferences(&self, all: bool) -> Result<(), String> {
        let display = if all {
            let prefs = self.store.lock().unwrap().reset()?;
            *self.preferences.lock().unwrap() = prefs;
            self.refresh_preferences();
            display::choose(&self.monitors.lock().unwrap(), None)
        } else {
            None
        }
        .unwrap_or_else(|| self.display.lock().unwrap().clone());
        let scale = self.controller.lock().unwrap().scale;
        let size = display.fit_size(self.loaded.canvas_size(scale));
        let position = self.base_position(display.bounds, size);
        if !all {
            let mut prefs = self.preferences.lock().unwrap().clone();
            prefs.base_placement = Some(BasePlacement {
                monitor_hint: display.id.clone(),
                position,
            });
            self.save(prefs)?;
        }
        self.place_on_display(display, position, size, None);
        if all
            && matches!(
                self.launch_at_login.status(),
                LaunchAtLoginStatus::On
                    | LaunchAtLoginStatus::NeedsApproval
                    | LaunchAtLoginStatus::PathChanged
            )
        {
            self.launch_at_login.set_enabled(false).map_err(|error| {
                format!("다른 설정은 초기화했습니다. 자동 실행은 끄지 못했습니다. {error}")
            })?;
        }
        Ok(())
    }
    fn refresh_displays(&self, app: &tauri::AppHandle) {
        let Ok(monitors) = display::list(app) else {
            return;
        };
        let _operation = self.edit_operation.lock().unwrap();
        let current = self.display.lock().unwrap().clone();
        let Some(selected) = display::choose(&monitors, Some(&current.id)) else {
            return;
        };
        *self.monitors.lock().unwrap() = monitors;
        if selected != current {
            let scale = self.controller.lock().unwrap().scale;
            let size = selected.fit_size(self.loaded.canvas_size(scale));
            let position = if selected.id != current.id {
                self.base_position(selected.bounds, size)
            } else {
                self.runtime.lock().unwrap().state().position
            };
            self.place_on_display(selected, position, size, None);
        }
    }
    fn toggle_source(&self, source: &str) -> Result<(), String> {
        let _operation = self.edit_operation.lock().unwrap();
        let mut prefs = self.preferences.lock().unwrap().clone();
        let c = self.controller.lock().unwrap().clone();
        match source {
            "keyboard" => prefs.keyboard = Some(!c.keyboard),
            "mouse" => prefs.mouse = Some(!c.mouse),
            "movement" => prefs.movement_enabled = Some(!c.movement),
            "cursor-follow" => prefs.cursor_follow = Some(!c.cursor_follow),
            _ => return Err("지원하지 않는 입력 소스입니다.".into()),
        }
        self.save(prefs)
    }
    fn open_settings(&self, app: &tauri::AppHandle) -> Result<(), String> {
        if self.controller.lock().unwrap().mode == Mode::Editing {
            self.finish_edit(false, None)?;
        }
        if let Some(window) = app.get_webview_window("settings") {
            window.unminimize().map_err(|e| e.to_string())?;
            window.show().map_err(|e| e.to_string())?;
            window.set_always_on_top(true).map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            return Ok(());
        }
        self.controller.lock().unwrap().settings_open = true;
        self.wake.wake();
        let result =
            tauri::WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
                .on_navigation(navigation::allow_player_navigation)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .initialization_script("window.location.hash = 'settings';")
                .title(format!("{} · 설정", self.loaded.data.project.product.name))
                .inner_size(620.0, 680.0)
                .min_inner_size(480.0, 560.0)
                .resizable(true)
                .always_on_top(true)
                .visible(true)
                .focused(true)
                .center()
                .build()
                .map_err(|e| e.to_string());
        if result.is_err() {
            self.controller.lock().unwrap().settings_open = false;
            self.wake.wake();
        }
        result.and_then(|window| {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())
        })
    }
}
fn require(window: &WebviewWindow, label: &str) -> Result<(), String> {
    if window.label() == label {
        Ok(())
    } else {
        Err("이 창에서는 허용되지 않는 명령입니다.".into())
    }
}
#[tauri::command]
fn get_bootstrap(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
) -> Result<PlayerBootstrap, String> {
    require(&window, "overlay")?;
    let c = host.controller.lock().unwrap().clone();
    let size = host.size(c.scale);
    Ok(PlayerBootstrap {
        character: host.loaded.data.character.clone(),
        product: host.loaded.data.project.product.clone(),
        image_paths: host.render_paths.clone(),
        sound_paths: host
            .loaded
            .data
            .project
            .bindings
            .iter()
            .filter_map(|binding| binding.sound.as_ref())
            .filter_map(|sound| host.sound_paths.get_key_value(&sound.resource))
            .map(|(id, path)| (id.clone(), path.clone()))
            .collect(),
        render_plan: host.loaded.data.render_plan.clone(),
        width: size.x,
        height: size.y,
    })
}
#[tauri::command]
fn read_reaction_audio(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    resource: String,
) -> Result<tauri::ipc::Response, String> {
    require(&window, "overlay")?;
    let asset = host
        .loaded
        .sound_files
        .get(&resource)
        .and_then(|name| host.loaded.files.get(name))
        .ok_or("사운드 리소스를 찾을 수 없습니다.")?;
    Ok(tauri::ipc::Response::new(asset.bytes.clone()))
}
#[tauri::command]
fn subscribe_character_state(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    channel: Channel<WireState>,
) -> Result<(), String> {
    require(&window, "overlay")?;
    {
        let mut target = host.channel.lock().unwrap();
        host.runtime.lock().unwrap().frame_reconnect();
        *target = Some(channel);
    }
    *host.hit_mask.lock().unwrap() = hit_test::HitMask::default();
    host.send_state();
    Ok(())
}
#[tauri::command]
fn ack_character_frame(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    ack: FrameAck,
) -> Result<(), String> {
    require(&window, "overlay")?;
    if !host.runtime.lock().unwrap().accepts_frame(&ack) {
        host.runtime
            .lock()
            .unwrap()
            .ack_frame(host.now(), &ack, false);
        return Err("발행되지 않았거나 이미 처리된 프레임입니다.".into());
    }
    let visible = host.controller.lock().unwrap().mode != Mode::Hidden;
    if visible && !window.is_visible().map_err(|e| e.to_string())? {
        window.show().map_err(|e| e.to_string())?;
    }
    host.runtime
        .lock()
        .unwrap()
        .ack_frame(host.now(), &ack, visible);
    host.wake.wake();
    Ok(())
}
#[tauri::command]
fn set_character_hit_mask(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    mask: hit_test::HitMask,
) -> Result<(), String> {
    require(&window, "overlay")?;
    if !mask.valid() {
        return Err("잘못된 클릭 영역입니다.".into());
    }
    *host.hit_mask.lock().unwrap() = mask;
    Ok(())
}
#[tauri::command]
fn get_runtime_status(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
) -> Result<RuntimeStatus, String> {
    require(&window, "settings")?;
    let c = host.controller.lock().unwrap().clone();
    let mut input = host.input_status.lock().unwrap().clone();
    if c.settings_open {
        let permission = buddy_platform::permission_state();
        input.keyboard.permission = permission;
        input.mouse.permission = permission;
        input.stop("설정 창을 닫으면 감지를 시작합니다.");
    }
    // Snapshot each owner separately; do not retain temporary mutex guards
    // across struct fields and invert the runtime/display lock order.
    let notice = host.store.lock().unwrap().notice.clone();
    let diagnostics = host.runtime.lock().unwrap().diagnostics();
    let performance = host.runtime.lock().unwrap().performance(host.now());
    let monitors = host
        .monitors
        .lock()
        .unwrap()
        .iter()
        .map(Display::choice)
        .collect();
    let selected = host.display.lock().unwrap().clone();
    let saved = host.preferences.lock().unwrap().base_placement.clone();
    let size = selected.fit_size(host.loaded.canvas_size(c.scale));
    let position = saved
        .filter(|p| p.monitor_hint == selected.id)
        .map(|p| {
            host.placement_bounds(&selected, size)
                .clamp(p.position.x, p.position.y, size)
        })
        .unwrap_or_else(|| host.base_position(selected.bounds, size));
    Ok(RuntimeStatus {
        product_name: host.loaded.data.project.product.name.clone(),
        visible: c.mode != Mode::Hidden,
        paused: c.paused,
        dragging: c.mode == Mode::Editing,
        timing: c.timing,
        keyboard: c.keyboard,
        mouse: c.mouse,
        movement_enabled: c.movement,
        cursor_follow: c.cursor_follow,
        always_on_top: c.always_on_top,
        launch_at_login: host.launch_at_login.status(),
        scale: c.scale,
        input,
        notice,
        diagnostics,
        performance,
        monitors,
        base_placement: BasePlacement {
            monitor_hint: selected.id,
            position,
        },
    })
}
#[tauri::command]
fn set_launch_at_login(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    enabled: bool,
) -> Result<(), String> {
    require(&window, "settings")?;
    let _operation = host.edit_operation.lock().unwrap();
    host.launch_at_login.set_enabled(enabled)
}
#[tauri::command]
fn reset_performance_session(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
) -> Result<(), String> {
    require(&window, "settings")?;
    let controller = host.controller.lock().unwrap();
    if !controller.settings_open {
        return Err("설정 창에서만 계측을 초기화할 수 있습니다.".into());
    }
    let now = host.now();
    let mut runtime = host.runtime.lock().unwrap();
    let mut controls = runtime.controls();
    controls.active = false;
    runtime.set_controls(now, controls);
    runtime.reset(now);
    runtime.reset_performance(now);
    drop(runtime);
    drop(controller);
    host.send_state();
    host.wake.wake();
    Ok(())
}
#[tauri::command]
fn export_runtime_diagnostics(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
) -> Result<String, String> {
    require(&window, "settings")?;
    let controller = host.controller.lock().unwrap().clone();
    let display = host.display.lock().unwrap().clone();
    let input = host.input_status.lock().unwrap().clone();
    let mut runtime = host.runtime.lock().unwrap();
    let report = serde_json::json!({
        "schemaVersion": 1,
        "engineVersion": env!("CARGO_PKG_VERSION"),
        "protocolVersion": PROTOCOL_VERSION,
        "projectHash": host.loaded.content_hash,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "debugBuild": cfg!(debug_assertions),
        "previewBuild": cfg!(feature = "preview"),
        "characterScale": controller.scale,
        "displayScale": display.scale,
        "workAreaDip": display.bounds,
        "keyboard": { "permission": input.keyboard.permission, "capture": input.keyboard.capture },
        "mouse": { "permission": input.mouse.permission, "capture": input.mouse.capture },
        "diagnostics": runtime.diagnostics(),
        "performance": runtime.performance(host.now()),
        "latencyClock": "native-callback-entry-to-renderer-draw-ack",
        "startupClock": "rust-main-entry-to-visible-character-draw-ack",
        "histogram": { "resolutionMs": 1, "limitMs": 2000, "percentile": "nearest-rank" },
    });
    drop(runtime);
    let path = host
        .data_directory
        .join(format!("diagnostics-{}.json", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    preferences::atomic_write(&path, &bytes)?;
    Ok(path.to_string_lossy().into_owned())
}
#[tauri::command]
fn request_input_access(window: WebviewWindow, host: State<'_, Arc<Host>>) -> Result<(), String> {
    require(&window, "settings")?;
    if buddy_platform::permission_state() == PermissionState::NeedsRequest {
        buddy_platform::request_input_permission();
    }
    host.controller.lock().unwrap().revision += 1;
    let c = host.controller.lock().unwrap().clone();
    *host.input_status.lock().unwrap() = InputOptions {
        keyboard: c.keyboard,
        mouse: c.mouse,
    }
    .status(
        buddy_platform::permission_state(),
        CaptureState::Stopped,
        Some("설정 창을 닫고 입력 감지 상태를 확인하세요.".into()),
    );
    host.wake.wake();
    #[cfg(target_os = "macos")]
    {
        let opened = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .status()
            .is_ok_and(|status| status.success());
        if !opened {
            return Err("시스템 설정에서 개인정보 보호 및 보안 → 입력 모니터링을 여세요.".into());
        }
        let _ = window.set_always_on_top(false);
    }
    Ok(())
}
#[tauri::command]
fn apply_user_preferences(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    update: PreferenceUpdate,
) -> Result<(), String> {
    require(&window, "settings")?;
    let _operation = host.edit_operation.lock().unwrap();
    if host.controller.lock().unwrap().mode == Mode::Editing {
        return Err("편집을 마친 뒤 설정을 저장하세요.".into());
    }
    let scale = update
        .scale
        .unwrap_or_else(|| host.controller.lock().unwrap().scale);
    if !(0.25..=3.0).contains(&scale) {
        return Err("배율은 25~300%여야 합니다.".into());
    }
    let mut prefs = host.preferences.lock().unwrap().clone();
    if let Some(timing) = update.timing {
        if !timing.is_valid() {
            return Err("타이밍 값이 허용 범위를 벗어났습니다.".into());
        }
        prefs.timing = Some(timing);
    }
    if let Some(value) = update.keyboard {
        prefs.keyboard = Some(value);
    }
    if let Some(value) = update.mouse {
        prefs.mouse = Some(value);
    }
    if let Some(value) = update.scale {
        prefs.scale = Some(value);
    }
    if let Some(value) = update.movement_enabled {
        prefs.movement_enabled = Some(value);
    }
    if let Some(value) = update.cursor_follow {
        prefs.cursor_follow = Some(value);
    }
    if let Some(value) = update.always_on_top {
        prefs.always_on_top = Some(value);
    }
    let placement = if let Some(mut placement) = update.base_placement {
        if !placement.position.x.is_finite()
            || !placement.position.y.is_finite()
            || placement.position.x.abs() > 1_000_000.0
            || placement.position.y.abs() > 1_000_000.0
        {
            return Err("유효하지 않은 위치입니다.".into());
        }
        let display = host
            .monitors
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.id == placement.monitor_hint)
            .cloned()
            .ok_or("선택한 디스플레이가 연결되어 있지 않습니다.")?;
        let size = display.fit_size(host.loaded.canvas_size(scale));
        placement.position = host.placement_bounds(&display, size).clamp(
            placement.position.x,
            placement.position.y,
            size,
        );
        prefs.base_placement = Some(placement.clone());
        Some((display, placement.position, size))
    } else {
        None
    };
    host.save(prefs)?;
    if let Some((display, position, size)) = placement {
        host.place_on_display(display, position, size, None);
    }
    Ok(())
}
#[tauri::command]
fn reset_user_preferences(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    all: bool,
) -> Result<(), String> {
    require(&window, "settings")?;
    let _operation = host.edit_operation.lock().unwrap();
    if host.controller.lock().unwrap().mode == Mode::Editing {
        return Err("편집을 마친 뒤 초기화하세요.".into());
    }
    host.reset_preferences(all)
}
#[tauri::command]
fn begin_character_drag(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    grab: Point,
) -> Result<String, String> {
    require(&window, "overlay")?;
    host.begin_edit(grab)
}
#[tauri::command]
fn update_character_edit(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    session_id: String,
) -> Result<(), String> {
    require(&window, "overlay")?;
    let cursor = window.cursor_position().map_err(|e| e.to_string())?;
    host.drag_to_cursor(
        &session_id,
        Point {
            x: cursor.x,
            y: cursor.y,
        },
        cfg!(target_os = "macos"),
    )
}
#[tauri::command]
fn finish_character_edit(
    window: WebviewWindow,
    session_id: String,
    host: State<'_, Arc<Host>>,
) -> Result<(), String> {
    require(&window, "overlay")?;
    host.finish_edit(true, Some(&session_id))
}
#[tauri::command]
fn cancel_character_edit(
    window: WebviewWindow,
    session_id: String,
    host: State<'_, Arc<Host>>,
) -> Result<(), String> {
    require(&window, "overlay")?;
    host.finish_edit(false, Some(&session_id))
}

#[tauri::command]
fn character_click(window: WebviewWindow, host: State<'_, Arc<Host>>) -> Result<(), String> {
    require(&window, "overlay")?;
    let controller = host.controller.lock().unwrap();
    if controller.mode == Mode::Normal && !controller.settings_open && !controller.paused {
        let mut clicks = host.buddy_clicks.lock().unwrap();
        if clicks.len() < 32 {
            clicks.push((controller.revision, host.now()));
            host.wake.wake();
        }
    }
    Ok(())
}

#[tauri::command]
async fn show_buddy_menu(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    controls: State<'_, TrayControls>,
    position: Point,
) -> Result<(), String> {
    require(&window, "overlay")?;
    if host.controller.lock().unwrap().mode != Mode::Normal
        || !position.x.is_finite()
        || !position.y.is_finite()
    {
        return Ok(());
    }
    if host.menu_open.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    host.track_follow_cursor(None);
    let result = window
        .popup_menu_at(
            &controls.menu,
            tauri::LogicalPosition::new(position.x, position.y),
        )
        .map_err(|error| error.to_string());
    host.menu_open.store(false, Ordering::Release);
    result
}

fn start_worker(app: tauri::AppHandle, host: Arc<Host>, overlay_id: usize) {
    std::thread::Builder::new()
        .name("buddy-runtime-host".into())
        .spawn(move || {
            let mut input: Option<InputSession> = None;
            let mut input_revision = u64::MAX;
            let mut last_state: Option<CharacterState> = None;
            let mut last_sent = 0;
            let mut last_mode = Mode::Normal;
            let mut last_size = Point::default();
            let mut sent_mode = None;
            let mut sent_size = Point::default();
            let mut sent_edit_id: Option<String> = None;
            let mut last_top = None;
            let mut last_window_placement: Option<(Display, Point)> = None;
            let mut monitor_check = 0;
            let mut last_menu = String::new();
            while !host.exiting.load(Ordering::Acquire) {
                let c = host.controller.lock().unwrap().clone();
                let active = c.mode == Mode::Normal && !c.settings_open && !c.paused;
                let now = host.now();
                if now >= monitor_check {
                    host.refresh_displays(&app);
                    monitor_check = now + 2000;
                }
                if input_revision != c.revision || !active {
                    input = None;
                    input_revision = c.revision;
                }
                if active && (c.keyboard || c.mouse) && input.is_none() {
                    match InputSession::start_for_overlay(
                        InputOptions {
                            keyboard: c.keyboard,
                            mouse: c.mouse,
                        },
                        host.wake.clone(),
                        overlay_id,
                    ) {
                        Ok(session) => input = Some(session),
                        Err(error) => {
                            *host.input_status.lock().unwrap() = InputOptions {
                                keyboard: c.keyboard,
                                mouse: c.mouse,
                            }
                            .status(
                                buddy_platform::permission_state(),
                                CaptureState::Failed,
                                Some(error),
                            );
                        }
                    }
                }
                if let Some(input) = &input {
                    *host.input_status.lock().unwrap() = input.status();
                }
                if !active || (!c.keyboard && !c.mouse) {
                    let reason = if c.mode == Mode::Hidden {
                        "캐릭터를 숨겨 감지가 중지됐습니다."
                    } else if c.mode == Mode::Editing {
                        "위치 편집 중에는 감지가 중지됩니다."
                    } else if c.settings_open {
                        "설정 창을 닫으면 감지를 시작합니다."
                    } else if c.paused {
                        "입력 감지를 일시 정지했습니다."
                    } else {
                        "사용자 설정에서 감지를 껐습니다."
                    };
                    host.input_status.lock().unwrap().stop(reason);
                }
                let source_status = host.input_status.lock().unwrap().clone();
                // Do not process a batch captured under a mode/source revision
                // that changed while the native adapter was stopping/starting.
                // Release this guard before making any window API calls.
                let controller_guard = host.controller.lock().unwrap();
                if controller_guard.revision != c.revision
                    || controller_guard.mode != c.mode
                    || controller_guard.settings_open != c.settings_open
                {
                    drop(controller_guard);
                    continue;
                }
                let mut runtime = host.runtime.lock().unwrap();
                let display = host.display.lock().unwrap().clone();
                let size = display.fit_size(host.loaded.canvas_size(c.scale));
                let center =
                    host.loaded
                        .footprints
                        .idle
                        .map_or(Point { x: 0.5, y: 0.5 }, |(min, max)| Point {
                            x: (min.x + max.x) / 2.0,
                            y: (min.y + max.y) / 2.0,
                        });
                runtime.set_native_cursor_follow(
                    c.cursor_follow,
                    *host.follow_target.lock().unwrap(),
                    center,
                );
                runtime.set_timing(now, c.timing);
                runtime.set_controls(
                    now,
                    Controls {
                        active,
                        keyboard: c.keyboard
                            && source_status.keyboard.capture == CaptureState::Running,
                        mouse: c.mouse && source_status.mouse.capture == CaptureState::Running,
                        movement_enabled: c.movement,
                    },
                );
                if size != last_size {
                    let position = runtime.state().position;
                    runtime.place(now, position, size, host.placement_bounds(&display, size));
                }
                if let Some(input) = &mut input {
                    for _ in 0..1024 {
                        let Some(captured) = input.poll_captured() else {
                            break;
                        };
                        let received_at = captured
                            .received_at
                            .saturating_duration_since(host.started)
                            .as_millis() as u64;
                        let mut event = captured.event;
                        if let InputEvent::CursorMove { x, y } = &mut event {
                            let point = display.local_cursor(*x, *y, cfg!(target_os = "macos"));
                            *x = point.x;
                            *y = point.y;
                        }
                        runtime.handle_received(host.now(), received_at, event);
                    }
                }
                runtime.handle(
                    host.now(),
                    if c.mode == Mode::Editing {
                        InputEvent::DragStart
                    } else {
                        InputEvent::DragEnd
                    },
                );
                for (revision, received_at) in host.buddy_clicks.lock().unwrap().drain(..) {
                    if active && revision == c.revision {
                        runtime.handle_received(host.now(), received_at, InputEvent::BuddyClick);
                    }
                }
                runtime.advance(host.now());
                let state = runtime.state();
                let deadline = runtime.next_deadline_ms();
                drop(runtime);
                drop(controller_guard);
                if let Some(window) = app.get_webview_window("overlay") {
                    if last_mode != c.mode {
                        if c.mode == Mode::Hidden {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                        }
                    }
                    if size != last_size {
                        let _ = window.set_size(tauri::LogicalSize::new(size.x, size.y));
                    }
                    if last_top != Some(c.always_on_top) {
                        let _ = window.set_always_on_top(c.always_on_top);
                        last_top = Some(c.always_on_top);
                    }
                    // Keep the native window inside the work area. The canvas
                    // can extend beyond its edges, even on macOS where AppKit
                    // constrains the frame below the menu bar.
                    let position = host.render_position(&display, size, &state);
                    let origin = display.bounds.clamp(position.x, position.y, size);
                    if last_window_placement.as_ref().is_none_or(
                        |(placed_display, placed_position)| {
                            placed_display != &display || *placed_position != origin
                        },
                    ) && display.position_window(&window, origin).is_ok()
                    {
                        last_window_placement = Some((display, origin));
                    }
                }
                let changed = last_state.as_ref().is_none_or(|old| {
                    old.action_instance_id != state.action_instance_id
                        || old.position != state.position
                        || old.facing_left != state.facing_left
                }) || sent_mode != Some(c.mode)
                    || size != sent_size
                    || sent_edit_id.as_deref()
                        != c.edit.as_ref().map(|session| session.id.as_str());
                let mut next = Some(deadline.map_or(monitor_check, |due| due.min(monitor_check)));
                if changed && now.saturating_sub(last_sent) >= 34 {
                    host.send_state();
                    last_sent = now;
                    last_state = Some(state);
                    sent_mode = Some(c.mode);
                    sent_size = size;
                    sent_edit_id = c.edit.as_ref().map(|session| session.id.clone());
                } else if changed {
                    next = Some(next.map_or(last_sent + 34, |due| due.min(last_sent + 34)));
                }
                let capture = |enabled: bool, source: &InputSourceStatus| {
                    if !enabled {
                        "꺼짐"
                    } else {
                        match source.capture {
                            CaptureState::Running => "감지 중",
                            CaptureState::Failed => "실패 · 설정 확인",
                            _ if source.permission != PermissionState::Granted
                                && source.permission != PermissionState::NotRequired =>
                            {
                                "권한 필요"
                            }
                            _ => "감지 보류",
                        }
                    }
                };
                let text = format!(
                    "키보드 {} · 마우스 {}",
                    capture(c.keyboard, &source_status.keyboard),
                    capture(c.mouse, &source_status.mouse)
                );
                let menu_key = format!(
                    "{text}:{:?}:{}:{}:{}:{}:{}",
                    c.mode as u8, c.paused, c.keyboard, c.mouse, c.movement, c.cursor_follow
                );
                if menu_key != last_menu {
                    if let Some(menu) = app.try_state::<TrayControls>() {
                        let _ = menu.status.set_text(text);
                        let _ = menu.visible.set_checked(c.mode != Mode::Hidden);
                        let _ = menu.pause.set_checked(c.paused);
                        let _ = menu.keyboard.set_checked(c.keyboard);
                        let _ = menu.mouse.set_checked(c.mouse);
                        let _ = menu.movement.set_checked(c.movement);
                        let _ = menu.cursor_follow.set_checked(c.cursor_follow);
                    }
                    last_menu = menu_key;
                }
                last_mode = c.mode;
                last_size = size;
                host.wake
                    .wait(next.map(|due| Duration::from_millis(due.saturating_sub(host.now()))));
            }
        })
        .expect("runtime worker creation");
}

fn project_path(app: &tauri::AppHandle) -> Result<(PathBuf, bool), Box<dyn std::error::Error>> {
    #[cfg(feature = "preview")]
    {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--project")) {
            return Err("미리보기에는 --project 경로가 필요합니다.".into());
        }
        let path = args.next().ok_or("미리보기 프로젝트 경로가 필요합니다.")?;
        if args.next().is_some() {
            return Err("지원하지 않는 미리보기 인수입니다.".into());
        }
        let _ = app;
        Ok((PathBuf::from(path), false))
    }
    #[cfg(not(feature = "preview"))]
    {
        if let Some(resource) = buddy_project::bundled_project_path(&app.path().resource_dir()?)? {
            return Ok((resource, true));
        }
        if !cfg!(debug_assertions) {
            return Err("Bundled project is missing".into());
        }
        let path = std::env::var_os("BUDDY_PROJECT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/typing-cat")
            });
        Ok((path, false))
    }
}

fn handle_buddy_menu(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    let Some(host) = app.try_state::<Arc<Host>>() else {
        return;
    };
    let result = match event.id.as_ref() {
        "settings" => host.open_settings(app),
        "visibility" => {
            if host.controller.lock().unwrap().mode == Mode::Editing {
                let _ = host.finish_edit(false, None);
            }
            let mut c = host.controller.lock().unwrap();
            c.mode = if c.mode == Mode::Hidden {
                Mode::Normal
            } else {
                Mode::Hidden
            };
            c.revision += 1;
            drop(c);
            host.wake.wake();
            Ok(())
        }
        "keyboard" | "mouse" | "movement" | "cursor-follow" => {
            host.toggle_source(event.id.as_ref())
        }
        "reset-position" => {
            if host.controller.lock().unwrap().mode == Mode::Editing {
                host.finish_edit(false, None)
                    .and_then(|_| host.reset_preferences(false))
            } else {
                host.reset_preferences(false)
            }
        }
        "pause" => {
            if host.controller.lock().unwrap().mode == Mode::Editing {
                let _ = host.finish_edit(false, None);
            }
            let mut c = host.controller.lock().unwrap();
            c.paused = !c.paused;
            c.revision += 1;
            drop(c);
            host.wake.wake();
            Ok(())
        }
        "quit" => {
            host.exiting.store(true, Ordering::Release);
            host.wake.wake();
            app.exit(0);
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(error) = result {
        host.store.lock().unwrap().notice = Some(error);
    }
}

fn main() {
    let process_started = Instant::now();
    let app = tauri::Builder::default()
        .on_menu_event(handle_buddy_menu)
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            read_reaction_audio,
            subscribe_character_state,
            ack_character_frame,
            set_character_hit_mask,
            get_runtime_status,
            set_launch_at_login,
            reset_performance_session,
            export_runtime_diagnostics,
            request_input_access,
            apply_user_preferences,
            reset_user_preferences,
            begin_character_drag,
            update_character_edit,
            finish_character_edit,
            cancel_character_edit,
            character_click,
            show_buddy_menu
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let (project, bundled_project) = project_path(app.handle())?;
            let prepared = app.path().resource_dir()?.join("prepared-render");
            let loaded = buddy_project::load_project_with_prepared(project, &prepared)?;
            let launch_at_login =
                LaunchAtLogin::new(bundled_project, &loaded.data.project.product.identifier);
            let preview = cfg!(feature = "preview") || cfg!(debug_assertions);
            let identity = buddy_runtime::preference_identity(
                &loaded.data.project.product.identifier,
                &loaded.data.project.project_id,
                preview,
            );
            let directory = app
                .path()
                .app_data_dir()?
                .join(buddy_project::sha256(identity.as_bytes()));
            fs::create_dir_all(&directory)?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(directory.join("instance.lock"))?;
            lock.try_lock_exclusive()
                .map_err(|_| "이 앱은 이미 실행 중입니다.")?;
            let (store, prefs) = PreferencesStore::open(&directory)?;
            let monitors = display::list(app.handle())?;
            let selected = display::choose(
                &monitors,
                prefs
                    .base_placement
                    .as_ref()
                    .map(|p| p.monitor_hint.as_str()),
            )
            .ok_or("디스플레이를 찾지 못했습니다.")?;
            let bounds = selected.bounds;
            let defaults = &loaded.data.project.defaults;
            let controller = Controller {
                timing: prefs.timing.unwrap_or(defaults.timing),
                mode: Mode::Normal,
                settings_open: false,
                paused: false,
                keyboard: prefs.keyboard.unwrap_or(defaults.input.keyboard),
                mouse: prefs.mouse.unwrap_or(defaults.input.mouse),
                movement: prefs.movement_enabled.unwrap_or(defaults.movement_enabled),
                cursor_follow: prefs.cursor_follow.unwrap_or_else(|| {
                    loaded
                        .data
                        .project
                        .cursor_follow
                        .as_ref()
                        .is_some_and(|config| config.enabled)
                }),
                scale: prefs.scale.unwrap_or_else(|| loaded.default_scale()),
                always_on_top: prefs.always_on_top.unwrap_or(defaults.always_on_top),
                revision: 0,
                edit: None,
            };
            let runtime = Runtime::new(
                loaded.data.clone(),
                bounds,
                Point::default(),
                Point { x: 256.0, y: 256.0 },
            );
            let mut render_paths = std::collections::BTreeMap::new();
            if loaded.prepared_render_paths.is_empty() {
                let media_directory = directory.join("render-cache").join(&loaded.content_hash);
                fs::create_dir_all(&media_directory)?;
                for (name, bytes) in &loaded.render_assets {
                    let path = media_directory.join(name);
                    fs::write(&path, bytes)?;
                    app.asset_protocol_scope().allow_file(&path)?;
                    render_paths.insert(name.clone(), path.to_string_lossy().into_owned());
                }
            } else {
                for (name, path) in &loaded.prepared_render_paths {
                    app.asset_protocol_scope().allow_file(path)?;
                    render_paths.insert(name.clone(), path.to_string_lossy().into_owned());
                }
            }
            let mut sound_paths = std::collections::BTreeMap::new();
            for (id, name) in &loaded.sound_files {
                let asset = &loaded.files[name];
                // Web Audio reads the validated bytes via IPC; HTML audio can
                // fall back to the original verified file without a cache copy.
                app.asset_protocol_scope().allow_file(&asset.path)?;
                sound_paths.insert(id.clone(), asset.path.to_string_lossy().into_owned());
            }
            let host = Arc::new(Host {
                loaded,
                render_paths,
                sound_paths,
                runtime: Mutex::new(runtime),
                controller: Mutex::new(controller),
                preferences: Mutex::new(prefs),
                store: Mutex::new(store),
                launch_at_login,
                input_status: Mutex::new(InputOptions::default().status(
                    buddy_platform::permission_state(),
                    CaptureState::Stopped,
                    None,
                )),
                channel: Mutex::new(None),
                sequence: AtomicU32::new(0),
                wake: WakeSignal::default(),
                started: process_started,
                data_directory: directory,
                exiting: AtomicBool::new(false),
                display: Mutex::new(selected),
                monitors: Mutex::new(monitors),
                _instance_lock: lock,
                edit_operation: Mutex::new(()),
                buddy_clicks: Mutex::new(Vec::new()),
                hit_mask: Mutex::default(),
                follow_target: Mutex::default(),
                menu_open: AtomicBool::new(false),
            });
            let scale = host.controller.lock().unwrap().scale;
            let size = host.size(scale);
            let position = host
                .preferences
                .lock()
                .unwrap()
                .base_placement
                .as_ref()
                .filter(|p| p.monitor_hint == host.display.lock().unwrap().id)
                .map(|p| p.position)
                .unwrap_or_else(|| host.base_position(bounds, size));
            host.runtime.lock().unwrap().place(
                0,
                position,
                size,
                host.placement_bounds(&host.display.lock().unwrap(), size),
            );
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("index.html".into()),
            )
            .on_navigation(navigation::allow_player_navigation)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .title(&host.loaded.data.project.product.name)
            .inner_size(size.x, size.y)
            .visible(false)
            .transparent(true)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(cfg!(target_os = "macos"))
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .shadow(false)
            .build()?;
            window.set_ignore_cursor_events(true)?;
            #[cfg(target_os = "macos")]
            let overlay_id = std::process::id() as usize;
            #[cfg(target_os = "windows")]
            let overlay_id = window.hwnd()?.0 as usize;
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let overlay_id = 0;
            let menu = Menu::new(app)?;
            let status =
                MenuItem::with_id(app, "status", "입력 감지 준비 중", false, None::<&str>)?;
            menu.append(&status)?;
            menu.append(&MenuItem::with_id(
                app,
                "settings",
                "설정 · 입력 권한",
                true,
                None::<&str>,
            )?)?;
            let visible =
                CheckMenuItem::with_id(app, "visibility", "버디 표시", true, true, None::<&str>)?;
            let pause =
                CheckMenuItem::with_id(app, "pause", "입력 일시 정지", true, false, None::<&str>)?;
            let keyboard = CheckMenuItem::with_id(
                app,
                "keyboard",
                "키보드 감지",
                true,
                host.controller.lock().unwrap().keyboard,
                None::<&str>,
            )?;
            let mouse = CheckMenuItem::with_id(
                app,
                "mouse",
                "마우스 감지",
                true,
                host.controller.lock().unwrap().mouse,
                None::<&str>,
            )?;
            let movement = CheckMenuItem::with_id(
                app,
                "movement",
                "키를 누른 채 이동",
                true,
                host.controller.lock().unwrap().movement,
                None::<&str>,
            )?;
            let cursor_follow = CheckMenuItem::with_id(
                app,
                "cursor-follow",
                "마우스 따라오기",
                true,
                host.controller.lock().unwrap().cursor_follow,
                None::<&str>,
            )?;
            for item in [
                &visible,
                &pause,
                &keyboard,
                &mouse,
                &movement,
                &cursor_follow,
            ] {
                menu.append(item)?;
            }
            for (id, label) in [("reset-position", "위치 초기화"), ("quit", "종료")] {
                menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
            }
            app.manage(TrayControls {
                menu: menu.clone(),
                status,
                visible,
                pause,
                keyboard,
                mouse,
                movement,
                cursor_follow,
            });
            TrayIconBuilder::new()
                .icon(Image::from_bytes(include_bytes!("../icons/icon.png"))?)
                .tooltip(&host.loaded.data.project.product.name)
                .menu(&menu)
                .build(app)?;
            app.manage(host.clone());
            hit_test::start(window, host.clone());
            start_worker(app.handle().clone(), host, overlay_id);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(host) = window.try_state::<Arc<Host>>() {
                    let mut c = host.controller.lock().unwrap();
                    c.settings_open = false;
                    c.revision += 1;
                    drop(c);
                    host.wake.wake();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Desktop Buddy Player startup");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(host) = app.try_state::<Arc<Host>>() {
                host.exiting.store(true, Ordering::Release);
                host.wake.wake();
            }
        }
    });
}

#[cfg(test)]
mod drag_tests {
    use super::*;

    const START: Point = Point { x: 100.0, y: 100.0 };
    const GRAB: Point = Point { x: 32.0, y: 40.0 };
    // The second screen is above and to the left of a 2× primary screen.
    const CURSOR_ON_EXTERNAL: Point = Point {
        x: -2200.0,
        y: 520.0,
    };
    const DROP_ON_EXTERNAL: Point = Point { x: 308.0, y: 320.0 };

    fn drag_host(directory: &std::path::Path, read_only: bool) -> Host {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/typing-cat");
        let loaded = buddy_project::load_project(source).unwrap();
        let defaults = &loaded.data.project.defaults;
        let controller = Controller {
            timing: defaults.timing,
            mode: Mode::Normal,
            settings_open: false,
            paused: false,
            keyboard: true,
            mouse: false,
            movement: false,
            cursor_follow: false,
            scale: 1.0,
            always_on_top: true,
            revision: 0,
            edit: None,
        };
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 1200.0,
            height: 900.0,
        };
        let display = Display {
            id: "test-display".into(),
            name: "Test display".into(),
            primary: true,
            origin: Point::default(),
            scale: 2.0,
            bounds,
            desktop_area: bounds,
        };
        let external = Display {
            id: "external".into(),
            name: "External display".into(),
            primary: false,
            origin: Point {
                x: -1440.0,
                y: -100.0,
            },
            scale: 1.0,
            bounds,
            desktop_area: Rect {
                x: -1440.0,
                y: -100.0,
                width: 1440.0,
                height: 900.0,
            },
        };
        let size = display.fit_size(loaded.canvas_size(1.0));
        let runtime = Runtime::new(loaded.data.clone(), bounds, START, size);
        let (mut store, prefs) = PreferencesStore::open(directory).unwrap();
        store.read_only = read_only;
        Host {
            loaded,
            render_paths: Default::default(),
            sound_paths: Default::default(),
            runtime: Mutex::new(runtime),
            controller: Mutex::new(controller),
            preferences: Mutex::new(prefs),
            store: Mutex::new(store),
            launch_at_login: LaunchAtLogin::default(),
            input_status: Mutex::new(InputOptions::default().status(
                PermissionState::NotRequired,
                CaptureState::Stopped,
                None,
            )),
            channel: Mutex::new(None),
            sequence: AtomicU32::new(0),
            wake: WakeSignal::default(),
            started: Instant::now(),
            data_directory: directory.to_path_buf(),
            exiting: AtomicBool::new(false),
            display: Mutex::new(display.clone()),
            monitors: Mutex::new(vec![display, external]),
            _instance_lock: File::create(directory.join("instance.lock")).unwrap(),
            edit_operation: Mutex::new(()),
            buddy_clicks: Mutex::new(Vec::new()),
            hit_mask: Mutex::default(),
            follow_target: Mutex::default(),
            menu_open: AtomicBool::new(false),
        }
    }

    #[test]
    fn cross_display_drop_persists_the_new_monitor_and_local_position() {
        let temp = tempfile::tempdir().unwrap();
        let host = drag_host(temp.path(), false);
        let session = host.begin_edit(GRAB).unwrap();
        host.drag_to_cursor(&session, CURSOR_ON_EXTERNAL, true)
            .unwrap();
        assert_eq!(host.display.lock().unwrap().id, "external");
        assert_eq!(
            host.runtime.lock().unwrap().state().position,
            DROP_ON_EXTERNAL
        );
        host.finish_edit(true, Some(&session)).unwrap();
        let (_, prefs) = PreferencesStore::open(temp.path()).unwrap();
        let placement = prefs.base_placement.unwrap();
        assert_eq!(placement.monitor_hint, "external");
        assert_eq!(placement.position, DROP_ON_EXTERNAL);
    }

    #[test]
    fn failed_position_save_restores_the_buddy_and_ends_the_drag() {
        let temp = tempfile::tempdir().unwrap();
        let host = drag_host(temp.path(), true);
        let session = host.begin_edit(GRAB).unwrap();
        host.drag_to_cursor(&session, CURSOR_ON_EXTERNAL, true)
            .unwrap();
        assert_eq!(host.display.lock().unwrap().id, "external");
        assert!(host.finish_edit(true, Some(&session)).is_err());
        assert_eq!(host.display.lock().unwrap().id, "test-display");
        assert_eq!(host.runtime.lock().unwrap().state().position, START);
        assert!(host.controller.lock().unwrap().mode == Mode::Normal);
        assert!(host.controller.lock().unwrap().edit.is_none());
        assert!(host
            .store
            .lock()
            .unwrap()
            .notice
            .as_deref()
            .unwrap()
            .contains("이전 위치"));
        assert_ne!(host.begin_edit(Point::default()).unwrap(), session);
    }
}
