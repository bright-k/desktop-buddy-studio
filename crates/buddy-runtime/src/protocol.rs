use crate::{CharacterState, PerformanceSnapshot, Rect, RuntimeDiagnostics};
use buddy_project::{Character, Point, Product};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub fn preference_identity(identifier: &str, project_id: &str, preview: bool) -> String {
    if preview {
        format!("io.desktopbuddy.preview.{project_id}")
    } else {
        identifier.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    NotRequired,
    NeedsRequest,
    Granted,
    Denied,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    Stopped,
    Running,
    Suspended,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct InputSourceStatus {
    pub permission: PermissionState,
    pub capture: CaptureState,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct InputStatus {
    pub keyboard: InputSourceStatus,
    pub mouse: InputSourceStatus,
}
impl InputStatus {
    pub fn stop(&mut self, reason: &str) {
        for source in [&mut self.keyboard, &mut self.mouse] {
            source.capture = CaptureState::Stopped;
            source.reason = Some(reason.into());
        }
    }
    pub fn any_running(&self) -> bool {
        self.keyboard.capture == CaptureState::Running
            || self.mouse.capture == CaptureState::Running
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBootstrap {
    pub character: Character,
    pub product: Product,
    pub image_paths: std::collections::BTreeMap<String, String>,
    pub render_plan: buddy_project::RenderPlan,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WireState {
    pub seq: u32,
    pub state: CharacterState,
    pub editing: bool,
    pub visible: bool,
    pub edit_session_id: Option<String>,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferenceUpdate {
    #[serde(default)]
    pub timing: Option<buddy_project::Timing>,
    pub keyboard: Option<bool>,
    pub mouse: Option<bool>,
    pub movement_enabled: Option<bool>,
    pub always_on_top: Option<bool>,
    pub scale: Option<f64>,
    #[serde(default)]
    pub base_placement: Option<BasePlacement>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub product_name: String,
    pub visible: bool,
    pub paused: bool,
    pub dragging: bool,
    pub timing: buddy_project::Timing,
    pub keyboard: bool,
    pub mouse: bool,
    pub movement_enabled: bool,
    pub always_on_top: bool,
    pub scale: f64,
    pub input: InputStatus,
    pub notice: Option<String>,
    pub monitors: Vec<MonitorChoice>,
    pub base_placement: BasePlacement,
    pub diagnostics: RuntimeDiagnostics,
    pub performance: PerformanceSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct MonitorChoice {
    pub id: String,
    pub name: String,
    pub primary: bool,
    pub work_area: Rect,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasePlacement {
    pub monitor_hint: String,
    pub position: Point,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PreferenceOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<buddy_project::Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_on_top: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movement_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_placement: Option<BasePlacement>,
}

pub(crate) fn declarations() -> String {
    [
        PermissionState::decl(),
        CaptureState::decl(),
        InputSourceStatus::decl(),
        InputStatus::decl(),
        PlayerBootstrap::decl(),
        WireState::decl(),
        PreferenceUpdate::decl(),
        RuntimeStatus::decl(),
        MonitorChoice::decl(),
        BasePlacement::decl(),
        PreferenceOverrides::decl(),
    ]
    .into_iter()
    .map(|line| format!("export {line}\n"))
    .collect()
}
