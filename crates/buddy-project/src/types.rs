use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub schema_version: u32,
    pub project_id: String,
    pub product: Product,
    pub character: String,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<IdleSettings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 4))]
    pub idle_groups: Vec<IdleGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_follow: Option<CursorFollow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<TimerDefaults>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub build: Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum TimerMode {
    #[default]
    Countdown,
    Pomodoro,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "camelCase")]
pub enum TimerPhase {
    Countdown,
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum TimerCompletion {
    Countdown,
    Focus,
    Break,
    ShortBreak,
    LongBreak,
}

impl From<TimerPhase> for TimerCompletion {
    fn from(phase: TimerPhase) -> Self {
        match phase {
            TimerPhase::Countdown => Self::Countdown,
            TimerPhase::Focus => Self::Focus,
            TimerPhase::ShortBreak => Self::ShortBreak,
            TimerPhase::LongBreak => Self::LongBreak,
        }
    }
}

impl TimerCompletion {
    pub(crate) fn matches(self, completed: Self) -> bool {
        self == completed
            || (self == Self::Break && matches!(completed, Self::ShortBreak | Self::LongBreak))
    }
}

pub const BUDDY_HOLD_MIN_MS: u32 = 300;
pub const BUDDY_HOLD_MAX_MS: u32 = 2000;
pub const BUDDY_HOLD_DEFAULT_MS: u32 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PomodoroSettings {
    #[schemars(range(min = 1, max = 120))]
    pub focus_minutes: u32,
    #[schemars(range(min = 1, max = 60))]
    pub short_break_minutes: u32,
    #[schemars(range(min = 1, max = 60))]
    pub long_break_minutes: u32,
    #[schemars(range(min = 2, max = 8))]
    pub rounds: u32,
}
impl Default for PomodoroSettings {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            rounds: 4,
        }
    }
}
impl PomodoroSettings {
    pub fn is_valid(self) -> bool {
        (1..=120).contains(&self.focus_minutes)
            && (1..=60).contains(&self.short_break_minutes)
            && (1..=60).contains(&self.long_break_minutes)
            && (2..=8).contains(&self.rounds)
    }
}

pub const DEFAULT_TIMER_PRESETS: [u32; 4] = [60, 300, 600, 1500];
pub const MAX_TIMER_PRESETS: usize = 10;

pub fn valid_timer_presets(presets: &[u32]) -> bool {
    presets.len() <= MAX_TIMER_PRESETS
        && presets.iter().enumerate().all(|(index, seconds)| {
            (1..=10_800).contains(seconds) && !presets[..index].contains(seconds)
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TimerDefaults {
    pub enabled: bool,
    pub mode: TimerMode,
    #[schemars(range(min = 1, max = 10800))]
    pub countdown_seconds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 10))]
    pub countdown_presets: Option<Vec<u32>>,
    pub pomodoro: PomodoroSettings,
}
impl Default for TimerDefaults {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: TimerMode::Countdown,
            countdown_seconds: 600,
            countdown_presets: None,
            pomodoro: PomodoroSettings::default(),
        }
    }
}
impl TimerDefaults {
    pub fn countdown_presets(&self) -> &[u32] {
        self.countdown_presets
            .as_deref()
            .unwrap_or(&DEFAULT_TIMER_PRESETS)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CursorFollow {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 100))]
    pub priority: Option<u8>,
    pub stop_distance_dip: f64,
    pub bands: Vec<CursorFollowBand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CursorFollowBand {
    pub min_distance_dip: f64,
    pub speed_dip_per_second: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
}
impl Default for CursorFollow {
    fn default() -> Self {
        Self {
            enabled: false,
            priority: None,
            stop_distance_dip: 48.0,
            bands: vec![CursorFollowBand {
                min_distance_dip: 0.0,
                speed_dip_per_second: 120.0,
                animation: None,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Product {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub identifier: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Defaults {
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 16, max = 1024))]
    pub display_height_dip: Option<f64>,
    pub always_on_top: bool,
    pub position: InitialPosition,
    pub input: InputPreferences,
    pub movement_enabled: bool,
    pub timing: Timing,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            scale: 1.0,
            display_height_dip: None,
            always_on_top: true,
            position: InitialPosition::default(),
            input: InputPreferences::default(),
            movement_enabled: false,
            timing: Timing::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Timing {
    pub rate_window_ms: u32,
    pub rate_evaluation_ms: u32,
    pub rate_stability_ms: u32,
    pub idle_timeout_ms: u32,
}
impl Timing {
    pub fn is_valid(&self) -> bool {
        (100..=10000).contains(&self.rate_window_ms)
            && (10..=250).contains(&self.rate_evaluation_ms)
            && self.rate_stability_ms <= 2000
            && (50..=5000).contains(&self.idle_timeout_ms)
            && self.rate_evaluation_ms <= self.idle_timeout_ms
    }
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            rate_window_ms: 2000,
            rate_evaluation_ms: 50,
            rate_stability_ms: 100,
            idle_timeout_ms: 250,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct InitialPosition {
    pub monitor: String,
    pub anchor: Anchor,
    pub margin_dip: f64,
}
impl Default for InitialPosition {
    fn default() -> Self {
        Self {
            monitor: "primary".into(),
            anchor: Anchor::BottomRight,
            margin_dip: 10.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, deny_unknown_fields)]
pub struct InputPreferences {
    pub keyboard: bool,
    pub mouse: bool,
}
impl Default for InputPreferences {
    fn default() -> Self {
        Self {
            keyboard: true,
            mouse: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Build {
    pub targets: Vec<Target>,
    pub signing: Signing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_dmg: Option<MacosDmg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct MacosDmg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<String>,
    pub use_default_background: bool,
    pub use_default_guide: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide_position: Option<DmgIconPosition>,
    pub width: u32,
    pub height: u32,
    pub app_position: DmgIconPosition,
    pub applications_position: DmgIconPosition,
}
impl Default for MacosDmg {
    fn default() -> Self {
        Self {
            background: None,
            guide: None,
            use_default_background: false,
            use_default_guide: false,
            guide_position: None,
            width: 660,
            height: 420,
            app_position: DmgIconPosition { x: 180, y: 180 },
            applications_position: DmgIconPosition { x: 480, y: 180 },
        }
    }
}
impl MacosDmg {
    pub const ICON_BOTTOM_MARGIN: u32 = 128;

    pub fn installer_default() -> Self {
        serde_json::from_str(include_str!(
            "../../../packaging/macos/defaults/layout.json"
        ))
        .expect("built-in macOS installer layout")
    }

    pub fn includes_guide(&self) -> bool {
        self.guide.is_some() || self.use_default_guide
    }

    fn guide_position_fits(&self, position: DmgIconPosition) -> bool {
        position.x >= 72
            && position.x <= self.width.saturating_sub(72)
            && position.y >= 72
            && position.y <= self.height.saturating_sub(Self::ICON_BOTTOM_MARGIN)
            && [self.app_position, self.applications_position]
                .into_iter()
                .all(|icon| {
                    position.x.abs_diff(icon.x) >= 132 || position.y.abs_diff(icon.y) >= 132
                })
    }

    pub fn guide_position(&self) -> Option<DmgIconPosition> {
        if let Some(position) = self.guide_position {
            return self.guide_position_fits(position).then_some(position);
        }
        if self.use_default_guide
            && self.app_position.y == self.applications_position.y
            && self.app_position.x < self.applications_position.x
        {
            let gap = self.applications_position.x - self.app_position.x;
            if let Some(x) = self.app_position.x.checked_sub(gap) {
                let position = DmgIconPosition {
                    x,
                    y: self.app_position.y,
                };
                if self.guide_position_fits(position) {
                    return Some(position);
                }
            }
        }
        let right = self.width.checked_sub(72)?;
        // Leave room for Finder's title bar and the icon label.
        let bottom = self.height.checked_sub(Self::ICON_BOTTOM_MARGIN)?;
        [bottom, 72, self.height / 2]
            .into_iter()
            .flat_map(|y| {
                [self.width / 2, 72, right]
                    .into_iter()
                    .map(move |x| DmgIconPosition { x, y })
            })
            .find(|position| self.guide_position_fits(*position))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DmgIconPosition {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, deny_unknown_fields)]
pub struct Signing {
    pub windows: WindowsSigning,
    pub macos: MacSigning,
}
impl Default for Signing {
    fn default() -> Self {
        Self {
            windows: WindowsSigning::None,
            macos: MacSigning::AdHoc,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsSigning {
    None,
    SelfSigned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "kebab-case")]
pub enum MacSigning {
    AdHoc,
    SelfSigned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Target {
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "macos-arm64")]
    MacosArm64,
}
impl Target {
    pub fn current() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Some(Self::WindowsX64),
            ("macos", "aarch64") => Some(Self::MacosArm64),
            _ => None,
        }
    }
    pub fn rust_target(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::MacosArm64 => "aarch64-apple-darwin",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: String,
    pub trigger: Trigger,
    pub effect: Effect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_cursor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundCue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SoundCue {
    pub resource: String,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Trigger {
    KeyDown {
        code: String,
        #[serde(default)]
        repeat: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modifiers: Option<Vec<KeyModifier>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alternatives: Vec<KeyChord>,
    },
    KeyUp {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modifiers: Option<Vec<KeyModifier>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alternatives: Vec<KeyChord>,
    },
    WhileKeyDown {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modifiers: Option<Vec<KeyModifier>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alternatives: Vec<KeyChord>,
    },
    MouseDown {
        button: MouseButton,
    },
    MouseUp {
        button: MouseButton,
    },
    Wheel {
        direction: WheelDirection,
    },
    MouseInput {
        #[schemars(length(min = 1, max = 11))]
        inputs: Vec<PointerInput>,
    },
    CursorMove,
    BuddyClick,
    BuddyLongPress {
        #[serde(rename = "holdMs")]
        #[schemars(range(min = 300, max = 2000))]
        hold_ms: u32,
    },
    Drag,
    DragDropped,
    FirstShown,
    ShownAgain,
    TimerStarted {
        phase: TimerPhase,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(length(max = 3))]
        alternatives: Vec<TimerPhase>,
    },
    TimerCompleted {
        phase: TimerCompletion,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(length(max = 3))]
        alternatives: Vec<TimerCompletion>,
    },
    IdleFor {
        #[schemars(range(min = 1, max = 3600))]
        seconds: u32,
    },
    TypingRate {
        #[serde(rename = "minKps")]
        min_kps: f64,
        #[serde(rename = "maxKps", default, skip_serializing_if = "Option::is_none")]
        max_kps: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    TypingStopped {
        #[schemars(range(min = 2, max = 60))]
        min_typing_seconds: u32,
        #[schemars(range(min = 2, max = 100))]
        min_keys: u32,
        #[schemars(range(min = 500, max = 5000))]
        pause_ms: u32,
        #[schemars(range(min = 10, max = 600))]
        min_interval_seconds: u32,
    },
    KeySequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keyword: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<KeyboardLayout>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<Vec<KeyStep>>,
        #[serde(rename = "timeoutMs", default = "default_timeout")]
        timeout_ms: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alternatives: Vec<KeywordAlternative>,
    },
}

impl Trigger {
    pub fn matches_timer_start(&self, started: TimerPhase) -> bool {
        matches!(self, Self::TimerStarted { phase, alternatives }
            if *phase == started || alternatives.contains(&started))
    }

    pub fn matches_timer_completion(&self, completed: TimerCompletion) -> bool {
        matches!(self, Self::TimerCompleted { phase, alternatives }
            if phase.matches(completed) || alternatives.iter().any(|phase| phase.matches(completed)))
    }

    pub fn is_automatic(&self) -> bool {
        matches!(
            self,
            Self::FirstShown
                | Self::ShownAgain
                | Self::IdleFor { .. }
                | Self::TypingStopped { .. }
                | Self::TimerStarted { .. }
                | Self::TimerCompleted { .. }
        )
    }

    pub fn supports_stop_on_release(&self) -> bool {
        match self {
            Self::KeyDown { .. } | Self::MouseDown { .. } => true,
            Self::MouseInput { inputs } => {
                !inputs.is_empty()
                    && inputs
                        .iter()
                        .all(|input| matches!(input, PointerInput::MouseDown { .. }))
            }
            _ => false,
        }
    }

    pub fn key_conditions(&self) -> impl Iterator<Item = (&str, Option<&[KeyModifier]>)> {
        let (primary, alternatives) = match self {
            Self::KeyDown {
                code,
                modifiers,
                alternatives,
                ..
            }
            | Self::KeyUp {
                code,
                modifiers,
                alternatives,
            }
            | Self::WhileKeyDown {
                code,
                modifiers,
                alternatives,
            } => (
                Some((code.as_str(), modifiers.as_deref())),
                alternatives.as_slice(),
            ),
            _ => (None, &[][..]),
        };
        primary.into_iter().chain(
            alternatives
                .iter()
                .map(|key| (key.code.as_str(), key.modifiers.as_deref())),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct KeyChord {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<KeyModifier>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct KeywordAlternative {
    pub keyword: String,
    pub layout: KeyboardLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum KeyModifier {
    Control,
    Alt,
    Shift,
    Meta,
}

impl KeyModifier {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "ControlLeft" | "ControlRight" => Some(Self::Control),
            "AltLeft" | "AltRight" => Some(Self::Alt),
            "ShiftLeft" | "ShiftRight" => Some(Self::Shift),
            "MetaLeft" | "MetaRight" => Some(Self::Meta),
            _ => None,
        }
    }

    pub fn bit(self) -> u8 {
        1 << self as u8
    }
}

fn default_timeout() -> u32 {
    5000
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum MotionVariationMode {
    Cycle,
    Shuffle,
    Weighted,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionVariation {
    pub mode: MotionVariationMode,
    #[schemars(length(min = 1, max = 7))]
    pub alternatives: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 2, max = 8))]
    pub weights: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Effect {
    PlayAnimation {
        animation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variation: Option<MotionVariation>,
        #[serde(default = "default_priority")]
        priority: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interruption: Option<Interruption>,
        #[serde(default)]
        retrigger: Retrigger,
        #[serde(rename = "cooldownMs", default)]
        cooldown_ms: u32,
        #[serde(
            rename = "durationMs",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        duration_ms: Option<u32>,
        #[serde(rename = "stopOnRelease", default)]
        stop_on_release: bool,
    },
    SustainAnimation {
        animation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<u8>,
    },
    Move {
        x: f64,
        y: f64,
        #[serde(rename = "speedDipPerSecond")]
        speed_dip_per_second: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        animation: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<u8>,
    },
    FaceCursor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directions: Option<BTreeMap<CursorDirection, CursorPose>>,
    },
}
fn default_priority() -> u8 {
    50
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum Interruption {
    #[default]
    ByPriority,
    Replace,
    Finish,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "camelCase")]
pub enum CursorDirection {
    UpLeft,
    Up,
    UpRight,
    Left,
    Center,
    Right,
    DownLeft,
    Down,
    DownRight,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CursorPose {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct IdleSettings {
    pub cursor_enabled: bool,
    pub directions: BTreeMap<CursorDirection, CursorPose>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum IdleCursorMode {
    #[default]
    Inherit,
    Custom,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdleGroup {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[schemars(length(min = 1, max = 4))]
    pub states: Vec<TimerPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    #[serde(default)]
    pub cursor_mode: IdleCursorMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub directions: BTreeMap<CursorDirection, CursorPose>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum Retrigger {
    #[default]
    Restart,
    Ignore,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Primary,
    Secondary,
    Middle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PointerInput {
    MouseDown { button: MouseButton },
    MouseUp { button: MouseButton },
    Wheel { direction: WheelDirection },
    BuddyClick,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum KeyboardLayout {
    #[serde(rename = "ko-KR-2set")]
    Korean2Set,
    #[serde(rename = "en-US-qwerty")]
    EnglishQwerty,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct KeyStep {
    pub code: String,
    pub shift: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Character {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub resources: BTreeMap<String, Resource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sounds: BTreeMap<String, SoundResource>,
    pub sheet: Sheet,
    pub anchor: Point,
    pub default_animation: String,
    pub animations: BTreeMap<String, Animation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_layout: Option<CanvasLayout>,
}

/// The viewport of the source project before unused clips were removed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasLayout {
    #[schemars(range(min = 0.5, max = 1.0))]
    pub horizontal_anchor_extent: f64,
    #[schemars(range(min = 1.0, max = 2.0))]
    pub scale_envelope: f64,
}

pub const MIN_ANIMATION_SCALE_PERCENT: u16 = 50;
pub const MAX_ANIMATION_SCALE_PERCENT: u16 = 200;

impl Character {
    pub fn awaiting_sprite(&self) -> bool {
        self.image.is_none() && self.default_animation.is_empty() && self.animations.is_empty()
    }

    pub fn canvas_size(&self, scale: f64) -> Point {
        let scale = scale * self.animation_scale_envelope();
        Point {
            x: f64::from(self.sheet.frame_width) * 2.0 * self.horizontal_anchor_extent() * scale,
            y: f64::from(self.sheet.frame_height) * scale,
        }
    }

    pub fn horizontal_anchor_extent(&self) -> f64 {
        self.animations
            .values()
            .filter_map(|animation| animation.anchor_x)
            .chain(std::iter::once(self.anchor.x))
            .map(|x| x.max(1.0 - x))
            .fold(
                self.canvas_layout
                    .map_or(0.5, |layout| layout.horizontal_anchor_extent),
                f64::max,
            )
    }

    /// A fixed viewport for the whole project keeps large clips in bounds.
    pub fn animation_scale_envelope(&self) -> f64 {
        let largest = self
            .animations
            .values()
            .filter_map(|animation| animation.scale_percent)
            .fold(100, u16::max);
        (f64::from(largest) / 100.0).max(
            self.canvas_layout
                .map_or(1.0, |layout| layout.scale_envelope),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sheet {
    pub width: u32,
    pub height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_count: u32,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum SourceFacing {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Animation {
    #[serde(default)]
    pub frames: Vec<u32>,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<FrameRegion>,
    #[serde(default = "default_source_timing")]
    pub use_source_timing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 50, max = 200))]
    pub scale_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_facing: Option<SourceFacing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub anchor_x: Option<f64>,
    #[serde(rename = "loop")]
    pub looping: bool,
}

fn default_fps() -> u32 {
    12
}
fn default_source_timing() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 80))]
    pub name: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct FrameRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RenderFrame {
    pub image: String,
    pub region: FrameRegion,
    pub duration_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RenderAnimation {
    pub frames: Vec<RenderFrame>,
    pub looping: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RenderPlan {
    pub animations: BTreeMap<String, RenderAnimation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SoundResource {
    pub audio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 80))]
    pub name: Option<String>,
}

pub fn typescript_declarations() -> String {
    let declarations = [
        Project::decl(),
        Product::decl(),
        Defaults::decl(),
        CursorFollow::decl(),
        CursorFollowBand::decl(),
        Timing::decl(),
        TimerMode::decl(),
        TimerPhase::decl(),
        TimerCompletion::decl(),
        TimerDefaults::decl(),
        PomodoroSettings::decl(),
        InitialPosition::decl(),
        Anchor::decl(),
        InputPreferences::decl(),
        Build::decl(),
        MacosDmg::decl(),
        DmgIconPosition::decl(),
        Signing::decl(),
        WindowsSigning::decl(),
        MacSigning::decl(),
        Target::decl(),
        Binding::decl(),
        SoundCue::decl(),
        Trigger::decl(),
        PointerInput::decl(),
        KeyModifier::decl(),
        KeyChord::decl(),
        KeywordAlternative::decl(),
        MotionVariationMode::decl(),
        MotionVariation::decl(),
        Effect::decl(),
        Interruption::decl(),
        CursorDirection::decl(),
        CursorPose::decl(),
        IdleSettings::decl(),
        IdleCursorMode::decl(),
        IdleGroup::decl(),
        Retrigger::decl(),
        MouseButton::decl(),
        WheelDirection::decl(),
        KeyboardLayout::decl(),
        KeyStep::decl(),
        Character::decl(),
        CanvasLayout::decl(),
        Sheet::decl(),
        Point::decl(),
        SourceFacing::decl(),
        Animation::decl(),
        Resource::decl(),
        SoundResource::decl(),
        FrameRegion::decl(),
        RenderFrame::decl(),
        RenderAnimation::decl(),
        RenderPlan::decl(),
    ];
    let mut output: String = declarations
        .into_iter()
        .map(|line| format!("export {line}\n"))
        .collect();
    let keys: Vec<_> = crate::supported_keys().collect();
    output.push_str(&format!(
        "export const supportedPhysicalKeys: readonly string[] = {};\n",
        serde_json::to_string(&keys).expect("key codes are serializable")
    ));
    output.push_str(&format!(
        "export const animationScaleLimits = {{ min: {MIN_ANIMATION_SCALE_PERCENT}, max: {MAX_ANIMATION_SCALE_PERCENT} }} as const;\n"
    ));
    output.push_str(&format!(
        "export const timerPresetLimit = {MAX_TIMER_PRESETS};\nexport const defaultTimerPresets = {DEFAULT_TIMER_PRESETS:?} as const;\n"
    ));
    output.push_str(&format!(
        "export const buddyHoldMs = {{ min: {BUDDY_HOLD_MIN_MS}, max: {BUDDY_HOLD_MAX_MS}, default: {BUDDY_HOLD_DEFAULT_MS} }} as const;\n"
    ));
    output
}
