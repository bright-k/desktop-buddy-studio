const LOAD_TIMEOUT_MS: u64 = 20_000;
const HEALTH_TIMEOUT_MS: u64 = 30_000;
const PAINT_TIMEOUT_MS: u64 = 15_000;
const RETRY_DELAYS_MS: [u64; 2] = [1_000, 4_000];
const MAX_ATTEMPTS: usize = RETRY_DELAYS_MS.len() + 1;

struct Delivery {
    latest: u32,
    deadline: u64,
}

#[derive(Default)]
pub struct Recovery {
    pub generation: u32,
    pub deadline: Option<u64>,
    connected: bool,
    attempts: usize,
    retry_at: Option<u64>,
    first_frame_deadline: Option<u64>,
    heartbeat_deadline: Option<u64>,
    paint_deadline: Option<u64>,
    pending_delivery: Option<Delivery>,
    last_seen: Option<u32>,
}

impl Recovery {
    pub fn start(&mut self, now: u64, manual: bool) -> bool {
        if self.deadline.is_some() {
            return false;
        }
        if manual {
            self.attempts = 0;
        } else if !self.retry_at.is_some_and(|at| now >= at) {
            return false;
        }
        self.attempts += 1;
        self.retry_at = None;
        self.generation = self.generation.wrapping_add(1).max(1);
        self.deadline = Some(now.saturating_add(LOAD_TIMEOUT_MS));
        self.connected = false;
        self.clear_watch();
        true
    }

    pub fn queue(&mut self, now: u64) {
        if self.deadline.is_none() && self.retry_at.is_none() && self.attempts < MAX_ATTEMPTS {
            self.retry_at = Some(now);
        }
    }

    pub fn replace_window(&self) -> bool {
        self.attempts > 1
    }

    pub fn in_progress(&self) -> bool {
        self.deadline.is_some() || self.retry_at.is_some()
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.deadline.or(self.retry_at)
    }

    pub fn require(&self, generation: u32) -> Result<(), String> {
        if generation == self.generation {
            Ok(())
        } else {
            Err("이전 버디 화면의 요청입니다.".into())
        }
    }

    pub fn connected(&mut self, now: u64, hidden: bool) {
        self.connected = true;
        self.first_frame_deadline = Some(now.saturating_add(LOAD_TIMEOUT_MS));
        self.heartbeat_deadline = Some(now.saturating_add(HEALTH_TIMEOUT_MS));
        self.paint_deadline = None;
        self.pending_delivery = None;
        self.last_seen = None;
        if hidden {
            self.complete();
        }
    }

    pub fn sent(&mut self, seq: u32, now: u64) {
        self.pending_delivery
            .get_or_insert(Delivery {
                latest: seq,
                deadline: now.saturating_add(HEALTH_TIMEOUT_MS),
            })
            .latest = seq;
    }

    pub fn heartbeat(&mut self, now: u64, seen: Option<u32>, paint_blocked: bool) {
        self.heartbeat_deadline = Some(now.saturating_add(HEALTH_TIMEOUT_MS));
        if paint_blocked {
            self.paint_deadline
                .get_or_insert(now.saturating_add(PAINT_TIMEOUT_MS));
        } else {
            self.paint_deadline = None;
        }
        let Some(seq) = seen else { return };
        let advanced = self
            .last_seen
            .is_none_or(|previous| (1..(1 << 31)).contains(&seq.wrapping_sub(previous)));
        if advanced {
            self.last_seen = seen;
        }
        if let Some(pending) = &mut self.pending_delivery {
            if seq.wrapping_sub(pending.latest) < (1 << 31) {
                self.pending_delivery = None;
            } else if advanced {
                pending.deadline = now.saturating_add(HEALTH_TIMEOUT_MS);
            }
        }
    }

    pub fn poll(&mut self, now: u64, hidden: bool, ready: bool, watch: bool) -> bool {
        if !watch {
            self.clear_watch();
            if !hidden {
                if self.deadline.is_some() {
                    self.deadline = Some(now.saturating_add(LOAD_TIMEOUT_MS));
                }
                return false;
            }
        }
        if let Some(deadline) = self.deadline {
            if hidden && self.connected {
                self.complete();
            } else if now >= deadline {
                self.fail(now);
            }
        } else if watch && self.retry_at.is_none() && self.attempts < MAX_ATTEMPTS {
            let overdue = if ready {
                now >= *self
                    .heartbeat_deadline
                    .get_or_insert(now.saturating_add(HEALTH_TIMEOUT_MS))
            } else {
                now >= *self
                    .first_frame_deadline
                    .get_or_insert(now.saturating_add(LOAD_TIMEOUT_MS))
            };
            if overdue
                || (ready && self.paint_deadline.is_some_and(|deadline| now >= deadline))
                || self
                    .pending_delivery
                    .as_ref()
                    .is_some_and(|pending| now >= pending.deadline)
            {
                self.queue(now);
            }
        }
        self.retry_at.is_some_and(|at| now >= at)
    }

    pub fn complete(&mut self) {
        self.deadline = None;
        self.retry_at = None;
        self.attempts = 0;
        self.first_frame_deadline = None;
        self.paint_deadline = None;
    }

    pub fn fail(&mut self, now: u64) {
        self.connected = false;
        if self.deadline.take().is_some() {
            self.retry_at = RETRY_DELAYS_MS
                .get(self.attempts - 1)
                .map(|delay| now.saturating_add(*delay));
        } else {
            self.queue(now);
        }
    }

    fn clear_watch(&mut self) {
        self.first_frame_deadline = None;
        self.heartbeat_deadline = None;
        self.paint_deadline = None;
        self.pending_delivery = None;
        self.last_seen = None;
    }

    pub fn menu_status(&self) -> Option<&'static str> {
        if self.in_progress() {
            Some("버디 화면을 다시 불러오는 중…")
        } else if self.attempts == MAX_ATTEMPTS {
            Some("버디 화면 복구 실패 · 버디 옵션에서 다시 시도")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_retries_and_stops_until_requested_again() {
        let mut recovery = Recovery::default();
        assert!(recovery.start(10, true));
        let generation = recovery.generation;
        assert!(!recovery.start(11, true));
        assert!(recovery.require(0).is_err());
        recovery.connected(12, false);
        let mut timeout = 10 + LOAD_TIMEOUT_MS;
        for delay in RETRY_DELAYS_MS {
            assert!(!recovery.poll(timeout, false, false, true));
            assert!(!recovery.poll(timeout + delay - 1, false, false, true));
            assert!(recovery.poll(timeout + delay, false, false, true));
            assert!(recovery.start(timeout + delay, false));
            assert!(recovery.replace_window());
            timeout += delay + LOAD_TIMEOUT_MS;
        }
        assert!(!recovery.poll(timeout, false, false, true));
        assert!(recovery.menu_status().unwrap().contains("실패"));
        assert!(!recovery.poll(timeout + HEALTH_TIMEOUT_MS, false, false, true));
        assert!(recovery.start(timeout + HEALTH_TIMEOUT_MS, true));
        assert!(recovery.require(generation).is_err());
        recovery.connected(timeout + HEALTH_TIMEOUT_MS, true);
        assert_eq!(recovery.menu_status(), None);
    }

    #[test]
    fn healthy_heartbeats_do_not_mask_stalled_state_delivery() {
        let mut recovery = Recovery::default();
        recovery.connected(0, false);
        recovery.complete();
        recovery.sent(u32::MAX, 1);
        recovery.heartbeat(10_000, Some(0), false);
        assert!(!recovery.poll(HEALTH_TIMEOUT_MS + 1, false, true, true));
        recovery.sent(1, HEALTH_TIMEOUT_MS + 1);
        recovery.sent(2, HEALTH_TIMEOUT_MS + 2);
        recovery.heartbeat(2 * HEALTH_TIMEOUT_MS, Some(0), false);
        assert!(recovery.poll(2 * HEALTH_TIMEOUT_MS + 1, false, true, true));
    }

    #[test]
    fn a_continuing_paint_failure_recovers_even_with_healthy_delivery() {
        let mut recovery = Recovery::default();
        recovery.connected(0, false);
        recovery.complete();
        recovery.heartbeat(1, Some(1), true);
        recovery.heartbeat(5_000, Some(1), false);
        assert!(!recovery.poll(PAINT_TIMEOUT_MS + 1, false, true, true));
        recovery.heartbeat(20_000, Some(2), true);
        recovery.heartbeat(25_000, Some(3), true);
        recovery.heartbeat(30_000, Some(4), true);
        assert!(!recovery.poll(34_999, false, true, true));
        assert!(recovery.poll(35_000, false, true, true));
    }

    #[test]
    fn partial_delivery_leaves_the_latest_state_under_watch() {
        let mut recovery = Recovery::default();
        recovery.connected(0, false);
        recovery.complete();
        recovery.sent(3, 1);
        recovery.sent(4, 2);
        recovery.heartbeat(10_000, Some(3), false);
        assert!(!recovery.poll(HEALTH_TIMEOUT_MS + 1, false, true, true));
        recovery.heartbeat(20_000, Some(3), false);
        assert!(recovery.poll(10_000 + HEALTH_TIMEOUT_MS, false, true, true));
    }

    #[test]
    fn hidden_or_suspended_webviews_get_a_fresh_timeout_on_resume() {
        let mut recovery = Recovery::default();
        recovery.connected(0, true);
        recovery.sent(1, 0);
        assert!(!recovery.poll(HEALTH_TIMEOUT_MS * 2, true, false, false));
        let resume = HEALTH_TIMEOUT_MS * 3;
        assert!(!recovery.poll(resume, false, false, true));
        recovery.heartbeat(resume + 1, Some(1), false);
        assert!(recovery.poll(resume + LOAD_TIMEOUT_MS, false, false, true));
    }

    #[test]
    fn suspension_does_not_consume_recovery_attempts() {
        let mut recovery = Recovery::default();
        recovery.queue(0);
        let resume = LOAD_TIMEOUT_MS * 3;
        assert!(!recovery.poll(resume, false, false, false));
        assert!(recovery.poll(resume, false, false, true));
        assert!(recovery.start(resume, false));
        let wake = resume + LOAD_TIMEOUT_MS * 2;
        assert!(!recovery.poll(wake, false, false, false));
        assert_eq!(recovery.deadline, Some(wake + LOAD_TIMEOUT_MS));
        assert!(!recovery.poll(wake + LOAD_TIMEOUT_MS - 1, false, false, true));
        assert!(!recovery.replace_window());
    }
}
