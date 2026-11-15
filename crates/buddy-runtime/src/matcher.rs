use buddy_project::KeyStep;

pub(crate) struct SequenceMatcher {
    pattern: Vec<KeyStep>,
    prefix: Vec<usize>,
    matched: usize,
    last_at: Option<u64>,
    timeout_ms: u64,
}
impl SequenceMatcher {
    pub fn new(pattern: Vec<KeyStep>, timeout_ms: u32) -> Self {
        let mut prefix = vec![0; pattern.len()];
        for i in 1..pattern.len() {
            let mut j = prefix[i - 1];
            while j > 0 && pattern[i] != pattern[j] {
                j = prefix[j - 1];
            }
            if pattern[i] == pattern[j] {
                j += 1;
            }
            prefix[i] = j;
        }
        Self {
            pattern,
            prefix,
            matched: 0,
            last_at: None,
            timeout_ms: timeout_ms.into(),
        }
    }
    pub fn reset(&mut self) {
        self.matched = 0;
        self.last_at = None;
    }
    pub fn push(&mut self, code: &str, shift: bool, now: u64) -> bool {
        if self
            .last_at
            .is_some_and(|last| now.saturating_sub(last) > self.timeout_ms)
        {
            self.matched = 0;
        }
        self.last_at = Some(now);
        let matches =
            |index: usize| self.pattern[index].code == code && self.pattern[index].shift == shift;
        while self.matched > 0 && !matches(self.matched) {
            self.matched = self.prefix[self.matched - 1];
        }
        if matches(self.matched) {
            self.matched += 1;
        }
        if self.matched == self.pattern.len() {
            self.matched = self.prefix[self.matched - 1];
            true
        } else {
            false
        }
    }
}
