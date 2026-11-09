use buddy_project::Point;
use buddy_runtime::{MonitorChoice, Rect};

const EDGE_INSET_DIP: f64 = 6.0;

pub fn placement_bounds(area: Rect, size: Point, footprint: Option<(Point, Point)>) -> Rect {
    placement_with_margin(area, size, footprint, EDGE_INSET_DIP)
}

pub fn placement_with_margin(
    area: Rect,
    size: Point,
    footprint: Option<(Point, Point)>,
    margin: f64,
) -> Rect {
    let (min, max) = footprint.unwrap_or((Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 }));
    let slack_x = (area.width - (max.x - min.x) * size.x).max(0.0);
    let slack_y = (area.height - (max.y - min.y) * size.y).max(0.0);
    let margin = margin.max(EDGE_INSET_DIP);
    let inset_x = margin.min(slack_x / 2.0);
    let inset_y = margin.min(slack_y / 2.0);
    Rect {
        x: area.x + inset_x - min.x * size.x,
        y: area.y + inset_y - min.y * size.y,
        width: size.x + (slack_x - 2.0 * inset_x),
        height: size.y + (slack_y - 2.0 * inset_y),
    }
}

#[derive(Clone, PartialEq)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub primary: bool,
    /// Tauri physical work-area origin. On macOS each screen uses its own scale.
    pub origin: Point,
    pub scale: f64,
    pub bounds: Rect,
    pub desktop_area: Rect,
}
impl Display {
    pub fn fit_size(&self, size: Point) -> Point {
        let fit = ((self.bounds.width - 2.0 * EDGE_INSET_DIP).max(1.0) / size.x)
            .min((self.bounds.height - 2.0 * EDGE_INSET_DIP).max(1.0) / size.y)
            .min(1.0);
        Point {
            x: size.x * fit,
            y: size.y * fit,
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

pub fn at_cursor(displays: &[Display], cursor: Point) -> Option<&Display> {
    displays.iter().find(|display| {
        let area = display.desktop_area;
        cursor.x >= area.x
            && cursor.x < area.x + area.width
            && cursor.y >= area.y
            && cursor.y < area.y + area.height
    })
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
        let desktop_scale = if cfg!(target_os = "macos") {
            scale
        } else {
            1.0
        };
        let position = monitor.position();
        let size = monitor.size();
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
            desktop_area: Rect {
                x: position.x as f64 / desktop_scale,
                y: position.y as f64 / desktop_scale,
                width: size.width as f64 / desktop_scale,
                height: size.height as f64 / desktop_scale,
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
            desktop_area: Rect {
                x: x / scale,
                y: 0.0,
                width: 1000.0,
                height: 750.0,
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
            monitor.fit_size(character.canvas_size(1.0)),
            Point { x: 512.0, y: 256.0 }
        );
        let size = monitor.fit_size(character.canvas_size(3.0));
        assert_eq!(size.x, 988.0);
        assert_eq!(size.y, 494.0);
        character.animations.get_mut("jump").unwrap().scale_percent = Some(150);
        assert_eq!(
            monitor.fit_size(character.canvas_size(1.0)),
            Point { x: 768.0, y: 384.0 }
        );
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
    #[test]
    fn drag_target_crosses_the_boundary_in_desktop_coordinates() {
        let monitors = [
            display("external", false, -2000.0, 2.0),
            display("primary", true, 0.0, 1.0),
        ];
        assert_eq!(
            at_cursor(&monitors, Point { x: -1.0, y: 10.0 }).unwrap().id,
            "external"
        );
        assert_eq!(
            at_cursor(&monitors, Point { x: 0.0, y: 10.0 }).unwrap().id,
            "primary"
        );
        assert!(at_cursor(
            &monitors,
            Point {
                x: -1001.0,
                y: 10.0
            }
        )
        .is_none());
    }

    #[test]
    fn transparent_padding_can_cross_the_work_area_but_ink_keeps_six_dip() {
        let size = Point { x: 200.0, y: 100.0 };
        let area = display("primary", true, 0.0, 1.0).bounds;
        let bounds = placement_bounds(
            area,
            size,
            Some((Point { x: 0.25, y: 0.5 }, Point { x: 0.75, y: 0.875 })),
        );
        assert_eq!(
            bounds.clamp(-200.0, -100.0, size),
            Point { x: -44.0, y: -44.0 }
        );
        assert_eq!(
            bounds.clamp(1000.0, 700.0, size),
            Point { x: 844.0, y: 606.5 }
        );
        // An animation with more visible pixels needs a temporary correction;
        // its bounds do not change the position saved using the idle footprint.
        let idle_position = bounds.clamp(-200.0, -100.0, size);
        let larger = placement_bounds(
            area,
            size,
            Some((Point { x: 0.1, y: 0.2 }, Point { x: 0.9, y: 0.95 })),
        );
        assert_eq!(
            larger.clamp(idle_position.x, idle_position.y, size),
            Point { x: -14.0, y: -14.0 }
        );
    }
}
