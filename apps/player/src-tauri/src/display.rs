use buddy_project::Point;
use buddy_runtime::{MonitorChoice, Rect};

#[derive(Clone, PartialEq)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub primary: bool,
    /// Tauri physical work-area origin. On macOS each screen uses its own scale.
    pub origin: Point,
    pub scale: f64,
    pub bounds: Rect,
}
impl Display {
    pub fn character_size(&self, character: &buddy_project::Character, scale: f64) -> Point {
        let sheet = character.sheet;
        // Reserve both mirrored extents so flipping about an off-center anchor
        // cannot clip the character or move its reference point.
        let width = sheet.frame_width as f64
            * 2.0
            * character.anchor.x.max(1.0 - character.anchor.x)
            * scale;
        let height = sheet.frame_height as f64 * scale;
        let fit = (self.bounds.width / width)
            .min(self.bounds.height / height)
            .min(1.0);
        Point {
            x: width * fit,
            y: height * fit,
        }
    }
    pub fn choice(&self) -> MonitorChoice {
        MonitorChoice {
            id: self.id.clone(),
            name: self.name.clone(),
            primary: self.primary,
            work_area: self.bounds,
        }
    }
    pub fn local_cursor(&self, x: f64, y: f64, macos: bool) -> Point {
        if macos {
            // CGEvent coordinates are global points. Never divide them by the
            // main display's scale when a different display is selected.
            Point {
                x: x - self.origin.x / self.scale,
                y: y - self.origin.y / self.scale,
            }
        } else {
            Point {
                x: (x - self.origin.x) / self.scale,
                y: (y - self.origin.y) / self.scale,
            }
        }
    }
    pub fn position_window(
        &self,
        window: &tauri::WebviewWindow,
        point: Point,
    ) -> tauri::Result<()> {
        if cfg!(target_os = "macos") {
            window.set_position(tauri::LogicalPosition::new(
                self.origin.x / self.scale + point.x,
                self.origin.y / self.scale + point.y,
            ))
        } else {
            window.set_position(tauri::PhysicalPosition::new(
                (self.origin.x + point.x * self.scale).round() as i32,
                (self.origin.y + point.y * self.scale).round() as i32,
            ))
        }
    }
}

pub fn choose(displays: &[Display], hint: Option<&str>) -> Option<Display> {
    displays
        .iter()
        .find(|display| Some(display.id.as_str()) == hint)
        .or_else(|| displays.iter().find(|display| display.primary))
        .or_else(|| displays.first())
        .cloned()
}

pub fn list(app: &tauri::AppHandle) -> Result<Vec<Display>, String> {
    let primary = app.primary_monitor().map_err(|e| e.to_string())?;
    let mut monitors = app.available_monitors().map_err(|e| e.to_string())?;
    monitors.sort_by_key(|m| (m.position().x, m.position().y));
    let mut occurrences = std::collections::BTreeMap::<String, usize>::new();
    let mut displays = Vec::new();
    for monitor in monitors {
        let name = monitor
            .name()
            .cloned()
            .unwrap_or_else(|| "디스플레이".into());
        let ordinal = occurrences.entry(name.clone()).or_default();
        // A persisted hint, not a hardware identity. Duplicate names use desktop
        // ordering; a missing hint always falls back to a visible primary screen.
        let id = format!(
            "display-{}",
            &buddy_project::sha256(format!("{name}:{ordinal}").as_bytes())[..16]
        );
        *ordinal += 1;
        let scale = monitor.scale_factor();
        let work = monitor.work_area();
        if !scale.is_finite() || scale <= 0.0 || work.size.width == 0 || work.size.height == 0 {
            continue;
        }
        displays.push(Display {
            id,
            name,
            primary: primary
                .as_ref()
                .is_some_and(|p| p.position() == monitor.position() && p.size() == monitor.size()),
            origin: Point {
                x: work.position.x as f64,
                y: work.position.y as f64,
            },
            scale,
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: work.size.width as f64 / scale,
                height: work.size.height as f64 / scale,
            },
        });
    }
    Ok(displays)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display(id: &str, primary: bool, x: f64, scale: f64) -> Display {
        Display {
            id: id.into(),
            name: id.into(),
            primary,
            origin: Point { x, y: 50.0 * scale },
            scale,
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 700.0,
            },
        }
    }
    #[test]
    fn off_center_anchor_reserves_both_mirrored_extents_and_fits_the_work_area() {
        let mut character: buddy_project::Character = serde_json::from_slice(include_bytes!(
            "../../../../examples/typing-cat/character/manifest.json"
        ))
        .unwrap();
        character.anchor.x = 0.0;
        let monitor = display("primary", true, 0.0, 1.0);
        assert_eq!(
            monitor.character_size(&character, 1.0),
            Point { x: 512.0, y: 256.0 }
        );
        let size = monitor.character_size(&character, 3.0);
        assert_eq!(size.x, 1000.0);
        assert_eq!(size.y, 500.0);
    }
    #[test]
    fn missing_monitor_recovers_to_primary_and_clamps_placement() {
        let monitors = [
            display("external", false, -2000.0, 2.0),
            display("primary", true, 0.0, 1.0),
        ];
        assert_eq!(choose(&monitors, Some("external")).unwrap().id, "external");
        let fallback = choose(&monitors[1..], Some("external")).unwrap();
        assert_eq!(fallback.id, "primary");
        assert_eq!(
            fallback
                .bounds
                .clamp(1500.0, -100.0, Point { x: 200.0, y: 200.0 }),
            Point { x: 800.0, y: 0.0 }
        );
    }
    #[test]
    fn mixed_dpi_and_negative_origins_use_selected_monitor_coordinates() {
        let secondary = display("external", false, -2000.0, 2.0);
        assert_eq!(
            secondary.local_cursor(-1800.0, 300.0, false),
            Point { x: 100.0, y: 100.0 }
        );
        assert_eq!(
            secondary.local_cursor(-900.0, 150.0, true),
            Point { x: 100.0, y: 100.0 }
        );
    }
}
