const TIMEOUT_MS: u64 = 6000;

#[derive(Default)]
pub struct Recovery {
    pub generation: u32,
    pub deadline: Option<u64>,
    connected: bool,
    failed: bool,
}

impl Recovery {
    pub fn start(&mut self, now: u64) -> bool {
        if self.deadline.is_some() {
            return false;
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.deadline = Some(now.saturating_add(TIMEOUT_MS));
        self.connected = false;
        self.failed = false;
        true
    }

    pub fn require(&self, generation: u32) -> Result<(), String> {
        if generation == self.generation {
            Ok(())
        } else {
            Err("이전 버디 화면의 요청입니다.".into())
        }
    }

    pub fn connected(&mut self, hidden: bool) {
        self.connected = true;
        if hidden {
            self.complete();
        }
    }

    pub fn poll(&mut self, now: u64, hidden: bool) {
        if self.deadline.is_some() && self.connected && hidden {
            self.complete();
        } else if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.fail();
        }
    }

    pub fn complete(&mut self) {
        self.deadline = None;
        self.failed = false;
    }

    pub fn fail(&mut self) {
        self.deadline = None;
        self.failed = true;
    }

    pub fn menu_status(&self) -> Option<&'static str> {
        if self.deadline.is_some() {
            Some("버디 화면을 다시 불러오는 중…")
        } else if self.failed {
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
    fn recovery_ignores_duplicates_rejects_old_pages_and_times_out_without_a_frame() {
        let mut recovery = Recovery::default();
        assert!(recovery.start(10));
        let generation = recovery.generation;
        assert!(!recovery.start(11));
        assert!(recovery.require(0).is_err());
        assert!(recovery.require(generation).is_ok());
        recovery.connected(false);
        recovery.poll(10 + TIMEOUT_MS, false);
        assert!(recovery.menu_status().unwrap().contains("실패"));
        assert!(recovery.start(10 + TIMEOUT_MS));
        assert!(recovery.require(generation).is_err());
        recovery.connected(true);
        assert_eq!(recovery.menu_status(), None);
    }
}
