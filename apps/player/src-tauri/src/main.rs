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
mod recovery;
mod timer;
mod timer_menu;
mod timer_quick;
use buddy_platform::{InputOptions, InputSession, LaunchAtLogin, WakeSignal};
use buddy_project::{Anchor, LoadedProject, Point};
use buddy_runtime::*;
use display::Display;
use edit::EditSession;
use fs2::FileExt;
use preferences::PreferencesStore;
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    image::Image,
    ipc::Channel,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    Emitter, Manager, State, WebviewUrl, WebviewWindow,
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
    sound_enabled: bool,
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
    timer: Option<timer_menu::TimerMenu>,
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
    overlay_ready: AtomicBool,
    recovery: Mutex<recovery::Recovery>,
    overlay_geometry_revision: AtomicU32,
    sequence: AtomicU32,
    wake: WakeSignal,
    hit_test_wake: WakeSignal,
    started: Instant,
    data_directory: PathBuf,
    exiting: AtomicBool,
    display: Mutex<Display>,
    monitors: Mutex<Vec<Display>>,
    _instance_lock: File,
    edit_operation: Mutex<()>,
    overlay_inputs: Mutex<Vec<(u64, u64, InputEvent)>>,
    hit_mask: Mutex<hit_test::HitMask>,
    follow_target: Mutex<Option<Point>>,
    menu_open: AtomicBool,
    timer: Option<Mutex<timer::Timer>>,
    timer_menu_requests: Mutex<VecDeque<(String, u32)>>,
    timer_menu_busy: AtomicBool,
    timer_window_closing: AtomicBool,
    quick_timer: Mutex<timer_quick::State>,
}
impl Host {
    fn request_exit(&self) {
        self.exiting.store(true, Ordering::Release);
        self.wake.wake();
        self.hit_test_wake.wake();
    }

    fn timer_interactions_allowed(c: &Controller) -> bool {
        c.mode != Mode::Hidden
            && !c.settings_open
            && !c.paused
            && buddy_platform::desktop_available()
    }

    fn open_timer(&self, app: &tauri::AppHandle) -> Result<(), String> {
        if self.timer.is_none() {
            return Err("타이머 사용이 꺼져 있습니다.".into());
        }
        let window = app
            .get_webview_window(timer::WINDOW_LABEL)
            .map_or_else(|| timer::create_window(app), Ok)
            .map_err(|error| error.to_string())?;
        window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
            .and_then(|_| window.emit("timer-focus", ()))
            .map_err(|error| error.to_string())
    }

    fn queue_timer_menu(&self, id: &str, revision: u32) {
        self.timer_menu_requests
            .lock()
            .unwrap()
            .push_back((id.into(), revision));
        self.wake.wake();
    }

    fn timer_window_is_closing(&self) -> bool {
        self.timer_window_closing.load(Ordering::Acquire)
            || self.quick_timer.lock().unwrap().closing
    }

    fn schedule_timer_menu_requests(self: &Arc<Self>, app: &tauri::AppHandle) {
        if self.timer_menu_requests.lock().unwrap().is_empty()
            || self.timer_window_is_closing()
            || self.timer_menu_busy.swap(true, Ordering::AcqRel)
        {
            return;
        }
        // Creating a WebView can block. Keep both the native event handler and
        // the runtime loop free while opening the timer window or popup.
        let host = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            host.handle_timer_menu_requests(&app);
            host.timer_menu_busy.store(false, Ordering::Release);
            host.wake.wake();
        });
    }

    fn handle_timer_menu_requests(&self, app: &tauri::AppHandle) {
        while !self.exiting.load(Ordering::Acquire) && !self.timer_window_is_closing() {
            let request = self.timer_menu_requests.lock().unwrap().pop_front();
            let Some((id, revision)) = request else {
                break;
            };
            let result = if id == "timer-open" {
                self.open_timer(app)
            } else {
                timer_quick::handle_menu(app, self, &id, revision)
            };
            if let Err(error) = result {
                self.store.lock().unwrap().notice = Some(error);
            }
        }
    }

    fn apply_timer_action(&self, action: timer::Action) -> Result<timer::TimerView, String> {
        let c = self.controller.lock().unwrap().clone();
        let mut timer = self
            .timer
            .as_ref()
            .ok_or("타이머 사용이 꺼져 있습니다.")?
            .lock()
            .unwrap();
        let now = timer::now_ms();
        let can_react = Self::timer_interactions_allowed(&c)
            && (self.overlay_ready.load(Ordering::Acquire) || self.recovering());
        let visible = timer.due(now) && can_react;
        timer.poll(now, visible);
        let result = timer.apply(action, now);
        if result.is_ok() && matches!(timer.signal, Some(timer::Signal::Start(_))) && !can_react {
            timer.signal = Some(timer::Signal::Cancel);
        }
        self.wake.wake();
        result.map(|_| timer.view(now))
    }

    fn timer_view(&self) -> Result<timer::TimerView, String> {
        let timer = self
            .timer
            .as_ref()
            .ok_or("타이머 사용이 꺼져 있습니다.")?
            .lock()
            .unwrap();
        Ok(timer.view(timer::now_ms()))
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn recovering(&self) -> bool {
        self.recovery.lock().unwrap().deadline.is_some()
    }

    fn request_overlay_recovery(&self) {
        let _operation = self.edit_operation.lock().unwrap();
        let mut c = self.controller.lock().unwrap();
        let mut channel = self.channel.lock().unwrap();
        let mut runtime = self.runtime.lock().unwrap();
        let mut recovery = self.recovery.lock().unwrap();
        if !recovery.start(self.now()) {
            return;
        }
        self.overlay_ready.store(false, Ordering::Release);
        *channel = None;
        runtime.frame_reconnect();
        *self.hit_mask.lock().unwrap() = hit_test::HitMask::default();
        *self.follow_target.lock().unwrap() = None;
        if c.mode == Mode::Editing {
            c.mode = Mode::Normal;
            c.edit = None;
        }
        c.revision += 1;
        self.overlay_inputs.lock().unwrap().clear();
        self.wake.wake();
        self.hit_test_wake.wake();
    }
    fn queue_overlay_input(&self, revision: u64, event: InputEvent) {
        let mut inputs = self.overlay_inputs.lock().unwrap();
        if inputs.len() < 32 {
            inputs.push((revision, self.now(), event));
            self.wake.wake();
        }
    }
    fn record_visibility(&self, visible: bool) {
        let c = self.controller.lock().unwrap();
        if visible == (c.mode != Mode::Hidden) {
            self.runtime
                .lock()
                .unwrap()
                .set_overlay_visible(self.now(), visible);
            self.wake.wake();
        }
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
    fn render_footprint(&self, state: &CharacterState) -> Option<(Point, Point)> {
        self.loaded
            .footprints
            .animations
            .get(&state.animation)
            .copied()
            .or_else(|| self.placement_footprint())
    }
    fn render_position(&self, display: &Display, size: Point, state: &CharacterState) -> Point {
        display::placement_bounds(display.bounds, size, self.render_footprint(state)).clamp(
            state.position.x,
            state.position.y,
            size,
        )
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
        c.sound_enabled = prefs.sound_enabled.unwrap_or(true);
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
            sound_enabled: c.sound_enabled,
            edit_session_id: c.edit.as_ref().map(|session| session.id.clone()),
            width: size.x,
            height: size.y,
            canvas_offset,
        };
        if sender.send(wire).is_err() {
            let mut runtime = self.runtime.lock().unwrap();
            runtime.frame_send_failed(seq);
            runtime.frame_reconnect();
            *channel = None;
            self.overlay_ready.store(false, Ordering::Release);
            self.recovery.lock().unwrap().fail();
            self.wake.wake();
            self.hit_test_wake.wake();
        }
    }
    fn begin_edit(&self, grab: Point) -> Result<String, String> {
        let _operation = self.edit_operation.lock().unwrap();
        let position = self.runtime.lock().unwrap().state().position;
        let mut controller = self.controller.lock().unwrap();
        if controller.mode == Mode::Hidden {
            return Err("숨김을 해제한 뒤 이동하세요.".into());
        }
        if self.recovering() {
            return Err("버디 화면을 다시 불러오는 중입니다.".into());
        }
        if let Some(session) = &controller.edit {
            return Ok(session.id.clone());
        }
        let display = self.display.lock().unwrap();
        let size = display.fit_size(self.loaded.canvas_size(controller.scale));
        if !(0.0..=size.x).contains(&grab.x) || !(0.0..=size.y).contains(&grab.y) {
            return Err("버디 안에서 드래그를 시작하세요.".into());
        }
        let grab_fraction = Point {
            x: grab.x / size.x,
            y: grab.y / size.y,
        };
        let session = EditSession::new(
            position,
            display.id.clone(),
            grab_fraction,
            controller.scale,
        );
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
            x: cursor.x - session.grab_fraction.x * size.x,
            y: cursor.y - session.grab_fraction.y * size.y,
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
            if commit && session_id.is_some() && result.is_ok() && !c.settings_open && !c.paused {
                self.queue_overlay_input(c.revision, InputEvent::DragDropped);
            }
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
    fn refresh_displays(&self, app: &tauri::AppHandle) -> bool {
        let Ok(monitors) = display::list(app) else {
            return false;
        };
        let _operation = self.edit_operation.lock().unwrap();
        let current = self.display.lock().unwrap().clone();
        let Some(selected) = display::choose(&monitors, Some(&current.id)) else {
            return false;
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
            return true;
        }
        false
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
    generation: u32,
) -> Result<PlayerBootstrap, String> {
    require(&window, "overlay")?;
    host.recovery.lock().unwrap().require(generation)?;
    let c = host.controller.lock().unwrap().clone();
    let size = host.size(c.scale);
    Ok(PlayerBootstrap {
        sound_enabled: c.sound_enabled,
        character: host.loaded.data.character.clone(),
        product: host.loaded.data.project.product.clone(),
        image_paths: host.render_paths.clone(),
        sound_paths: host.sound_paths.clone(),
        render_plan: host.loaded.data.render_plan.clone(),
        width: size.x,
        height: size.y,
    })
}
#[tauri::command]
async fn read_render_frame(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    frame: String,
    generation: u32,
) -> Result<tauri::ipc::Response, String> {
    require(&window, "overlay")?;
    host.recovery.lock().unwrap().require(generation)?;
    let path = host
        .render_paths
        .get(&frame)
        .ok_or("모션 프레임을 찾을 수 없습니다.")?
        .clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        // A prepared frame can contain up to 4096 × 4096 RGBA pixels.
        const MAX_FRAME_BYTES: u64 = 96 * 1024 * 1024;
        let file = File::open(path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if length > MAX_FRAME_BYTES {
            return Err("모션 프레임이 크기 제한을 초과합니다.".into());
        }
        let mut bytes = Vec::with_capacity(length as usize);
        file.take(length + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != length {
            return Err("모션 프레임을 읽는 동안 파일이 변경되었습니다.".into());
        }
        Ok::<_, String>(bytes)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}
#[tauri::command]
async fn read_reaction_audio(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    resource: String,
    sample_rate: u32,
    max_decoded_bytes: u32,
    generation: u32,
) -> Result<tauri::ipc::Response, String> {
    require(&window, "overlay")?;
    host.recovery.lock().unwrap().require(generation)?;
    if !host.controller.lock().unwrap().sound_enabled {
        return Err("사운드가 꺼져 있습니다.".into());
    }
    let name = host
        .loaded
        .sound_files
        .get(&resource)
        .ok_or("사운드 리소스를 찾을 수 없습니다.")?;
    let asset = host.loaded.files[name].clone();
    let root = host.loaded.root.clone();
    let name = name.clone();
    let budget = max_decoded_bytes.min(8 * 1024 * 1024);
    if asset.size > u64::from(budget) {
        return Ok(tauri::ipc::Response::new(Vec::new()));
    }
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        static AUDIO_READ: Mutex<()> = Mutex::new(());
        let _reading = AUDIO_READ.try_lock().map_err(|e| e.to_string())?;
        let path = buddy_project::checked_path(&root, &name).map_err(|e| e.to_string())?;
        if path != asset.path {
            return Err("사운드 리소스 위치가 변경되었습니다.".into());
        }
        let bytes = asset.read_verified().map_err(|e| e.to_string())?;
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        Ok::<_, String>(
            if buddy_project::sound_fits_decode_budget(extension, &bytes, sample_rate, budget) {
                bytes
            } else {
                Vec::new()
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}
#[tauri::command]
fn subscribe_character_state(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    channel: Channel<WireState>,
    generation: u32,
) -> Result<(), String> {
    require(&window, "overlay")?;
    let hidden = host.controller.lock().unwrap().mode == Mode::Hidden;
    {
        let mut target = host.channel.lock().unwrap();
        let mut runtime = host.runtime.lock().unwrap();
        let mut recovery = host.recovery.lock().unwrap();
        recovery.require(generation)?;
        recovery.connected(hidden);
        runtime.frame_reconnect();
        *host.hit_mask.lock().unwrap() = hit_test::HitMask::default();
        *target = Some(channel);
    }
    host.send_state();
    host.wake.wake();
    Ok(())
}
#[tauri::command]
fn ack_character_frame(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    ack: FrameAck,
    generation: u32,
) -> Result<(), String> {
    require(&window, "overlay")?;
    host.recovery.lock().unwrap().require(generation)?;
    if !host.runtime.lock().unwrap().accepts_frame(&ack) {
        host.runtime
            .lock()
            .unwrap()
            .ack_frame(host.now(), &ack, false);
        return Err("발행되지 않았거나 이미 처리된 프레임입니다.".into());
    }
    let visible = host.controller.lock().unwrap().mode != Mode::Hidden;
    let ready = generation == 0 || ack.outcome == FrameOutcome::Drawn;
    if visible && ready && !window.is_visible().map_err(|e| e.to_string())? {
        window.show().map_err(|e| e.to_string())?;
    }
    {
        let mut runtime = host.runtime.lock().unwrap();
        let mut recovery = host.recovery.lock().unwrap();
        recovery.require(generation)?;
        if !runtime.ack_frame(host.now(), &ack, visible) {
            return Err("발행되지 않았거나 이미 처리된 프레임입니다.".into());
        }
        if ready {
            host.overlay_ready.store(true, Ordering::Release);
            recovery.complete();
        }
    }
    if visible && ready {
        host.record_visibility(true);
    }
    host.wake.wake();
    host.hit_test_wake.wake();
    Ok(())
}
#[tauri::command]
fn set_character_hit_mask(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    mask: hit_test::HitMask,
    generation: u32,
) -> Result<(), String> {
    require(&window, "overlay")?;
    let recovery = host.recovery.lock().unwrap();
    recovery.require(generation)?;
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
        sound_enabled: c.sound_enabled,
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
    if let Some(value) = update.sound_enabled {
        prefs.sound_enabled = Some(value);
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
    generation: u32,
) -> Result<String, String> {
    require(&window, "overlay")?;
    host.recovery.lock().unwrap().require(generation)?;
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
fn character_click(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    held_ms: Option<u32>,
    generation: u32,
) -> Result<(), String> {
    require(&window, "overlay")?;
    let controller = host.controller.lock().unwrap();
    let recovery = host.recovery.lock().unwrap();
    recovery.require(generation)?;
    if recovery.deadline.is_some() {
        return Ok(());
    }
    if controller.mode == Mode::Normal && !controller.settings_open && !controller.paused {
        let is_hold = held_ms.is_some_and(|held| host.loaded.data.project.bindings.iter().any(|binding| {
            matches!(binding.trigger, buddy_project::Trigger::BuddyLongPress { hold_ms } if held >= hold_ms)
        }));
        let event = if is_hold {
            InputEvent::BuddyLongPress
        } else {
            InputEvent::BuddyClick
        };
        host.queue_overlay_input(controller.revision, event);
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

#[tauri::command]
fn get_timer(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
) -> Result<timer::TimerView, String> {
    if !matches!(window.label(), "timer-badge" | timer_quick::LABEL) {
        require(&window, "timer")?;
    }
    host.timer_view()
}

#[tauri::command]
async fn timer_action(
    window: WebviewWindow,
    host: State<'_, Arc<Host>>,
    action: timer::Action,
) -> Result<timer::TimerView, String> {
    require(&window, "timer")?;
    host.apply_timer_action(action)
}

#[tauri::command]
fn open_timer_from_badge(window: WebviewWindow, host: State<'_, Arc<Host>>) -> Result<(), String> {
    require(&window, timer::BADGE_LABEL)?;
    host.queue_timer_menu("timer-open", 0);
    Ok(())
}

fn start_worker(app: tauri::AppHandle, host: Arc<Host>) {
    std::thread::Builder::new()
        .name("buddy-runtime-host".into())
        .spawn(move || {
            let mut input: Option<InputSession> = None;
            let mut input_revision = u64::MAX;
            let mut last_state: Option<CharacterState> = None;
            let mut last_sent = 0;
            let mut last_visibility = None;
            let mut last_size = Point::default();
            let mut sent_mode = None;
            let mut sent_size = Point::default();
            let mut sent_canvas_offset = None;
            let mut sent_sound_enabled = None;
            let mut sent_edit_id: Option<String> = None;
            let mut last_top = None;
            let mut last_window_placement: Option<(Display, Point)> = None;
            let mut last_window_size = None;
            let mut last_geometry_revision = 0;
            let mut last_reload_generation = 0;
            let mut monitor_check = 0;
            let mut last_menu = String::new();
            let mut last_timer = None;
            let mut last_timer_menu = None;
            let mut badge_placement = None;
            let mut badge_input = None;
            let mut badge_top = None;
            let mut badge_closing = false;
            let mut badge_loading: Option<mpsc::Receiver<bool>> = None;
            let mut retry_badge_at = 0;
            while !host.exiting.load(Ordering::Acquire) {
                host.schedule_timer_menu_requests(&app);
                let c = host.controller.lock().unwrap().clone();
                let now = host.now();
                let (generation, recovery_deadline) = {
                    let mut recovery = host.recovery.lock().unwrap();
                    recovery.poll(now, c.mode == Mode::Hidden);
                    (recovery.generation, recovery.deadline)
                };
                let recovering = recovery_deadline.is_some();
                let reload_requested = generation != last_reload_generation && recovering;
                let active = c.mode == Mode::Normal && !c.settings_open && !c.paused && !recovering;
                let quick_open = host.quick_timer.lock().unwrap().is_open();
                let capture_active = active && !quick_open;
                let geometry_revision = host.overlay_geometry_revision.load(Ordering::Acquire);
                let geometry_changed = geometry_revision != last_geometry_revision;
                let displays_changed =
                    if reload_requested || geometry_changed || now >= monitor_check {
                        monitor_check = now + 2000;
                        host.refresh_displays(&app)
                    } else {
                        false
                    };
                if input_revision != c.revision || !capture_active {
                    input = None;
                    input_revision = c.revision;
                }
                if capture_active && (c.keyboard || c.mouse) && input.is_none() {
                    match InputSession::start_for_player(
                        InputOptions {
                            keyboard: c.keyboard,
                            mouse: c.mouse,
                        },
                        host.wake.clone(),
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
                if !capture_active || (!c.keyboard && !c.mouse) {
                    let reason = if c.mode == Mode::Hidden {
                        "캐릭터를 숨겨 감지가 중지됐습니다."
                    } else if recovering {
                        "버디 화면을 다시 불러오고 있습니다."
                    } else if c.mode == Mode::Editing {
                        "위치 편집 중에는 감지가 중지됩니다."
                    } else if c.settings_open {
                        "설정 창을 닫으면 감지를 시작합니다."
                    } else if quick_open {
                        "타이머 팝업을 닫으면 감지를 시작합니다."
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
                    c.cursor_follow && !recovering,
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
                for (revision, received_at, event) in host.overlay_inputs.lock().unwrap().drain(..)
                {
                    if active && revision == c.revision {
                        runtime.handle_received(host.now(), received_at, event);
                    }
                }
                let timer_update = host.timer.as_ref().map(|timer| {
                    let mut timer = timer.lock().unwrap();
                    let wall = timer::now_ms();
                    let pending = runtime.timer_reaction_pending();
                    let ready = host.overlay_ready.load(Ordering::Acquire);
                    let visible = (timer.due(wall) || pending || timer.signal.is_some())
                        && (ready || recovering)
                        && Host::timer_interactions_allowed(&c);
                    if pending && !visible {
                        runtime.cancel_timer_reaction();
                    }
                    timer.poll(wall, visible);
                    runtime.set_timer_phase(host.now(), timer.running_phase());
                    // Keep a fresh completion while the renderer reconnects. A
                    // timer action can still replace or cancel this signal.
                    match timer.take_signal(visible && recovering && !ready) {
                        Some(timer::Signal::Start(phase)) => {
                            runtime.cancel_timer_reaction();
                            if visible {
                                runtime.handle(host.now(), InputEvent::TimerStarted { phase });
                            }
                        }
                        Some(timer::Signal::Complete(phase)) if visible => {
                            runtime.handle(host.now(), InputEvent::TimerCompleted { phase })
                        }
                        Some(timer::Signal::Cancel) => runtime.cancel_timer_reaction(),
                        _ => {}
                    }
                    (timer.view(wall), timer.wake_after(wall))
                });
                runtime.tick(host.now());
                let state = runtime.state();
                let deadline = runtime.next_deadline_ms();
                drop(runtime);
                drop(controller_guard);
                let canvas_position = host.render_position(&display, size, &state);
                let window_origin =
                    display
                        .bounds
                        .clamp(canvas_position.x, canvas_position.y, size);
                let canvas_offset = Point {
                    x: canvas_position.x - window_origin.x,
                    y: canvas_position.y - window_origin.y,
                };
                if let Some(window) = app.get_webview_window("overlay") {
                    let windows = cfg!(target_os = "windows");
                    let desired_size = display.window_size(size, windows);
                    let mut sync_size = || {
                        if last_window_size != Some(desired_size)
                            || geometry_changed
                            || reload_requested
                        {
                            last_window_size = display
                                .sync_window_size(&window, size)
                                .ok()
                                .map(|_| desired_size);
                        }
                    };
                    if !windows {
                        sync_size();
                    }
                    // Keep the native window inside the work area. Transparent
                    // padding on the canvas can extend past the window edges.
                    let placement_changed = last_window_placement.as_ref().is_none_or(
                        |(placed_display, placed_position)| {
                            placed_display != &display || *placed_position != window_origin
                        },
                    );
                    let position_shifted = geometry_changed
                        && window.outer_position().ok()
                            != Some(display.physical_position(window_origin));
                    if placement_changed || position_shifted || reload_requested {
                        last_window_placement = display
                            .position_window(&window, window_origin)
                            .ok()
                            .map(|_| (display.clone(), window_origin));
                    }
                    // On Windows the move can trigger a DPI change. Size against
                    // the target monitor, then check the live client size again
                    // when the native window reports a resize or scale change.
                    if windows {
                        sync_size();
                    }
                    last_geometry_revision = geometry_revision;
                    let visible = c.mode != Mode::Hidden;
                    if (!visible || host.overlay_ready.load(Ordering::Acquire))
                        && (last_visibility != Some(visible) || geometry_changed)
                    {
                        let result = window.is_visible().and_then(|actual| {
                            if actual == visible {
                                Ok(())
                            } else if visible {
                                window.show()
                            } else {
                                window.hide()
                            }
                        });
                        last_visibility = result.is_ok().then_some(visible);
                        if result.is_ok() {
                            host.record_visibility(visible);
                        }
                    }
                    if last_top != Some(c.always_on_top) {
                        let _ = window.set_always_on_top(c.always_on_top);
                        last_top = Some(c.always_on_top);
                    }
                }
                if reload_requested {
                    last_reload_generation = generation;
                    if let Some(popup) = app.get_webview_window(timer_quick::LABEL) {
                        let _ = timer_quick::dismiss(&popup, &host, None);
                    }
                    // A new local URL gives WebKit and WebView2 a fresh
                    // navigation and ties all renderer callbacks to this load.
                    let reloading = app.get_webview_window("overlay").is_some_and(|window| {
                        navigation::overlay_recovery_url(generation)
                            .is_ok_and(|url| window.navigate(url).is_ok())
                    });
                    if !reloading {
                        host.recovery.lock().unwrap().fail();
                        host.wake.wake();
                    }
                } else if quick_open && (geometry_changed || displays_changed) {
                    let main = app.clone();
                    let popup_host = host.clone();
                    let _ = app.run_on_main_thread(move || timer_quick::refit(&main, &popup_host));
                }
                let changed = last_state.as_ref().is_none_or(|old| {
                    old.action_instance_id != state.action_instance_id
                        || old.position != state.position
                        || old.facing_left != state.facing_left
                }) || sent_mode != Some(c.mode)
                    || size != sent_size
                    || sent_canvas_offset != Some(canvas_offset)
                    || sent_sound_enabled != Some(c.sound_enabled)
                    || sent_edit_id.as_deref()
                        != c.edit.as_ref().map(|session| session.id.as_str());
                let mut next = Some(deadline.map_or(monitor_check, |due| due.min(monitor_check)));
                if let Some(due) = recovery_deadline {
                    next = next.map(|next| next.min(due));
                }
                if let Some((view, wake)) = timer_update {
                    if let Some(delay) = wake {
                        let due = host.now().saturating_add(delay);
                        next = Some(next.map_or(due, |next| next.min(due)));
                    }
                    if last_timer.as_ref() != Some(&view) {
                        for label in [timer::WINDOW_LABEL, timer::BADGE_LABEL] {
                            let _ = app.emit_to(label, "timer-view", &view);
                        }
                        let model = view.menu();
                        if last_timer_menu.as_ref() != Some(&model) {
                            let main = app.clone();
                            let update = model.clone();
                            if app
                                .run_on_main_thread(move || {
                                    if let Some(items) = main.try_state::<TrayControls>() {
                                        if let Some(menu) = &items.timer {
                                            let _ = menu.update(&main, &items.menu, &update);
                                        }
                                    }
                                })
                                .is_ok()
                            {
                                last_timer_menu = Some(model);
                            }
                        }
                    }
                    let existing_badge = app.get_webview_window(timer::BADGE_LABEL);
                    if let Some(loading) = &badge_loading {
                        match loading.try_recv() {
                            Ok(true) => badge_loading = None,
                            Ok(false) | Err(mpsc::TryRecvError::Disconnected) => {
                                badge_loading = None;
                                retry_badge_at = now + 1000;
                            }
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                    }
                    if existing_badge.is_none() {
                        badge_closing = false;
                        badge_placement = None;
                        badge_input = None;
                        badge_top = None;
                    }
                    let placement = if view.badge.is_some()
                        && c.mode != Mode::Hidden
                        && host.overlay_ready.load(Ordering::Acquire)
                    {
                        last_window_placement
                            .as_ref()
                            .filter(|(placed, origin)| {
                                placed == &display && *origin == window_origin
                            })
                            .and_then(|_| {
                                // The canvas may extend past the native window's
                                // edges when its transparent padding is clipped.
                                timer::badge_position(
                                    display.bounds,
                                    canvas_position,
                                    size,
                                    host.render_footprint(&state),
                                )
                            })
                    } else {
                        None
                    };
                    if !badge_closing {
                        if let Some(placement) = placement {
                            if let Some(badge) = existing_badge {
                                let position = placement.origin;
                                let clickable = c.mode == Mode::Normal
                                    && !c.settings_open
                                    && !placement.overlaps_buddy;
                                if badge_input != Some(clickable)
                                    && badge.set_ignore_cursor_events(!clickable).is_ok()
                                {
                                    badge_input = Some(clickable);
                                }
                                if badge_top != Some(c.always_on_top)
                                    && badge.set_always_on_top(c.always_on_top).is_ok()
                                {
                                    badge_top = Some(c.always_on_top);
                                }
                                let previous = badge_placement.as_ref();
                                let target_changed =
                                    previous.is_none_or(|(placed, _)| placed != &display);
                                let position_changed =
                                    previous.is_none_or(|(_, point)| *point != position);
                                let check_geometry =
                                    cfg!(target_os = "windows") && geometry_changed;
                                if target_changed || position_changed || check_geometry {
                                    let in_place = !target_changed
                                        && !position_changed
                                        && check_geometry
                                        && badge.outer_position().ok()
                                            == Some(display.physical_position(position));
                                    let positioned = in_place
                                        || display.position_window(&badge, position).is_ok();
                                    let sized = !cfg!(target_os = "windows")
                                        || !(target_changed || check_geometry)
                                        || display
                                            .sync_window_size(&badge, timer::BADGE_SIZE)
                                            .is_ok();
                                    if positioned
                                        && sized
                                        && (previous.is_some() || badge.show().is_ok())
                                    {
                                        badge_placement = Some((display.clone(), position));
                                    }
                                }
                            } else if badge_loading.is_none() && now >= retry_badge_at {
                                let (send, receive) = mpsc::channel();
                                let main = app.clone();
                                let wake = host.wake.clone();
                                badge_loading = Some(receive);
                                tauri::async_runtime::spawn_blocking(move || {
                                    let _ = send.send(timer::create_badge(&main).is_ok());
                                    wake.wake();
                                });
                            }
                        } else if let Some(badge) = existing_badge {
                            badge_closing = badge.destroy().is_ok();
                        } else {
                            retry_badge_at = 0;
                        }
                    }
                    last_timer = Some(view);
                }
                if changed && now.saturating_sub(last_sent) >= 34 {
                    host.send_state();
                    last_sent = now;
                    last_state = Some(state);
                    sent_mode = Some(c.mode);
                    sent_size = size;
                    sent_canvas_offset = Some(canvas_offset);
                    sent_sound_enabled = Some(c.sound_enabled);
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
                let input_text = format!(
                    "키보드 {} · 마우스 {}",
                    capture(c.keyboard, &source_status.keyboard),
                    capture(c.mouse, &source_status.mouse)
                );
                let text = host
                    .recovery
                    .lock()
                    .unwrap()
                    .menu_status()
                    .unwrap_or(&input_text);
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
        "timer-open" => {
            host.queue_timer_menu("timer-open", 0);
            Ok(())
        }
        id if matches!(
            id,
            "timer-pomodoro" | "timer-toggle" | "timer-end" | "timer-custom"
        ) || id.starts_with(timer_menu::PRESET_PREFIX) =>
        {
            app.try_state::<TrayControls>()
                .and_then(|items| items.timer.as_ref().map(timer_menu::TimerMenu::revision))
                .map_or(Ok(()), |revision| {
                    host.queue_timer_menu(id, revision);
                    Ok(())
                })
        }
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
            host.hit_test_wake.wake();
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
        "reload-overlay" => {
            host.request_overlay_recovery();
            Ok(())
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
            host.request_exit();
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
            read_render_frame,
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
            get_timer,
            timer_action,
            open_timer_from_badge,
            timer_quick::get_timer_quick,
            timer_quick::finish_timer_quick,
            show_buddy_menu
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let (project, bundled_project) = project_path(app.handle())?;
            let prepared = app.path().resource_dir()?.join("prepared-render");
            let mut loaded = buddy_project::load_player_project(project, &prepared)?;
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
                sound_enabled: prefs.sound_enabled.unwrap_or(true),
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
            let mut runtime = Runtime::new(
                loaded.data.clone(),
                bounds,
                Point::default(),
                Point { x: 256.0, y: 256.0 },
            );
            runtime.set_overlay_visible(0, false);
            let mut render_paths = std::collections::BTreeMap::new();
            if loaded.prepared_render_paths.is_empty() {
                let media_directory = directory.join("render-cache").join(&loaded.content_hash);
                fs::create_dir_all(&media_directory)?;
                for (name, bytes) in std::mem::take(&mut loaded.render_assets) {
                    let path = media_directory.join(&name);
                    fs::write(&path, bytes)?;
                    app.asset_protocol_scope().allow_file(&path)?;
                    render_paths.insert(name, path.to_string_lossy().into_owned());
                }
            } else {
                for (name, path) in &loaded.prepared_render_paths {
                    app.asset_protocol_scope().allow_file(path)?;
                    render_paths.insert(name.clone(), path.to_string_lossy().into_owned());
                }
            }
            let mut sound_paths = std::collections::BTreeMap::new();
            loaded.sound_files.retain(|id, _| {
                loaded.data.project.bindings.iter().any(|binding| {
                    binding
                        .sound
                        .as_ref()
                        .is_some_and(|cue| cue.volume > 0.0 && cue.resource == *id)
                })
            });
            let mut sound_assets = std::collections::BTreeMap::new();
            for (id, name) in &loaded.sound_files {
                let asset = &loaded.files[name];
                app.asset_protocol_scope().allow_file(&asset.path)?;
                sound_paths.insert(id.clone(), asset.path.to_string_lossy().into_owned());
                sound_assets
                    .entry(name.clone())
                    .or_insert_with(|| asset.clone());
            }
            loaded.files = sound_assets;
            let timer = loaded
                .data
                .project
                .timer
                .as_ref()
                .filter(|timer| timer.enabled)
                .map(|defaults| {
                    Mutex::new(timer::Timer::open(
                        &directory,
                        defaults.clone(),
                        timer::now_ms(),
                    ))
                });
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
                overlay_ready: AtomicBool::new(false),
                recovery: Mutex::default(),
                overlay_geometry_revision: AtomicU32::new(0),
                sequence: AtomicU32::new(0),
                wake: WakeSignal::default(),
                hit_test_wake: WakeSignal::default(),
                started: process_started,
                data_directory: directory,
                exiting: AtomicBool::new(false),
                display: Mutex::new(selected),
                monitors: Mutex::new(monitors),
                _instance_lock: lock,
                edit_operation: Mutex::new(()),
                overlay_inputs: Mutex::new(Vec::new()),
                hit_mask: Mutex::default(),
                follow_target: Mutex::default(),
                menu_open: AtomicBool::new(false),
                timer,
                timer_menu_requests: Mutex::default(),
                timer_menu_busy: AtomicBool::new(false),
                timer_window_closing: AtomicBool::new(false),
                quick_timer: Mutex::default(),
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
            app.manage(host.clone());
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("index.html".into()),
            )
            .on_navigation(navigation::allow_overlay_navigation)
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
            let menu = Menu::new(app)?;
            let status =
                MenuItem::with_id(app, "status", "입력 감지 준비 중", false, None::<&str>)?;
            let visible =
                CheckMenuItem::with_id(app, "visibility", "버디 표시", true, true, None::<&str>)?;
            let pause =
                CheckMenuItem::with_id(app, "pause", "입력 일시 정지", true, false, None::<&str>)?;
            menu.append_items(&[
                &visible,
                &pause,
                &status,
                &PredefinedMenuItem::separator(app)?,
            ])?;
            let timer_items = if let Some(timer) = &host.timer {
                let model = timer.lock().unwrap().view(timer::now_ms()).menu();
                let items = timer_menu::TimerMenu::new(app.handle(), &menu, &model)?;
                menu.append(&PredefinedMenuItem::separator(app)?)?;
                Some(items)
            } else {
                None
            };
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
            let options = Submenu::with_items(
                app,
                "버디 옵션",
                true,
                &[
                    &keyboard,
                    &mouse,
                    &movement,
                    &cursor_follow,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(
                        app,
                        "reload-overlay",
                        "현재 위치에서 다시 불러오기",
                        true,
                        None::<&str>,
                    )?,
                    &MenuItem::with_id(app, "reset-position", "위치 초기화", true, None::<&str>)?,
                ],
            )?;
            menu.append_items(&[
                &options,
                &MenuItem::with_id(app, "settings", "설정 · 입력 권한", true, None::<&str>)?,
                &PredefinedMenuItem::separator(app)?,
                &MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?,
            ])?;
            app.manage(TrayControls {
                menu: menu.clone(),
                status,
                visible,
                pause,
                keyboard,
                mouse,
                movement,
                cursor_follow,
                timer: timer_items,
            });
            TrayIconBuilder::new()
                .icon(Image::from_bytes(include_bytes!("../icons/icon.png"))?)
                .tooltip(&host.loaded.data.project.product.name)
                .menu(&menu)
                .build(app)?;
            hit_test::start(window, host.clone());
            start_worker(app.handle().clone(), host);
            Ok(())
        })
        .on_window_event(|window, event| {
            if cfg!(target_os = "windows")
                && matches!(
                    window.label(),
                    "overlay" | timer::BADGE_LABEL | timer_quick::LABEL
                )
                && matches!(
                    event,
                    tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
                )
            {
                if let Some(host) = window.try_state::<Arc<Host>>() {
                    host.overlay_geometry_revision
                        .fetch_add(1, Ordering::Release);
                    host.wake.wake();
                }
            }
            if window.label() == timer_quick::LABEL
                && matches!(event, tauri::WindowEvent::Focused(false))
            {
                if let Some(host) = window.try_state::<Arc<Host>>() {
                    if let Some(popup) = window.get_webview_window(timer_quick::LABEL) {
                        let _ = timer_quick::dismiss(&popup, &host, None);
                    }
                }
            }
            if window.label() == timer::WINDOW_LABEL
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                if let Some(host) = window.try_state::<Arc<Host>>() {
                    host.timer_window_closing.store(true, Ordering::Release);
                }
            }

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
        let Some(host) = app.try_state::<Arc<Host>>() else {
            return;
        };
        match event {
            tauri::RunEvent::Exit => host.request_exit(),
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } => {
                // Tauri unregisters the old label before delivering this event.
                match label.as_str() {
                    timer::WINDOW_LABEL => {
                        host.timer_window_closing.store(false, Ordering::Release)
                    }
                    timer_quick::LABEL => host.quick_timer.lock().unwrap().closed(),
                    timer::BADGE_LABEL => {}
                    _ => return,
                }
                host.wake.wake();
            }
            _ => {}
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
            sound_enabled: true,
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
            overlay_ready: AtomicBool::new(false),
            recovery: Mutex::default(),
            overlay_geometry_revision: AtomicU32::new(0),
            sequence: AtomicU32::new(0),
            wake: WakeSignal::default(),
            hit_test_wake: WakeSignal::default(),
            started: Instant::now(),
            data_directory: directory.to_path_buf(),
            exiting: AtomicBool::new(false),
            display: Mutex::new(display.clone()),
            monitors: Mutex::new(vec![display, external]),
            _instance_lock: File::create(directory.join("instance.lock")).unwrap(),
            edit_operation: Mutex::new(()),
            overlay_inputs: Mutex::new(Vec::new()),
            hit_mask: Mutex::default(),
            follow_target: Mutex::default(),
            menu_open: AtomicBool::new(false),
            timer: None,
            timer_menu_requests: Mutex::default(),
            timer_menu_busy: AtomicBool::new(false),
            timer_window_closing: AtomicBool::new(false),
            quick_timer: Mutex::default(),
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
        let inputs = host.overlay_inputs.lock().unwrap();
        assert!(matches!(
            inputs.as_slice(),
            [(_, _, InputEvent::DragDropped)]
        ));
        let (_, prefs) = PreferencesStore::open(temp.path()).unwrap();
        let placement = prefs.base_placement.unwrap();
        assert_eq!(placement.monitor_hint, "external");
        assert_eq!(placement.position, DROP_ON_EXTERNAL);
    }

    #[test]
    fn recovery_keeps_a_cross_display_drag_position_without_persisting_it() {
        let temp = tempfile::tempdir().unwrap();
        let host = drag_host(temp.path(), false);
        let session = host.begin_edit(GRAB).unwrap();
        host.drag_to_cursor(&session, CURSOR_ON_EXTERNAL, true)
            .unwrap();
        host.request_overlay_recovery();
        assert!(host.recovering());
        assert_eq!(host.display.lock().unwrap().id, "external");
        assert_eq!(
            host.runtime.lock().unwrap().state().position,
            DROP_ON_EXTERNAL
        );
        assert!(host.controller.lock().unwrap().edit.is_none());
        assert!(host.overlay_inputs.lock().unwrap().is_empty());
        assert!(PreferencesStore::open(temp.path())
            .unwrap()
            .1
            .base_placement
            .is_none());
        assert!(host.finish_edit(true, Some(&session)).is_err());
    }

    #[test]
    fn dragging_to_a_smaller_display_keeps_the_same_relative_grab_point() {
        let temp = tempfile::tempdir().unwrap();
        let host = drag_host(temp.path(), false);
        let size = host.size(1.0);
        {
            let mut monitors = host.monitors.lock().unwrap();
            monitors[1].bounds.height = 150.0;
        }
        let grab = Point {
            x: size.x * 0.75,
            y: size.y * 0.75,
        };
        let session = host.begin_edit(grab).unwrap();
        let target = host.monitors.lock().unwrap()[1].clone();
        let fitted = target.fit_size(host.loaded.canvas_size(1.0));
        let expected = host
            .placement_bounds(&target, fitted)
            .clamp(100.0, 10.0, fitted);
        let local = Point {
            x: expected.x + fitted.x * 0.75,
            y: expected.y + fitted.y * 0.75,
        };
        let cursor = Point {
            x: (target.origin.x + local.x) * 2.0,
            y: (target.origin.y + local.y) * 2.0,
        };
        host.drag_to_cursor(&session, cursor, true).unwrap();
        assert_eq!(host.display.lock().unwrap().id, target.id);
        assert!(fitted.x < size.x);
        let position = host.runtime.lock().unwrap().state().position;
        assert!((local.x - position.x - fitted.x * 0.75).abs() < 0.0001);
        assert!((local.y - position.y - fitted.y * 0.75).abs() < 0.0001);
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
        assert!(host.overlay_inputs.lock().unwrap().is_empty());
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
