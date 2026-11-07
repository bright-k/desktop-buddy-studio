use buddy_runtime::InputEvent;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender},
    Arc,
};
use std::time::Instant;

struct Envelope {
    epoch: u64,
    event: InputEvent,
    received_at: Instant,
}
pub(crate) struct EventSender {
    sender: SyncSender<Envelope>,
    epoch: Arc<AtomicU64>,
    wake: crate::WakeSignal,
}
pub(crate) struct EventReceiver {
    receiver: Receiver<Envelope>,
    epoch: Arc<AtomicU64>,
    observed: u64,
}
#[cfg(test)]
pub(crate) fn bounded(size: usize) -> (EventSender, EventReceiver) {
    bounded_with_signal(size, crate::WakeSignal::default())
}
pub(crate) fn bounded_with_signal(
    size: usize,
    wake: crate::WakeSignal,
) -> (EventSender, EventReceiver) {
    let (sender, receiver) = mpsc::sync_channel(size);
    let epoch = Arc::new(AtomicU64::new(0));
    (
        EventSender {
            sender,
            epoch: epoch.clone(),
            wake,
        },
        EventReceiver {
            receiver,
            epoch,
            observed: 0,
        },
    )
}
impl EventSender {
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    pub fn send(&self, event: InputEvent) {
        self.send_at(event, Instant::now());
    }
    pub fn send_at(&self, event: InputEvent, received_at: Instant) {
        let motion = matches!(event, InputEvent::CursorMove { .. });
        if matches!(
            self.sender.try_send(Envelope {
                epoch: self.epoch.load(Ordering::Acquire),
                event,
                received_at,
            }),
            Err(mpsc::TrySendError::Full(_))
        ) && !motion
        {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        self.wake.wake();
    }
    pub fn reset(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.wake.wake();
    }
}
impl EventReceiver {
    #[cfg(test)]
    pub fn poll(&mut self) -> Option<InputEvent> {
        self.poll_captured().map(|captured| captured.event)
    }
    pub fn poll_captured(&mut self) -> Option<crate::CapturedInput> {
        let current = self.epoch.load(Ordering::Acquire);
        if current != self.observed {
            self.observed = current;
            return Some(crate::CapturedInput {
                event: InputEvent::Reset,
                received_at: Instant::now(),
            });
        }
        while let Ok(envelope) = self.receiver.try_recv() {
            let current = self.epoch.load(Ordering::Acquire);
            if current != self.observed {
                self.observed = current;
                return Some(crate::CapturedInput {
                    event: InputEvent::Reset,
                    received_at: Instant::now(),
                });
            }
            if envelope.epoch == current {
                return Some(crate::CapturedInput {
                    event: envelope.event,
                    received_at: envelope.received_at,
                });
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_retains_callback_time_instead_of_poll_time() {
        let (tx, mut rx) = bounded(1);
        let received_at = Instant::now() - std::time::Duration::from_millis(150);
        tx.send_at(
            InputEvent::Wheel {
                direction: buddy_project::WheelDirection::Up,
            },
            received_at,
        );
        let captured = rx.poll_captured().unwrap();
        assert_eq!(captured.received_at, received_at);
        assert!(matches!(captured.event, InputEvent::Wheel { .. }));
    }
    #[test]
    fn overflow_invalidates_queued_transitions_before_new_events() {
        let (tx, mut rx) = bounded(1);
        tx.send(InputEvent::KeyDown {
            code: "ArrowRight".into(),
            repeat: false,
        });
        tx.send(InputEvent::KeyUp {
            code: "ArrowRight".into(),
        });
        assert!(matches!(rx.poll(), Some(InputEvent::Reset)));
        assert!(rx.poll().is_none());
        tx.send(InputEvent::KeyDown {
            code: "KeyA".into(),
            repeat: false,
        });
        assert!(matches!(rx.poll(),Some(InputEvent::KeyDown{code,..}) if code=="KeyA"));
    }
    #[test]
    fn dropping_coalescible_motion_does_not_erase_key_transition() {
        let (tx, mut rx) = bounded(1);
        tx.send(InputEvent::KeyDown {
            code: "KeyA".into(),
            repeat: false,
        });
        tx.send(InputEvent::CursorMove { x: 1.0, y: 2.0 });
        assert!(matches!(rx.poll(), Some(InputEvent::KeyDown { .. })));
    }
}
