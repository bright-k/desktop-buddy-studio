use buddy_project::{MouseButton, TimerCompletion, TimerPhase, WheelDirection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum InputEvent {
    KeyDown {
        code: String,
        #[serde(default)]
        repeat: bool,
    },
    KeyUp {
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
    /// Both axes were reported by the same native scroll sample.
    DiagonalWheel {
        left: bool,
        up: bool,
    },
    CursorMove {
        x: f64,
        y: f64,
    },
    BuddyClick,
    BuddyLongPress,
    DragStart,
    DragEnd,
    DragDropped,
    TimerPhaseChanged {
        phase: Option<TimerPhase>,
    },
    TimerStarted {
        phase: TimerPhase,
    },
    TimerCompleted {
        phase: TimerCompletion,
    },
    OverlayVisibility {
        visible: bool,
    },
    Reset,
    /// Native capture epoch boundary. These keys/buttons are suppressed until
    /// release; this snapshot never restores held actions or matcher progress.
    Rearm {
        keys: Vec<String>,
        buttons: Vec<MouseButton>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Controls {
    pub active: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub movement_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn clamp(self, x: f64, y: f64, size: buddy_project::Point) -> buddy_project::Point {
        buddy_project::Point {
            x: x.clamp(self.x, self.x + (self.width - size.x).max(0.0)),
            y: y.clamp(self.y, self.y + (self.height - size.y).max(0.0)),
        }
    }
}
