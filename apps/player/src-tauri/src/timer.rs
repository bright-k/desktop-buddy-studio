use crate::preferences::atomic_write;
use buddy_project::{
    valid_timer_presets, Point, PomodoroSettings, TimerCompletion, TimerDefaults, TimerMode,
    TimerPhase, DEFAULT_TIMER_PRESETS, MAX_TIMER_PRESETS,
};
use buddy_runtime::Rect;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const BADGE_SIZE: Point = Point { x: 116.0, y: 46.0 };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Countdown,
    Focus,
    ShortBreak,
    LongBreak,
}
impl Phase {
    fn mode(self) -> TimerMode {
        match self {
            Self::Countdown => TimerMode::Countdown,
            _ => TimerMode::Pomodoro,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Countdown => "일반 타이머",
            Self::Focus => "집중",
            Self::ShortBreak => "짧은 휴식",
            Self::LongBreak => "긴 휴식",
        }
    }

    fn reaction_phase(self) -> TimerPhase {
        match self {
            Self::Countdown => TimerPhase::Countdown,
            Self::Focus => TimerPhase::Focus,
            Self::ShortBreak => TimerPhase::ShortBreak,
            Self::LongBreak => TimerPhase::LongBreak,
        }
    }

    fn seconds(self, profile: PomodoroSettings) -> u32 {
        match self {
            Self::Focus => profile.focus_minutes * 60,
            Self::ShortBreak => profile.short_break_minutes * 60,
            Self::LongBreak => profile.long_break_minutes * 60,
            Self::Countdown => unreachable!("countdown has its own duration"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Clock {
    Ready,
    Running { ends_at: u64 },
    Paused { remaining_ms: u64 },
    Done,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Session {
    phase: Phase,
    duration_seconds: u32,
    clock: Clock,
    completed: u32,
    profile: Option<PomodoroSettings>,
}
impl Session {
    fn duration_ms(self) -> u64 {
        u64::from(self.duration_seconds) * 1000
    }

    fn remaining(self, now: u64) -> u64 {
        match self.clock {
            Clock::Ready => self.duration_ms(),
            Clock::Running { ends_at } => ends_at.saturating_sub(now).min(self.duration_ms()),
            Clock::Paused { remaining_ms } => remaining_ms,
            Clock::Done => 0,
        }
    }

    fn valid(self) -> bool {
        valid_duration(self.duration_seconds)
            && match self.phase {
                Phase::Countdown => self.profile.is_none() && self.completed == 0,
                phase => self.profile.is_some_and(|profile| {
                    profile.is_valid()
                        && self.completed <= profile.rounds
                        && phase.seconds(profile) == self.duration_seconds
                        && self.clock != Clock::Done
                }),
            }
            && match self.clock {
                Clock::Paused { remaining_ms } => (1..=self.duration_ms()).contains(&remaining_ms),
                _ => true,
            }
    }

    fn start(&mut self, now: u64) {
        self.clock = Clock::Running {
            ends_at: now.saturating_add(self.remaining(now)),
        };
    }

    fn next(&mut self, completed: bool) {
        let Some(profile) = self.profile else {
            self.clock = Clock::Done;
            return;
        };
        self.phase = match self.phase {
            Phase::Focus => {
                if completed {
                    self.completed += 1;
                }
                if self.completed >= profile.rounds {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::LongBreak => {
                self.completed = 0;
                Phase::Focus
            }
            _ => Phase::Focus,
        };
        self.duration_seconds = self.phase.seconds(profile);
        self.clock = Clock::Ready;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Outcome {
    phase: Phase,
    skipped: bool,
}
impl Outcome {
    pub fn text(self) -> String {
        format!(
            "{} {}",
            self.phase.name(),
            if self.skipped { "건너뜀" } else { "완료" }
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    version: u32,
    last_countdown: Option<u32>,
    presets: Option<Vec<u32>>,
    personal: Option<PomodoroSettings>,
    show_badge: bool,
    mode: TimerMode,
    session: Option<Session>,
    outcome: Option<Outcome>,
}
impl Saved {
    fn valid(&self) -> bool {
        matches!(self.version, 1 | 2)
            && self.last_countdown.is_none_or(valid_duration)
            && self.presets.as_deref().is_none_or(valid_timer_presets)
            && self.personal.is_none_or(PomodoroSettings::is_valid)
            && self.session.is_none_or(Session::valid)
    }
    fn migrate_presets(&mut self) {
        if self.version != 1 {
            return;
        }
        self.presets = self
            .presets
            .take()
            .filter(|presets| !presets.is_empty())
            .map(|mut presets| {
                for seconds in DEFAULT_TIMER_PRESETS {
                    if presets.len() < MAX_TIMER_PRESETS && !presets.contains(&seconds) {
                        presets.push(seconds);
                    }
                }
                presets
            });
        self.version = 2;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Status {
    Ready,
    Running,
    Paused,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    mode: TimerMode,
    phase: Phase,
    status: Status,
    duration_seconds: u32,
    remaining_seconds: u32,
    completed: u32,
    profile: Option<PomodoroSettings>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BadgeView {
    phase: Phase,
    label: String,
    time: String,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    pub revision: u32,
    pub storage_blocked: bool,
    mode: TimerMode,
    countdown_seconds: u32,
    presets: Vec<u32>,
    custom_presets: bool,
    default_profile: PomodoroSettings,
    personal: Option<PomodoroSettings>,
    show_badge: bool,
    pub session: Option<SessionView>,
    pub outcome: Option<Outcome>,
    pub badge: Option<BadgeView>,
    pub notice: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    ResetStorage,
    SelectMode {
        mode: TimerMode,
    },
    Start {
        mode: TimerMode,
        seconds: Option<u32>,
        replace_revision: Option<u32>,
    },
    Toggle {
        revision: u32,
    },
    Skip {
        revision: u32,
    },
    Reset {
        revision: u32,
    },
    End {
        revision: u32,
    },
    SavePreset {
        seconds: u32,
    },
    SetPresets {
        presets: Option<Vec<u32>>,
    },
    SetProfile {
        profile: Option<PomodoroSettings>,
    },
    ShowBadge {
        enabled: bool,
    },
}

pub enum Signal {
    Start(TimerPhase),
    Complete(TimerCompletion),
    Cancel,
}

pub struct Timer {
    defaults: TimerDefaults,
    saved: Saved,
    path: PathBuf,
    revision: u32,
    expired_revision: Option<u32>,
    badge_until: u64,
    last_poll: u64,
    pub signal: Option<Signal>,
    storage_blocked: bool,
    notice: Option<String>,
}

fn valid_duration(seconds: u32) -> bool {
    (1..=10_800).contains(&seconds)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn format_time(seconds: u32) -> String {
    if seconds >= 3600 {
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

pub fn menu_time(seconds: u32) -> String {
    [
        (seconds / 3600, "시간"),
        (seconds / 60 % 60, "분"),
        (seconds % 60, "초"),
    ]
    .into_iter()
    .filter(|(value, _)| *value > 0)
    .map(|(value, unit)| format!("{value}{unit}"))
    .collect::<Vec<_>>()
    .join(" ")
}

#[derive(Clone, PartialEq, Eq)]
pub struct MenuView {
    pub revision: u32,
    pub pomodoro: String,
    pub countdown_action: Option<&'static str>,
    pub can_end: bool,
    pub presets: Vec<u32>,
}

impl TimerView {
    pub fn active_mode(&self) -> Option<TimerMode> {
        self.session
            .filter(|timer| timer.status != Status::Done)
            .map(|timer| timer.mode)
    }

    pub fn countdown_seconds(&self) -> u32 {
        self.countdown_seconds
    }

    pub fn has_preset(&self, seconds: u32) -> bool {
        self.presets.contains(&seconds)
    }

    pub fn focus_seconds(&self) -> u32 {
        self.personal.unwrap_or(self.default_profile).focus_minutes * 60
    }

    pub fn menu(&self) -> MenuView {
        let active = self.session.filter(|timer| timer.status != Status::Done);
        let pomodoro = match active.filter(|timer| timer.mode == TimerMode::Pomodoro) {
            Some(timer) => {
                let phase = timer.phase.name();
                match timer.status {
                    Status::Running => format!("{phase} 일시정지"),
                    Status::Paused => format!("{phase} 재개"),
                    _ => format!("{phase} {} 시작", menu_time(timer.duration_seconds)),
                }
            }
            None => format!("집중 {} 시작", menu_time(self.focus_seconds())),
        };
        let countdown_action = active
            .filter(|timer| timer.mode == TimerMode::Countdown)
            .and_then(|timer| match timer.status {
                Status::Running => Some("일반 타이머 일시정지"),
                Status::Paused => Some("일반 타이머 재개"),
                _ => None,
            });
        MenuView {
            revision: self.revision,
            pomodoro: format!("뽀모도로 · {pomodoro}"),
            countdown_action,
            can_end: active.is_some(),
            presets: self.presets.clone(),
        }
    }
}

impl Timer {
    pub fn open(directory: &Path, defaults: TimerDefaults, now: u64) -> Self {
        let path = directory.join("timer.json");
        let (mut saved, storage_blocked) = match read_saved(&path) {
            Ok(saved) => (saved, false),
            Err(_) => (None, true),
        };
        let notice = storage_blocked
            .then(|| "타이머 저장 파일을 읽을 수 없어 원본을 보호하고 있습니다.".into());
        if let Some(saved) = &mut saved {
            saved.migrate_presets();
        }
        let mode = defaults.mode;
        let mut timer = Self {
            defaults,
            saved: saved.unwrap_or(Saved {
                version: 2,
                last_countdown: None,
                presets: None,
                personal: None,
                show_badge: true,
                mode,
                session: None,
                outcome: None,
            }),
            path,
            revision: 0,
            expired_revision: None,
            badge_until: 0,
            last_poll: now,
            signal: None,
            storage_blocked,
            notice,
        };
        timer.poll(now, false);
        timer
    }

    fn save(&self, next: &Saved) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(next).map_err(|e| e.to_string())?;
        atomic_write(&self.path, &bytes).map_err(|error| format!("타이머 저장 실패 · {error}"))
    }

    fn reset_storage(&mut self) -> Result<(), String> {
        if !self.storage_blocked {
            return Ok(());
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.is_file() || metadata.is_symlink() => {
                let archive = self
                    .path
                    .with_extension(format!("preserved-{}.json", uuid::Uuid::new_v4()));
                fs::rename(&self.path, archive)
                    .map_err(|error| format!("타이머 원본 보관 실패 · {error}"))?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return Err("타이머 저장 경로를 확인한 뒤 다시 시도하세요.".into()),
        }
        self.storage_blocked = false;
        self.notice = None;
        self.changed();
        Ok(())
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn expired_since(&self, revision: u32) -> bool {
        self.expired_revision == Some(revision) && self.revision == revision.wrapping_add(1)
    }

    pub fn poll(&mut self, now: u64, visible_completion: bool) {
        let last_poll = std::mem::replace(&mut self.last_poll, now);
        let Some(mut session) = self.saved.session else {
            return;
        };
        let Clock::Running { ends_at } = session.clock else {
            return;
        };
        if now < ends_at {
            if ends_at - now > session.duration_ms() {
                session.clock = Clock::Running {
                    ends_at: now + session.duration_ms(),
                };
                self.saved.session = Some(session);
                self.notice = self.save(&self.saved).err();
            }
            return;
        }
        let phase = session.phase;
        session.next(true);
        self.saved.session = Some(session);
        self.saved.outcome = Some(Outcome {
            phase,
            skipped: false,
        });
        self.expired_revision = Some(self.revision);
        self.changed();
        self.notice = self.save(&self.saved).err();
        if visible_completion && now - ends_at <= 1000 && now.saturating_sub(last_poll) <= 1000 {
            self.badge_until = now + 4000;
            self.signal = Some(Signal::Complete(phase.reaction_phase().into()));
        } else if matches!(self.signal, Some(Signal::Start(_))) {
            self.signal = Some(Signal::Cancel);
        }
    }

    pub fn take_signal(&mut self, defer: bool) -> Option<Signal> {
        if defer && !matches!(self.signal, Some(Signal::Cancel)) {
            None
        } else {
            self.signal.take()
        }
    }

    pub fn running_phase(&self) -> Option<TimerPhase> {
        self.saved
            .session
            .filter(|session| matches!(session.clock, Clock::Running { .. }))
            .map(|session| session.phase.reaction_phase())
    }

    pub fn due(&self, now: u64) -> bool {
        self.saved.session.is_some_and(
            |timer| matches!(timer.clock, Clock::Running { ends_at } if now >= ends_at),
        )
    }

    pub fn wake_after(&self, now: u64) -> Option<u64> {
        if let Some(Session {
            clock: Clock::Running { ends_at },
            ..
        }) = self.saved.session
        {
            let remaining = ends_at.saturating_sub(now);
            return Some((remaining.saturating_sub(1) % 1000 + 1).min(250));
        }
        (self.badge_until > now).then(|| self.badge_until - now)
    }

    fn countdown(&self) -> u32 {
        self.saved
            .last_countdown
            .unwrap_or(self.defaults.countdown_seconds)
    }

    fn presets(&self) -> &[u32] {
        self.saved
            .presets
            .as_deref()
            .unwrap_or_else(|| self.defaults.countdown_presets())
    }

    fn profile(&self) -> PomodoroSettings {
        self.saved.personal.unwrap_or(self.defaults.pomodoro)
    }

    pub fn view(&self, now: u64) -> TimerView {
        let session = self.saved.session.map(|timer| SessionView {
            mode: timer.phase.mode(),
            phase: timer.phase,
            status: match timer.clock {
                Clock::Ready => Status::Ready,
                Clock::Running { .. } => Status::Running,
                Clock::Paused { .. } => Status::Paused,
                Clock::Done => Status::Done,
            },
            duration_seconds: timer.duration_seconds,
            remaining_seconds: timer.remaining(now).div_ceil(1000) as u32,
            completed: timer.completed,
            profile: timer.profile,
        });
        let badge = self
            .saved
            .show_badge
            .then(|| {
                if self.badge_until > now {
                    self.saved.outcome.map(|outcome| BadgeView {
                        phase: outcome.phase,
                        label: outcome.text(),
                        time: format_time(0),
                    })
                } else {
                    session
                        .filter(|timer| matches!(timer.status, Status::Running | Status::Paused))
                        .map(|timer| {
                            let phase = match timer.phase {
                                Phase::Countdown => "타이머",
                                phase => phase.name(),
                            };
                            BadgeView {
                                phase: timer.phase,
                                label: if timer.status == Status::Paused {
                                    format!("{phase} · 일시정지")
                                } else {
                                    phase.into()
                                },
                                time: format_time(timer.remaining_seconds),
                            }
                        })
                }
            })
            .flatten();
        TimerView {
            revision: self.revision,
            storage_blocked: self.storage_blocked,
            mode: self.saved.mode,
            countdown_seconds: self.countdown(),
            presets: self.presets().to_vec(),
            custom_presets: self.saved.presets.is_some(),
            default_profile: self.defaults.pomodoro,
            personal: self.saved.personal,
            show_badge: self.saved.show_badge,
            session,
            outcome: self.saved.outcome,
            badge,
            notice: self.notice.clone(),
        }
    }

    fn new_session(&self, mode: TimerMode, seconds: Option<u32>) -> Result<Session, String> {
        let (phase, duration_seconds, profile) = match mode {
            TimerMode::Countdown => {
                let duration = seconds.unwrap_or_else(|| self.countdown());
                if !valid_duration(duration) {
                    return Err("시간을 1초부터 3시간 사이로 설정하세요.".into());
                }
                (Phase::Countdown, duration, None)
            }
            TimerMode::Pomodoro => (
                Phase::Focus,
                self.profile().focus_minutes * 60,
                Some(self.profile()),
            ),
        };
        Ok(Session {
            phase,
            duration_seconds,
            clock: Clock::Ready,
            completed: 0,
            profile,
        })
    }

    pub fn apply(&mut self, action: Action, now: u64) -> Result<(), String> {
        if self.storage_blocked {
            return match action {
                Action::ResetStorage => self.reset_storage(),
                _ => Err("타이머 창에서 원본을 보관하고 초기화한 뒤 사용하세요.".into()),
            };
        }
        let skipping = matches!(action, Action::Skip { .. });
        let mut next = self.saved.clone();
        let mut changed_session = false;
        let mut started = None;
        match action {
            Action::ResetStorage => return Ok(()),
            Action::SelectMode { mode } => {
                if next.session.is_some_and(|timer| timer.clock != Clock::Done) || next.mode == mode
                {
                    return Ok(());
                }
                next.mode = mode;
                next.session = None;
            }
            Action::Start {
                mode,
                seconds,
                replace_revision,
            } => {
                if replace_revision.is_some_and(|revision| {
                    revision != self.revision && !self.expired_since(revision)
                }) {
                    return Err("타이머 상태가 바뀌었습니다. 다시 확인하세요.".into());
                }
                if next.session.is_some_and(|timer| timer.clock != Clock::Done)
                    && replace_revision.is_none()
                {
                    return Err("진행 중인 타이머를 확인한 뒤 전환하세요.".into());
                }
                let mut session = self.new_session(mode, seconds)?;
                session.start(now);
                started = Some(session.phase.reaction_phase());
                if mode == TimerMode::Countdown {
                    next.last_countdown = Some(session.duration_seconds);
                }
                next.mode = mode;
                next.session = Some(session);
                changed_session = true;
            }
            Action::Toggle { revision }
            | Action::Skip { revision }
            | Action::Reset { revision }
            | Action::End { revision } => {
                let expired_end =
                    matches!(action, Action::End { .. }) && self.expired_since(revision);
                if self.revision != revision && !expired_end {
                    return Err("타이머 상태가 바뀌었습니다. 다시 확인하세요.".into());
                }
                if expired_end && next.session.is_none_or(|timer| timer.clock == Clock::Done) {
                    return Ok(());
                }
                match action {
                    Action::Toggle { .. } => {
                        let mut timer = match next.session {
                            Some(timer) => timer,
                            None => self.new_session(next.mode, None)?,
                        };
                        match timer.clock {
                            Clock::Running { .. } => {
                                let remaining_ms = timer.remaining(now);
                                if remaining_ms == 0 {
                                    return Err("타이머가 완료됐습니다. 다시 확인하세요.".into());
                                }
                                timer.clock = Clock::Paused { remaining_ms };
                            }
                            Clock::Done => {
                                timer.clock = Clock::Ready;
                                timer.start(now);
                                started = Some(timer.phase.reaction_phase());
                            }
                            Clock::Ready => {
                                timer.start(now);
                                started = Some(timer.phase.reaction_phase());
                            }
                            Clock::Paused { .. } => timer.start(now),
                        }
                        if timer.phase == Phase::Countdown {
                            next.last_countdown = Some(timer.duration_seconds);
                        }
                        next.mode = timer.phase.mode();
                        next.session = Some(timer);
                    }
                    Action::Skip { .. } => {
                        let mut timer = next
                            .session
                            .filter(|timer| timer.profile.is_some())
                            .ok_or("건너뛸 뽀모도로 단계가 없습니다.")?;
                        let phase = timer.phase;
                        timer.next(false);
                        if phase != Phase::Focus {
                            timer.start(now);
                            started = Some(timer.phase.reaction_phase());
                        }
                        next.session = Some(timer);
                        next.outcome = Some(Outcome {
                            phase,
                            skipped: true,
                        });
                    }
                    Action::Reset { .. } => {
                        next.session = next.session.filter(|timer| timer.profile.is_some()).map(
                            |mut timer| {
                                timer.clock = Clock::Ready;
                                timer
                            },
                        );
                    }
                    _ => next.session = None,
                }
                changed_session = true;
            }
            Action::SavePreset { seconds } => {
                if !valid_duration(seconds) {
                    return Err("저장할 시간을 1초부터 3시간 사이로 설정하세요.".into());
                }
                let mut presets = self.presets().to_vec();
                if presets.contains(&seconds) {
                    return Err("이미 등록된 시간입니다.".into());
                }
                if presets.len() == MAX_TIMER_PRESETS {
                    return Err(format!(
                        "시간은 최대 {MAX_TIMER_PRESETS}개까지 저장할 수 있습니다."
                    ));
                }
                presets.push(seconds);
                next.presets = Some(presets);
            }
            Action::SetPresets { presets } => {
                if presets
                    .as_deref()
                    .is_some_and(|presets| !valid_timer_presets(presets))
                {
                    return Err(format!(
                        "1초~3시간 사이의 서로 다른 시간을 최대 {MAX_TIMER_PRESETS}개 입력하세요."
                    ));
                }
                next.presets = presets;
            }
            Action::SetProfile { profile } => {
                if profile.is_some_and(|profile| !profile.is_valid()) {
                    return Err("뽀모도로 시간과 집중 횟수를 확인하세요.".into());
                }
                next.personal = profile;
            }
            Action::ShowBadge { enabled } => next.show_badge = enabled,
        }
        if changed_session && !skipping {
            next.outcome = None;
        }
        self.save(&next)?;
        self.saved = next;
        self.notice = None;
        if changed_session {
            self.changed();
            self.badge_until = 0;
            self.signal = Some(started.map_or(Signal::Cancel, Signal::Start));
        }
        Ok(())
    }
}

fn read_saved(path: &Path) -> Result<Option<Saved>, std::io::Error> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= 16_384 => {}
        Err(error) if error.kind() == ErrorKind::NotFound && !path.is_symlink() => return Ok(None),
        Err(error) => return Err(error),
        Ok(_) => {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "invalid timer file",
            ))
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
    let saved = (bytes.len() <= 16_384)
        .then(|| serde_json::from_slice::<Saved>(&bytes).ok())
        .flatten()
        .filter(Saved::valid)
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "invalid timer data"))?;
    Ok(Some(saved))
}

pub const WINDOW_LABEL: &str = "timer";
pub const BADGE_LABEL: &str = "timer-badge";

pub fn create_window(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    tauri::WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .on_navigation(crate::navigation::allow_player_navigation)
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .initialization_script("window.location.hash = 'timer';")
    .title("Desktop Buddy · 타이머")
    .inner_size(460.0, 580.0)
    .min_inner_size(340.0, 380.0)
    .visible(false)
    .focused(false)
    .build()
}

pub fn create_badge(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    tauri::WebviewWindowBuilder::new(
        app,
        BADGE_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .on_navigation(crate::navigation::allow_player_navigation)
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .initialization_script("window.location.hash = 'timer-badge';")
    .title("버디 타이머")
    .inner_size(BADGE_SIZE.x, BADGE_SIZE.y)
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .skip_taskbar(true)
    .visible_on_all_workspaces(cfg!(target_os = "macos"))
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePlacement {
    pub origin: Point,
    pub overlaps_buddy: bool,
    pub side: SurfaceSide,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceSide {
    Above,
    Below,
    Side,
}

pub fn badge_position(
    bounds: Rect,
    origin: Point,
    size: Point,
    footprint: Option<(Point, Point)>,
) -> Option<SurfacePlacement> {
    adjacent_position(bounds, origin, size, footprint, BADGE_SIZE, false)
}

pub fn adjacent_position(
    bounds: Rect,
    origin: Point,
    size: Point,
    footprint: Option<(Point, Point)>,
    surface: Point,
    above_first: bool,
) -> Option<SurfacePlacement> {
    const GAP: f64 = 6.0;
    if bounds.width < surface.x || bounds.height < surface.y {
        return None;
    }
    let inset_x = ((bounds.width - surface.x) / 2.0).min(4.0);
    let inset_y = ((bounds.height - surface.y) / 2.0).min(4.0);
    let safe = Rect {
        x: bounds.x + inset_x,
        y: bounds.y + inset_y,
        width: bounds.width - 2.0 * inset_x,
        height: bounds.height - 2.0 * inset_y,
    };
    let (min, max) = footprint.unwrap_or((Point::default(), Point { x: 1.0, y: 1.0 }));
    let left = origin.x + min.x * size.x;
    let right = origin.x + max.x * size.x;
    let top = origin.y + min.y * size.y;
    let bottom = origin.y + max.y * size.y;
    let center_x = (left + right - surface.x) / 2.0;
    let center_y = (top + bottom - surface.y) / 2.0;
    let max_x = safe.x + (safe.width - surface.x).max(0.0);
    let max_y = safe.y + (safe.height - surface.y).max(0.0);
    let below = bottom + GAP;
    let above = top - GAP - surface.y;
    let beside_right = right + GAP;
    let beside_left = left - GAP - surface.x;
    let mut candidates = [
        (center_x, below, below <= max_y, SurfaceSide::Below),
        (center_x, above, above >= safe.y, SurfaceSide::Above),
        (
            beside_right,
            center_y,
            beside_right <= max_x,
            SurfaceSide::Side,
        ),
        (
            beside_left,
            center_y,
            beside_left >= safe.x,
            SurfaceSide::Side,
        ),
    ];
    if above_first {
        candidates.swap(0, 1);
    }
    for (x, y, fits, side) in candidates {
        if fits {
            return Some(SurfacePlacement {
                origin: safe.clamp(x, y, surface),
                overlaps_buddy: false,
                side,
            });
        }
    }

    // A large sprite can leave no room on any side. Use the corner with the
    // least overlap, and let pointer events reach the buddy under the badge.
    let overlap = |x: f64, y: f64| {
        let width = (right.min(x + surface.x) - left.max(x)).max(0.0);
        let height = (bottom.min(y + surface.y) - top.max(y)).max(0.0);
        width * height
    };
    [
        Point { x: max_x, y: max_y },
        Point {
            x: safe.x,
            y: max_y,
        },
        Point {
            x: max_x,
            y: safe.y,
        },
        Point {
            x: safe.x,
            y: safe.y,
        },
    ]
    .into_iter()
    .map(|origin| (origin, overlap(origin.x, origin.y)))
    .min_by(|(_, a), (_, b)| a.total_cmp(b))
    .map(|(origin, area)| SurfacePlacement {
        origin,
        overlaps_buddy: area > 0.0,
        side: SurfaceSide::Side,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle(timer: &mut Timer, now: u64) {
        timer
            .apply(
                Action::Toggle {
                    revision: timer.revision,
                },
                now,
            )
            .unwrap();
    }

    #[test]
    fn unrecognized_timer_storage_stays_intact_until_explicitly_archived() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timer.json");
        for original in [
            br#"{"version":3,"presets":[65,400]}"#.as_slice(),
            b"{damaged",
        ] {
            fs::write(&path, original).unwrap();
            let mut timer = Timer::open(dir.path(), TimerDefaults::default(), 0);
            assert!(timer.view(0).storage_blocked);
            assert!(timer.apply(Action::SavePreset { seconds: 75 }, 0).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            timer.apply(Action::ResetStorage, 1).unwrap();
            assert!(!path.exists());
            assert!(!timer.view(1).storage_blocked);
            assert!(fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("timer.preserved-")
                        && fs::read(entry.path()).unwrap() == original
                }));
            timer.apply(Action::SavePreset { seconds: 75 }, 2).unwrap();
            let reopened = Timer::open(dir.path(), TimerDefaults::default(), 3);
            assert!(!reopened.view(3).storage_blocked);
            assert!(reopened.view(3).has_preset(75));
        }
    }

    #[test]
    fn confirmations_survive_natural_expiration_and_reject_newer_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut timer = Timer::open(dir.path(), TimerDefaults::default(), 0);
        let start = |seconds, replace_revision| Action::Start {
            mode: TimerMode::Countdown,
            seconds: Some(seconds),
            replace_revision,
        };
        timer.apply(start(1, None), 0).unwrap();
        let ending = timer.revision;
        timer.poll(999, true);
        timer.poll(1_000, true);
        timer
            .apply(Action::End { revision: ending }, 1_001)
            .unwrap();
        assert_eq!(timer.view(1_001).session.unwrap().status, Status::Done);
        assert!(matches!(
            timer.signal,
            Some(Signal::Complete(TimerCompletion::Countdown))
        ));

        timer.apply(start(5, Some(ending)), 1_001).unwrap();
        let current = timer.revision;
        assert!(timer.apply(start(2, Some(ending)), 1_002).is_err());
        assert!(timer
            .apply(Action::End { revision: ending }, 1_002)
            .is_err());
        assert_eq!(timer.view(1_002).session.unwrap().remaining_seconds, 5);
        timer
            .apply(Action::End { revision: current }, 6_000)
            .unwrap();
        timer.poll(6_001, true);
        assert!(timer.view(6_001).session.is_none());
        assert!(matches!(timer.signal, Some(Signal::Cancel)));
    }

    #[test]
    fn recovery_delivers_a_completion_once_and_a_replacement_discards_the_old_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut timer = Timer::open(dir.path(), TimerDefaults::default(), 0);
        let start = |seconds, replace_revision| Action::Start {
            mode: TimerMode::Countdown,
            seconds: Some(seconds),
            replace_revision,
        };
        timer.apply(start(1, None), 0).unwrap();
        timer.poll(1000, true);
        assert!(timer.take_signal(true).is_none());
        assert_eq!(timer.view(1000).session.unwrap().status, Status::Done);
        assert!(matches!(
            timer.take_signal(false),
            Some(Signal::Complete(TimerCompletion::Countdown))
        ));
        assert!(timer.take_signal(false).is_none());

        timer.apply(start(1, None), 1000).unwrap();
        timer.poll(2000, true);
        assert!(timer.take_signal(true).is_none());
        timer.apply(start(5, None), 2000).unwrap();
        assert!(matches!(
            timer.take_signal(false),
            Some(Signal::Start(TimerPhase::Countdown))
        ));
    }

    #[test]
    fn menu_actions_follow_the_active_timer_and_ending_it_cancels_completion() {
        let dir = tempfile::tempdir().unwrap();
        let defaults = TimerDefaults {
            countdown_presets: Some(vec![600, 300, 9]),
            ..TimerDefaults::default()
        };
        let mut timer = Timer::open(dir.path(), defaults, 0);
        let idle = timer.view(0).menu();
        assert_eq!(idle.pomodoro, "뽀모도로 · 집중 25분 시작");
        assert_eq!(idle.presets, [600, 300, 9]);
        assert!(!idle.can_end && idle.countdown_action.is_none());

        timer
            .apply(
                Action::Start {
                    mode: TimerMode::Countdown,
                    seconds: Some(9),
                    replace_revision: None,
                },
                0,
            )
            .unwrap();
        let running = timer.view(1).menu();
        assert!(running.can_end);
        assert_eq!(running.pomodoro, idle.pomodoro);
        assert_eq!(running.countdown_action, Some("일반 타이머 일시정지"));
        toggle(&mut timer, 500);
        let paused = timer.view(500).menu();
        assert_eq!(paused.countdown_action, Some("일반 타이머 재개"));
        assert!(timer
            .apply(
                Action::End {
                    revision: running.revision
                },
                501
            )
            .is_err());
        assert!(timer.view(501).menu().can_end);
        timer
            .apply(
                Action::End {
                    revision: paused.revision,
                },
                502,
            )
            .unwrap();
        timer.poll(10_000, true);
        let ended = timer.view(10_000);
        assert!(!ended.menu().can_end);
        assert!(ended.outcome.is_none() && ended.badge.is_none());
        assert!(matches!(timer.signal, Some(Signal::Cancel)));

        timer
            .apply(
                Action::Start {
                    mode: TimerMode::Pomodoro,
                    seconds: None,
                    replace_revision: None,
                },
                10_000,
            )
            .unwrap();
        assert_eq!(
            timer.view(10_001).menu().pomodoro,
            "뽀모도로 · 집중 일시정지"
        );
        timer.poll(1_510_000, false);
        let ready = timer.view(1_510_000).menu();
        assert_eq!(ready.pomodoro, "뽀모도로 · 짧은 휴식 5분 시작");
        assert!(ready.can_end && ready.countdown_action.is_none());
    }

    #[test]
    fn countdown_presets_pause_and_expiration_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let defaults = TimerDefaults::default();
        let mut timer = Timer::open(dir.path(), defaults.clone(), 100);
        timer
            .apply(Action::SavePreset { seconds: 30 }, 100)
            .unwrap();
        assert!(timer
            .apply(Action::SavePreset { seconds: 30 }, 100)
            .is_err());
        timer
            .apply(
                Action::Start {
                    mode: TimerMode::Countdown,
                    seconds: Some(30),
                    replace_revision: None,
                },
                100,
            )
            .unwrap();
        toggle(&mut timer, 1100);
        let mut timer = Timer::open(dir.path(), defaults.clone(), 900_000);
        let view = timer.view(900_000);
        assert_eq!(view.presets, [60, 300, 600, 1500, 30]);
        assert_eq!(view.session.unwrap().remaining_seconds, 29);
        toggle(&mut timer, 900_000);
        let mut timer = Timer::open(dir.path(), defaults, 940_000);
        assert_eq!(timer.view(940_000).session.unwrap().status, Status::Done);
        assert!(timer.signal.is_none());
        assert!(timer.view(940_000).badge.is_none());
        toggle(&mut timer, 940_000);
        assert_eq!(timer.view(940_000).session.unwrap().remaining_seconds, 30);
    }

    #[test]
    fn shortcuts_inherit_studio_until_customized_and_restore_the_current_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let mut defaults = TimerDefaults {
            countdown_presets: Some(vec![30, 90, 240]),
            ..Default::default()
        };
        let mut timer = Timer::open(dir.path(), defaults.clone(), 0);
        assert_eq!(timer.view(0).presets, [30, 90, 240]);
        assert!(!timer.view(0).custom_presets);
        timer
            .apply(
                Action::SetPresets {
                    presets: Some(vec![240, 45]),
                },
                1,
            )
            .unwrap();
        assert!(timer
            .apply(
                Action::SetPresets {
                    presets: Some(vec![45, 45])
                },
                2
            )
            .is_err());
        defaults.countdown_presets = Some(vec![7, 700]);
        let mut timer = Timer::open(dir.path(), defaults.clone(), 3);
        assert_eq!(timer.view(3).presets, [240, 45]);
        assert!(timer.view(3).custom_presets);
        timer
            .apply(
                Action::SetPresets {
                    presets: Some(vec![]),
                },
                4,
            )
            .unwrap();
        let mut timer = Timer::open(dir.path(), defaults, 5);
        assert!(timer.view(5).presets.is_empty());
        timer
            .apply(Action::SetPresets { presets: None }, 6)
            .unwrap();
        assert_eq!(timer.view(6).presets, [7, 700]);
        assert!(!timer.view(6).custom_presets);
    }

    #[test]
    fn legacy_presets_keep_saved_times_and_the_previous_quick_choices() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("timer.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "lastCountdown": 600, "presets": [45, 300],
                "personal": null, "showBadge": true, "mode": "countdown",
                "session": null, "outcome": null
            }))
            .unwrap(),
        )
        .unwrap();
        let defaults = TimerDefaults {
            countdown_presets: Some(vec![100]),
            ..Default::default()
        };
        let timer = Timer::open(dir.path(), defaults, 0);
        assert_eq!(timer.view(0).presets, [45, 300, 60, 600, 1500]);
        assert!(timer.view(0).custom_presets);
    }

    #[test]
    fn pomodoro_snapshots_settings_counts_only_expired_focus_and_waits_for_start() {
        let dir = tempfile::tempdir().unwrap();
        let profile = PomodoroSettings {
            focus_minutes: 1,
            short_break_minutes: 1,
            long_break_minutes: 2,
            rounds: 2,
        };
        let mut timer = Timer::open(
            dir.path(),
            TimerDefaults {
                mode: TimerMode::Pomodoro,
                pomodoro: profile,
                ..Default::default()
            },
            0,
        );
        toggle(&mut timer, 0);
        assert!(matches!(
            timer.signal.take(),
            Some(Signal::Start(TimerPhase::Focus))
        ));
        assert_eq!(timer.running_phase(), Some(TimerPhase::Focus));
        assert!(timer
            .apply(
                Action::Start {
                    mode: TimerMode::Countdown,
                    seconds: Some(10),
                    replace_revision: None
                },
                1
            )
            .is_err());
        timer
            .apply(
                Action::SetProfile {
                    profile: Some(PomodoroSettings::default()),
                },
                1,
            )
            .unwrap();
        timer.poll(59_800, false);
        timer.poll(60_000, true);
        assert!(matches!(
            timer.signal.take(),
            Some(Signal::Complete(TimerCompletion::Focus))
        ));
        let rest = timer.view(60_000).session.unwrap();
        assert_eq!(
            (
                rest.phase,
                rest.status,
                rest.completed,
                rest.duration_seconds
            ),
            (Phase::ShortBreak, Status::Ready, 1, 60)
        );
        timer.poll(70_000, true);
        assert!(timer.signal.is_none());
        timer
            .apply(
                Action::Skip {
                    revision: timer.revision,
                },
                70_000,
            )
            .unwrap();
        assert_eq!(timer.view(70_000).session.unwrap().status, Status::Running);
        assert!(matches!(
            timer.signal.take(),
            Some(Signal::Start(TimerPhase::Focus))
        ));
        timer.poll(129_800, false);
        timer.poll(130_000, false);
        let rest = timer.view(130_000).session.unwrap();
        assert_eq!(
            (rest.phase, rest.completed, rest.duration_seconds),
            (Phase::LongBreak, 2, 120)
        );
        let revision = timer.revision;
        toggle(&mut timer, 130_000);
        assert_eq!(timer.running_phase(), Some(TimerPhase::LongBreak));
        assert!(matches!(
            timer.signal.take(),
            Some(Signal::Start(TimerPhase::LongBreak))
        ));
        assert!(timer.apply(Action::Toggle { revision }, 130_001).is_err());
        timer.poll(250_000, false);
        assert_eq!(timer.view(250_000).session.unwrap().completed, 0);
        timer
            .apply(
                Action::End {
                    revision: timer.revision,
                },
                250_001,
            )
            .unwrap();
        toggle(&mut timer, 250_002);
        assert_eq!(timer.view(250_002).session.unwrap().duration_seconds, 1500);
    }

    #[test]
    fn badge_stays_in_the_work_area_and_uses_the_opposite_edge_when_needed() {
        let work = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let size = Point { x: 120.0, y: 100.0 };
        let below = badge_position(work, Point { x: 0.0, y: 40.0 }, size, None).unwrap();
        let above = badge_position(work, Point { x: 520.0, y: 370.0 }, size, None).unwrap();
        assert_eq!(below.origin, Point { x: 4.0, y: 146.0 });
        assert_eq!(above.origin, Point { x: 520.0, y: 318.0 });
        assert!(!below.overlaps_buddy && !above.overlaps_buddy);

        let side =
            badge_position(work, Point::default(), Point { x: 50.0, y: 480.0 }, None).unwrap();
        assert_eq!(side.origin, Point { x: 56.0, y: 217.0 });
        assert!(!side.overlaps_buddy);

        let corner =
            badge_position(work, Point::default(), Point { x: 620.0, y: 470.0 }, None).unwrap();
        assert_eq!(corner.origin, Point { x: 520.0, y: 430.0 });
        assert!(corner.overlaps_buddy);
    }

    #[test]
    fn badge_follows_painted_pixels_on_a_narrow_work_area() {
        let work = Rect {
            x: 0.0,
            y: 0.0,
            width: 150.0,
            height: 100.0,
        };
        let padding = Some((Point { x: 0.35, y: 0.1 }, Point { x: 0.65, y: 0.3 }));
        let badge = badge_position(
            work,
            Point::default(),
            Point { x: 150.0, y: 100.0 },
            padding,
        )
        .unwrap();
        assert_eq!(badge.origin, Point { x: 17.0, y: 36.0 });
        assert!(!badge.overlaps_buddy);

        let offset = Point { x: -46.0, y: -4.0 };
        let badge = badge_position(work, offset, Point { x: 150.0, y: 100.0 }, padding).unwrap();
        assert_eq!(badge.origin, Point { x: 4.0, y: 32.0 });
        assert!(!badge.overlaps_buddy);
    }

    #[test]
    fn badge_shows_long_and_paused_times_and_clears_after_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut timer = Timer::open(dir.path(), TimerDefaults::default(), 0);
        timer
            .apply(
                Action::Start {
                    mode: TimerMode::Countdown,
                    seconds: Some(10800),
                    replace_revision: None,
                },
                0,
            )
            .unwrap();
        assert_eq!(timer.view(0).badge.unwrap().time, "03:00:00");
        toggle(&mut timer, 1_000);
        let paused = timer.view(10_000).badge.unwrap();
        assert_eq!(paused.time, "02:59:59");
        assert!(paused.label.ends_with("일시정지"));
        toggle(&mut timer, 10_000);
        let ends_at = 10_809_000;
        timer.poll(ends_at - 50, true);
        timer.poll(ends_at, true);
        let completed = timer.view(ends_at).badge.unwrap();
        assert_eq!(completed.time, "00:00");
        assert!(completed.label.ends_with("완료"));
        assert!(timer.view(ends_at + 4000).badge.is_none());
    }
}
