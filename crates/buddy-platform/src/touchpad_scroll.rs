use buddy_project::WheelDirection;
use std::time::{Duration, Instant};

const START_DISTANCE_MM: f64 = 1.8;
const STEP_DISTANCE_MM: f64 = 2.8;
const STEP_INTERVAL: Duration = Duration::from_millis(45);
const CONTACT_TIMEOUT: Duration = Duration::from_millis(150);
const DUPLICATE_WINDOW: Duration = Duration::from_millis(60);

#[derive(Clone, Copy)]
pub(crate) struct Contact {
    pub id: u32,
    pub x_mm: f64,
    pub y_mm: f64,
}

pub(crate) enum TouchFrame {
    Released,
    OtherGesture,
    Pair([Contact; 2]),
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

struct Pan {
    reference: [Contact; 2],
    axis: Option<Axis>,
    last_frame: Instant,
    last_step: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct TouchpadScroll {
    pan: Option<Pan>,
    blocked_at: Option<Instant>,
}

impl TouchpadScroll {
    pub(crate) fn update(
        &mut self,
        frame: TouchFrame,
        at: Instant,
        reversed: bool,
    ) -> Option<WheelDirection> {
        let mut contacts = match frame {
            TouchFrame::Released => {
                *self = Self::default();
                return None;
            }
            TouchFrame::OtherGesture => {
                self.pan = None;
                self.blocked_at = Some(at);
                return None;
            }
            TouchFrame::Pair(contacts) => {
                if Self::within(self.blocked_at, at, CONTACT_TIMEOUT) {
                    self.blocked_at = Some(at);
                    return None;
                }
                self.blocked_at = None;
                contacts
            }
        };
        contacts.sort_by_key(|contact| contact.id);
        if contacts[0].id == contacts[1].id {
            self.pan = None;
            return None;
        }
        let Some(pan) = self.pan.as_mut().filter(|pan| {
            at.saturating_duration_since(pan.last_frame) <= CONTACT_TIMEOUT
                && contacts
                    .iter()
                    .zip(&pan.reference)
                    .all(|(a, b)| a.id == b.id)
        }) else {
            self.pan = Some(Pan {
                reference: contacts,
                axis: None,
                last_frame: at,
                last_step: None,
            });
            return None;
        };
        pan.last_frame = at;
        let [(ax, ay), (bx, by)] = [0, 1].map(|i| {
            (
                contacts[i].x_mm - pan.reference[i].x_mm,
                contacts[i].y_mm - pan.reference[i].y_mm,
            )
        });
        let (x, y) = ((ax + bx) * 0.5, (ay + by) * 0.5);
        // A pinch or rotation moves the fingers apart or in different directions.
        if (ax - bx).hypot(ay - by) > 2.0 + 0.75 * x.hypot(y) {
            self.pan = None;
            self.blocked_at = Some(at);
            return None;
        }
        let axis = pan.axis.unwrap_or(if x.abs() > y.abs() {
            Axis::Horizontal
        } else {
            Axis::Vertical
        });
        let (a, b, distance) = match axis {
            Axis::Horizontal => (ax, bx, x),
            Axis::Vertical => (ay, by, y),
        };
        let threshold = if pan.axis.is_some() {
            STEP_DISTANCE_MM
        } else {
            START_DISTANCE_MM
        };
        if distance.abs() < threshold
            || a * b <= 0.0
            || a.abs().min(b.abs()) < threshold * 0.25
            || pan
                .last_step
                .is_some_and(|last| at.saturating_duration_since(last) < STEP_INTERVAL)
        {
            return None;
        }
        pan.axis = Some(axis);
        pan.reference = contacts;
        pan.last_step = Some(at);
        // The default Windows direction scrolls opposite to finger movement.
        Some(match (axis, (distance > 0.0) == reversed) {
            (Axis::Horizontal, true) => WheelDirection::Right,
            (Axis::Horizontal, false) => WheelDirection::Left,
            (Axis::Vertical, true) => WheelDirection::Down,
            (Axis::Vertical, false) => WheelDirection::Up,
        })
    }

    fn within(timestamp: Option<Instant>, at: Instant, window: Duration) -> bool {
        timestamp.is_some_and(|time| at.saturating_duration_since(time) <= window)
    }
}

#[derive(Default)]
pub(crate) struct ScrollSources {
    anonymous_wheel: [Option<Instant>; 4],
    touchpad: [Option<Instant>; 4],
}

impl ScrollSources {
    /// Physical devices with a handle keep their own wheel stream.
    pub(crate) fn accept_mouse(
        &mut self,
        direction: WheelDirection,
        at: Instant,
        anonymous: bool,
    ) -> bool {
        if !anonymous {
            return true;
        }
        let slot = Self::slot(direction);
        if Self::recent(self.touchpad[slot], at) {
            return false;
        }
        self.anonymous_wheel[slot] = Some(at);
        true
    }

    pub(crate) fn accept_touchpad(&mut self, direction: WheelDirection, at: Instant) -> bool {
        let slot = Self::slot(direction);
        self.touchpad[slot] = Some(at);
        !Self::recent(self.anonymous_wheel[slot], at)
    }

    fn recent(timestamp: Option<Instant>, at: Instant) -> bool {
        timestamp.is_some_and(|time| at.saturating_duration_since(time) <= DUPLICATE_WINDOW)
    }

    fn slot(direction: WheelDirection) -> usize {
        match direction {
            WheelDirection::Up => 0,
            WheelDirection::Down => 1,
            WheelDirection::Left => 2,
            WheelDirection::Right => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(ax: f64, ay: f64, bx: f64, by: f64) -> TouchFrame {
        TouchFrame::Pair([
            Contact {
                id: 1,
                x_mm: ax,
                y_mm: ay,
            },
            Contact {
                id: 2,
                x_mm: bx,
                y_mm: by,
            },
        ])
    }

    #[test]
    fn slow_pan_accumulates_and_respects_system_direction() {
        let at = Instant::now();
        let mut scroll = TouchpadScroll::default();
        assert_eq!(scroll.update(pair(0., 0., 8., 0.), at, false), None);
        assert_eq!(
            scroll.update(pair(0., 1., 8., 1.), at + Duration::from_millis(20), false),
            None
        );
        assert_eq!(
            scroll.update(pair(0., 2., 8., 2.), at + Duration::from_millis(40), false),
            Some(WheelDirection::Up)
        );
        scroll.update(TouchFrame::Released, at, false);
        scroll.update(pair(0., 0., 8., 0.), at, true);
        assert_eq!(
            scroll.update(pair(3., 0., 11., 0.), at, true),
            Some(WheelDirection::Right)
        );
    }

    #[test]
    fn pinch_or_third_finger_blocks_pan_until_fingers_lift() {
        let at = Instant::now();
        let mut scroll = TouchpadScroll::default();
        scroll.update(pair(4., 0., 8., 0.), at, false);
        assert_eq!(scroll.update(pair(1., 0., 11., 0.), at, false), None);
        assert_eq!(scroll.update(pair(1., 5., 11., 5.), at, false), None);
        scroll.update(TouchFrame::Released, at, false);
        scroll.update(pair(4., 0., 8., 0.), at, false);
        scroll.update(TouchFrame::OtherGesture, at, false);
        assert_eq!(scroll.update(pair(4., 5., 8., 5.), at, false), None);
        scroll.update(TouchFrame::Released, at, false);
        scroll.update(pair(4., 0., 8., 0.), at, false);
        assert_eq!(
            scroll.update(pair(4., -3., 8., -3.), at, false),
            Some(WheelDirection::Down)
        );
    }

    #[test]
    fn only_nearby_same_direction_anonymous_wheel_input_is_deduplicated() {
        let at = Instant::now();
        let mut sources = ScrollSources::default();
        assert!(sources.accept_mouse(WheelDirection::Up, at, true));
        assert!(!sources.accept_touchpad(WheelDirection::Up, at));
        assert!(sources.accept_touchpad(WheelDirection::Left, at));
        assert!(!sources.accept_mouse(WheelDirection::Left, at, true));
        assert!(sources.accept_mouse(WheelDirection::Left, at, false));
        assert!(sources.accept_mouse(WheelDirection::Right, at, true));
        assert!(sources.accept_mouse(
            WheelDirection::Left,
            at + DUPLICATE_WINDOW + Duration::from_millis(1),
            true
        ));
    }
}
