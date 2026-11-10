use crate::timer::{menu_time, MenuView};
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Wry,
};

pub const PRESET_PREFIX: &str = "timer-preset-";
const CUSTOM: &str = "timer-custom";

pub struct TimerMenu {
    pomodoro: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    end: MenuItem<Wry>,
    countdown: Submenu<Wry>,
    revision: AtomicU32,
}

impl TimerMenu {
    pub fn new(app: &AppHandle, menu: &Menu<Wry>, view: &MenuView) -> tauri::Result<Self> {
        let timer = Self {
            pomodoro: MenuItem::with_id(app, "timer-pomodoro", &view.pomodoro, true, None::<&str>)?,
            toggle: MenuItem::with_id(app, "timer-toggle", "", true, None::<&str>)?,
            end: MenuItem::with_id(app, "timer-end", "타이머 종료…", true, None::<&str>)?,
            countdown: Submenu::with_id(app, "timer-countdown", "일반 타이머", true)?,
            revision: AtomicU32::new(view.revision),
        };
        timer.countdown.append(&MenuItem::with_id(
            app,
            CUSTOM,
            "사용자 지정…",
            true,
            None::<&str>,
        )?)?;
        menu.append_items(&[
            &timer.pomodoro,
            &timer.countdown,
            &MenuItem::with_id(app, "timer-open", "타이머 창 열기", true, None::<&str>)?,
        ])?;
        timer.update(app, menu, view)?;
        Ok(timer)
    }

    pub fn revision(&self) -> u32 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn update(&self, app: &AppHandle, menu: &Menu<Wry>, view: &MenuView) -> tauri::Result<()> {
        self.pomodoro.set_text(&view.pomodoro)?;
        if let Some(label) = view.countdown_action {
            self.toggle.set_text(label)?;
        }
        // Inserting the end action first keeps the toggle next to Pomodoro
        // when a replacement switches between the two timer modes.
        self.set_action(menu, &self.end, view.can_end)?;
        self.set_action(menu, &self.toggle, view.countdown_action.is_some())?;
        self.update_presets(app, &view.presets)?;
        self.revision.store(view.revision, Ordering::Release);
        Ok(())
    }

    fn set_action(
        &self,
        menu: &Menu<Wry>,
        action: &MenuItem<Wry>,
        visible: bool,
    ) -> tauri::Result<()> {
        let items = menu.items()?;
        let present = items.iter().any(|item| item.id() == action.id());
        if visible && !present {
            if let Some(index) = items
                .iter()
                .position(|item| item.id() == self.pomodoro.id())
            {
                menu.insert(action, index + 1)?;
            }
        } else if !visible && present {
            menu.remove(action)?;
        }
        Ok(())
    }

    fn update_presets(&self, app: &AppHandle, presets: &[u32]) -> tauri::Result<()> {
        let items = self.countdown.items()?;
        let current = items.iter().filter_map(|item| {
            item.id()
                .as_ref()
                .strip_prefix(PRESET_PREFIX)?
                .parse::<u32>()
                .ok()
        });
        if current.eq(presets.iter().copied()) {
            return Ok(());
        }
        let shortcuts = presets
            .iter()
            .map(|&seconds| {
                MenuItem::with_id(
                    app,
                    format!("{PRESET_PREFIX}{seconds}"),
                    menu_time(seconds),
                    true,
                    None::<&str>,
                )
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        for item in items.iter().filter(|item| item.id().as_ref() != CUSTOM) {
            self.countdown.remove(item)?;
        }
        for (index, item) in shortcuts.iter().enumerate() {
            self.countdown.insert(item, index)?;
        }
        if !shortcuts.is_empty() {
            self.countdown
                .insert(&PredefinedMenuItem::separator(app)?, shortcuts.len())?;
        }
        Ok(())
    }
}
