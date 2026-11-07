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
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub build: Build,
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
            margin_dip: 24.0,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Trigger {
    KeyDown {
        code: String,
        #[serde(default)]
        repeat: bool,
    },
    KeyUp {
        code: String,
    },
    WhileKeyDown {
        code: String,
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
    CursorMove,
    Drag,
    TypingRate {
        #[serde(rename = "minKps")]
        min_kps: f64,
        #[serde(rename = "maxKps", default, skip_serializing_if = "Option::is_none")]
        max_kps: Option<f64>,
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
    },
}
fn default_timeout() -> u32 {
    5000
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Effect {
    PlayAnimation {
        animation: String,
        #[serde(default = "default_priority")]
        priority: u8,
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
    },
    Move {
        x: f64,
        y: f64,
        #[serde(rename = "speedDipPerSecond")]
        speed_dip_per_second: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        animation: Option<String>,
    },
    FaceCursor,
}
fn default_priority() -> u8 {
    50
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum Retrigger {
    #[default]
    Restart,
    Ignore,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Primary,
    Secondary,
    Middle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
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
    pub image: String,
    #[serde(default)]
    pub resources: BTreeMap<String, Resource>,
    pub sheet: Sheet,
    pub anchor: Point,
    pub default_animation: String,
    pub animations: BTreeMap<String, Animation>,
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

pub fn typescript_declarations() -> String {
    let declarations = [
        Project::decl(),
        Product::decl(),
        Defaults::decl(),
        Timing::decl(),
        InitialPosition::decl(),
        Anchor::decl(),
        InputPreferences::decl(),
        Build::decl(),
        Signing::decl(),
        WindowsSigning::decl(),
        MacSigning::decl(),
        Target::decl(),
        Binding::decl(),
        Trigger::decl(),
        Effect::decl(),
        Retrigger::decl(),
        MouseButton::decl(),
        WheelDirection::decl(),
        KeyboardLayout::decl(),
        KeyStep::decl(),
        Character::decl(),
        Sheet::decl(),
        Point::decl(),
        Animation::decl(),
        Resource::decl(),
        FrameRegion::decl(),
        RenderFrame::decl(),
        RenderAnimation::decl(),
        RenderPlan::decl(),
    ];
    declarations
        .into_iter()
        .map(|line| format!("export {line}\n"))
        .collect()
}
