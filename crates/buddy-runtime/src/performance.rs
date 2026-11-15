use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use ts_rs::TS;

const MAX_REACTIONS: usize = 512;
const MAX_TICKETS: usize = 256;
const ACK_GRACE_MS: u64 = 5000;
const HISTOGRAM_LIMIT_MS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum FrameOutcome {
    Drawn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameAck {
    pub seq: u32,
    pub epoch: u32,
    pub action_instance_id: u32,
    pub outcome: FrameOutcome,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub samples: u32,
    pub p50_ms: Option<u32>,
    pub p95_ms: Option<u32>,
    pub max_ms: Option<u32>,
    pub above_2000_ms: u32,
    pub p95_above_2000_ms: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSnapshot {
    pub session_duration_ms: u32,
    pub startup_frame_ms: Option<u32>,
    pub startup_error_frames: u32,
    pub candidates: u32,
    pub ignored_cooldown: u32,
    pub ignored_retrigger: u32,
    pub ignored_priority: u32,
    pub suppressed_same_input: u32,
    pub reactions_started: u32,
    pub reactions_interrupted: u32,
    pub reactions_cancelled: u32,
    pub ended_before_snapshot: u32,
    pub issued_without_ack: u32,
    pub pending_reactions: u32,
    pub render_errors: u32,
    pub invalid_or_duplicate_acks: u32,
    pub late_reaction_acks: u32,
    pub hidden_frame_acks: u32,
    pub snapshot_send_failures: u32,
    pub tracking_overflows: u32,
    pub latency_30_fps: LatencySummary,
    pub latency_other_fps: LatencySummary,
}

#[derive(Default)]
struct Histogram {
    buckets: Vec<u32>,
    samples: u32,
    max: u32,
    overflow: u32,
}
impl Histogram {
    fn add(&mut self, ms: u64) {
        if self.buckets.is_empty() {
            self.buckets.resize(HISTOGRAM_LIMIT_MS + 1, 0);
        }
        self.samples = self.samples.saturating_add(1);
        self.max = self.max.max(ms.min(u32::MAX as u64) as u32);
        if ms > HISTOGRAM_LIMIT_MS as u64 {
            self.overflow = self.overflow.saturating_add(1);
        } else {
            let i = ms as usize;
            self.buckets[i] = self.buckets[i].saturating_add(1);
        }
    }
    fn percentile(&self, percent: u32) -> Option<u32> {
        if self.samples == 0 {
            return None;
        }
        let rank = (u64::from(self.samples) * u64::from(percent)).div_ceil(100);
        let mut seen = 0u64;
        for (ms, count) in self.buckets.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                return Some(ms as u32);
            }
        }
        None // The quantile is above the bounded histogram's range.
    }
    fn summary(&self) -> LatencySummary {
        let p95 = self.percentile(95);
        LatencySummary {
            samples: self.samples,
            p50_ms: self.percentile(50),
            p95_ms: p95,
            max_ms: (self.samples != 0).then_some(self.max),
            above_2000_ms: self.overflow,
            p95_above_2000_ms: self.samples != 0 && p95.is_none(),
        }
    }
}

type FrameKey = (u32, u32);
struct Reaction {
    started: u64,
    fps: u32,
    issued: bool,
    ended: Option<u64>,
    error_reported: bool,
}
struct Ticket {
    seq: u32,
    key: FrameKey,
    reaction: bool,
    error_reported: bool,
}

/// Bounded correlation metadata only: never keys, text, binding names or an
/// input timeline. Only aggregate counts and quantiles leave this tracker.
#[derive(Default)]
pub struct FramePerformance {
    totals: PerformanceSnapshot,
    session_started: u64,
    pending: BTreeMap<FrameKey, Reaction>,
    tickets: VecDeque<Ticket>,
    settled: VecDeque<FrameKey>,
    latency_30: Histogram,
    latency_other: Histogram,
}
#[derive(Clone, Copy)]
pub enum ReactionEnd {
    Completed,
    Interrupted,
    Cancelled,
}
impl FramePerformance {
    pub fn reset_session(&mut self, now: u64) {
        let startup = self.totals.startup_frame_ms;
        let startup_errors = self.totals.startup_error_frames;
        *self = Self::default();
        self.session_started = now;
        self.totals.startup_frame_ms = startup;
        self.totals.startup_error_frames = startup_errors;
    }
    pub fn candidates(&mut self, matched: u32, cooldown: u32, retrigger: u32, suppressed: u32) {
        self.totals.candidates = self.totals.candidates.saturating_add(matched);
        self.totals.ignored_cooldown = self.totals.ignored_cooldown.saturating_add(cooldown);
        self.totals.ignored_retrigger = self.totals.ignored_retrigger.saturating_add(retrigger);
        self.totals.suppressed_same_input =
            self.totals.suppressed_same_input.saturating_add(suppressed);
    }
    pub fn priority_rejected(&mut self) {
        self.totals.ignored_priority = self.totals.ignored_priority.saturating_add(1);
    }
    pub fn start(&mut self, now: u64, epoch: u32, instance: u32, fps: u32) {
        self.advance(now);
        self.totals.reactions_started = self.totals.reactions_started.saturating_add(1);
        if self.pending.len() >= MAX_REACTIONS {
            // Disclose dropped tracking instead of making a partial p95 look complete.
            if let Some(key) = self
                .pending
                .iter()
                .min_by_key(|(_, reaction)| reaction.started)
                .map(|(key, _)| *key)
            {
                self.pending.remove(&key);
            }
            self.totals.tracking_overflows = self.totals.tracking_overflows.saturating_add(1);
        }
        self.pending.insert(
            (epoch, instance),
            Reaction {
                started: now,
                fps,
                issued: false,
                ended: None,
                error_reported: false,
            },
        );
    }
    pub fn end(&mut self, now: u64, epoch: u32, instance: u32, reason: ReactionEnd) {
        match reason {
            ReactionEnd::Interrupted => {
                self.totals.reactions_interrupted =
                    self.totals.reactions_interrupted.saturating_add(1)
            }
            ReactionEnd::Cancelled => {
                self.totals.reactions_cancelled = self.totals.reactions_cancelled.saturating_add(1)
            }
            ReactionEnd::Completed => {}
        }
        let key = (epoch, instance);
        if let Some(reaction) = self.pending.get_mut(&key) {
            if !reaction.issued {
                self.pending.remove(&key);
                self.totals.ended_before_snapshot =
                    self.totals.ended_before_snapshot.saturating_add(1);
            } else {
                reaction.ended = Some(now);
            }
        }
    }
    pub fn prepare_snapshot(&mut self, seq: u32, epoch: u32, instance: u32, visible: bool) {
        if !visible {
            return;
        }
        let key = (epoch, instance);
        if self.settled.contains(&key) {
            return;
        }
        if let Some(reaction) = self.pending.get_mut(&key) {
            reaction.issued = true;
        }
        if self.tickets.len() == MAX_TICKETS {
            let evicted = self.tickets.pop_front().unwrap();
            if evicted.reaction && !self.tickets.iter().any(|ticket| ticket.key == evicted.key) {
                self.totals.tracking_overflows = self.totals.tracking_overflows.saturating_add(1);
            }
        }
        let error_reported = self
            .tickets
            .iter()
            .any(|ticket| ticket.key == key && ticket.error_reported);
        self.tickets.push_back(Ticket {
            seq,
            key,
            reaction: self.pending.contains_key(&key),
            error_reported,
        });
    }
    pub fn send_failed(&mut self, seq: u32) {
        self.tickets.retain(|ticket| ticket.seq != seq);
        self.totals.snapshot_send_failures = self.totals.snapshot_send_failures.saturating_add(1);
    }
    pub fn reconnect(&mut self) {
        self.tickets.clear();
        self.settled.clear();
    }
    pub fn accepts(&self, ack: &FrameAck) -> bool {
        self.tickets.iter().any(|ticket| {
            ticket.seq == ack.seq
                && ticket.key == (ack.epoch, ack.action_instance_id)
                && (ack.outcome != FrameOutcome::Error || !ticket.error_reported)
        })
    }
    pub fn ack(&mut self, now: u64, ack: &FrameAck, visible: bool) -> bool {
        self.advance(now);
        if !self.accepts(ack) {
            self.totals.invalid_or_duplicate_acks =
                self.totals.invalid_or_duplicate_acks.saturating_add(1);
            return false;
        }
        let key = (ack.epoch, ack.action_instance_id);
        let was_reaction = self
            .tickets
            .iter()
            .any(|ticket| ticket.key == key && ticket.reaction);
        if ack.outcome == FrameOutcome::Error {
            for ticket in self.tickets.iter_mut().filter(|ticket| ticket.key == key) {
                ticket.error_reported = true;
            }
            if self.totals.startup_frame_ms.is_none() {
                self.totals.startup_error_frames =
                    self.totals.startup_error_frames.saturating_add(1);
            }
            if let Some(reaction) = self.pending.get_mut(&key) {
                if !reaction.error_reported {
                    self.totals.render_errors = self.totals.render_errors.saturating_add(1);
                    reaction.error_reported = true;
                }
            }
            return true;
        }
        self.tickets.retain(|ticket| ticket.key != key);
        if self.settled.len() == MAX_TICKETS {
            self.settled.pop_front();
        }
        self.settled.push_back(key);
        if !visible {
            self.totals.hidden_frame_acks = self.totals.hidden_frame_acks.saturating_add(1);
            self.pending.remove(&key);
            return true;
        }
        if visible && self.totals.startup_frame_ms.is_none() {
            self.totals.startup_frame_ms = Some(now.min(u32::MAX as u64) as u32);
        }
        if let Some(reaction) = self.pending.remove(&key) {
            let histogram = if reaction.fps == 30 {
                &mut self.latency_30
            } else {
                &mut self.latency_other
            };
            histogram.add(now.saturating_sub(reaction.started));
        } else if was_reaction {
            self.totals.late_reaction_acks = self.totals.late_reaction_acks.saturating_add(1);
        }
        true
    }
    pub fn advance(&mut self, now: u64) {
        let before = self.pending.len();
        self.pending.retain(|_, reaction| {
            reaction
                .ended
                .is_none_or(|ended| now.saturating_sub(ended) < ACK_GRACE_MS)
        });
        self.totals.issued_without_ack = self
            .totals
            .issued_without_ack
            .saturating_add((before - self.pending.len()) as u32);
    }
    pub fn next_deadline(&self) -> Option<u64> {
        self.pending
            .values()
            .filter_map(|reaction| reaction.ended.map(|end| end.saturating_add(ACK_GRACE_MS)))
            .min()
    }
    pub fn snapshot(&mut self, now: u64) -> PerformanceSnapshot {
        self.advance(now);
        let mut result = self.totals.clone();
        result.session_duration_ms = now
            .saturating_sub(self.session_started)
            .min(u32::MAX as u64) as u32;
        result.pending_reactions = self.pending.len() as u32;
        result.latency_30_fps = self.latency_30.summary();
        result.latency_other_fps = self.latency_other.summary();
        result
    }
}

pub(crate) fn declarations() -> String {
    [
        FrameOutcome::decl(),
        FrameAck::decl(),
        LatencySummary::decl(),
        PerformanceSnapshot::decl(),
    ]
    .into_iter()
    .map(|line| format!("export {line}\n"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ack(seq: u32, instance: u32, outcome: FrameOutcome) -> FrameAck {
        FrameAck {
            seq,
            epoch: 1,
            action_instance_id: instance,
            outcome,
        }
    }

    #[test]
    fn nearest_rank_quantiles_do_not_clamp_slow_samples_to_a_passing_value() {
        let mut histogram = Histogram::default();
        for ms in 0..100 {
            histogram.add(ms);
        }
        let summary = histogram.summary();
        assert_eq!(
            (summary.p50_ms, summary.p95_ms, summary.max_ms),
            (Some(49), Some(94), Some(99))
        );
        let mut histogram = Histogram::default();
        for _ in 0..94 {
            histogram.add(80);
        }
        for _ in 0..6 {
            histogram.add(2501);
        }
        let summary = histogram.summary();
        assert_eq!(summary.samples, 100);
        assert_eq!(summary.p95_ms, None);
        assert!(summary.p95_above_2000_ms);
        assert_eq!(summary.above_2000_ms, 6);
        assert_eq!(summary.max_ms, Some(2501));
    }

    #[test]
    fn only_issued_matching_drawn_frames_sample_latency_and_error_can_recover() {
        let mut p = FramePerformance::default();
        p.start(100, 1, 7, 30);
        p.prepare_snapshot(u32::MAX, 1, 7, true);
        p.prepare_snapshot(0, 1, 7, true);
        assert!(!p.ack(110, &ack(8, 7, FrameOutcome::Drawn), true));
        assert!(!p.ack(110, &ack(0, 8, FrameOutcome::Drawn), true));
        let mut wrong_epoch = ack(0, 7, FrameOutcome::Drawn);
        wrong_epoch.epoch = 0;
        assert!(!p.ack(110, &wrong_epoch, true));
        assert!(p.ack(120, &ack(0, 7, FrameOutcome::Error), true));
        assert!(!p.ack(125, &ack(u32::MAX, 7, FrameOutcome::Error), true));
        assert_eq!(p.snapshot(125).startup_frame_ms, None);
        assert!(p.ack(145, &ack(u32::MAX, 7, FrameOutcome::Drawn), true));
        assert!(!p.ack(146, &ack(0, 7, FrameOutcome::Drawn), true));
        let result = p.snapshot(150);
        assert_eq!(result.startup_frame_ms, Some(145));
        assert_eq!(result.startup_error_frames, 1);
        assert_eq!(result.render_errors, 1);
        assert_eq!(result.invalid_or_duplicate_acks, 5);
        assert_eq!(result.latency_30_fps.p95_ms, Some(45));
        assert_eq!(result.latency_other_fps.samples, 0);
        assert_eq!(result.pending_reactions, 0);
    }

    #[test]
    fn interruptions_unrendered_reactions_and_missing_or_late_acks_remain_visible() {
        let mut p = FramePerformance::default();
        p.start(0, 1, 1, 30);
        p.end(1, 1, 1, ReactionEnd::Interrupted);
        p.start(2, 1, 2, 30);
        p.prepare_snapshot(2, 1, 2, true);
        p.end(12, 1, 2, ReactionEnd::Interrupted);
        assert!(p.ack(32, &ack(2, 2, FrameOutcome::Drawn), true));
        p.start(100, 1, 3, 12);
        p.prepare_snapshot(3, 1, 3, true);
        p.end(200, 1, 3, ReactionEnd::Completed);
        assert_eq!(p.next_deadline(), Some(5200));
        assert_eq!(p.snapshot(5199).pending_reactions, 1);
        assert_eq!(p.snapshot(5200).issued_without_ack, 1);
        assert!(p.ack(5300, &ack(3, 3, FrameOutcome::Drawn), true));
        let result = p.snapshot(5400);
        assert_eq!(result.reactions_started, 3);
        assert_eq!(result.reactions_interrupted, 2);
        assert_eq!(result.ended_before_snapshot, 1);
        assert_eq!(result.late_reaction_acks, 1);
        assert_eq!(result.latency_30_fps.samples, 1);
        assert_eq!(result.latency_other_fps.samples, 0);
        assert_eq!(p.next_deadline(), None);
    }

    #[test]
    fn hidden_and_failed_deliveries_do_not_create_success_samples() {
        let mut p = FramePerformance::default();
        p.start(10, 1, 1, 30);
        p.prepare_snapshot(1, 1, 1, true);
        assert!(p.ack(20, &ack(1, 1, FrameOutcome::Drawn), false));
        p.start(30, 1, 2, 12);
        p.prepare_snapshot(2, 1, 2, true);
        p.send_failed(2);
        assert!(!p.ack(40, &ack(2, 2, FrameOutcome::Drawn), true));
        p.end(50, 1, 2, ReactionEnd::Cancelled);
        let result = p.snapshot(5050);
        assert_eq!(result.hidden_frame_acks, 1);
        assert_eq!(result.snapshot_send_failures, 1);
        assert_eq!(result.reactions_cancelled, 1);
        assert_eq!(result.issued_without_ack, 1);
        assert_eq!(result.startup_frame_ms, None);
        assert_eq!(
            result.latency_30_fps.samples + result.latency_other_fps.samples,
            0
        );
    }

    #[test]
    fn correlation_storage_is_bounded_and_tracking_losses_are_counted() {
        let mut p = FramePerformance::default();
        for i in 0..600 {
            p.start(i.into(), 1, i, 30);
        }
        assert_eq!(p.pending.len(), MAX_REACTIONS);
        assert_eq!(p.snapshot(600).tracking_overflows, 88);
        for i in 88..600 {
            p.prepare_snapshot(i, 1, i, true);
        }
        assert_eq!(p.tickets.len(), MAX_TICKETS);
        assert_eq!(p.snapshot(600).tracking_overflows, 344);
    }

    #[test]
    fn reconnect_and_session_reset_reject_old_acks_and_preserve_startup_only() {
        let mut p = FramePerformance::default();
        p.prepare_snapshot(1, 1, 1, true);
        assert!(p.ack(60, &ack(1, 1, FrameOutcome::Drawn), true));
        p.start(100, 1, 2, 12);
        p.prepare_snapshot(2, 1, 2, true);
        p.reconnect();
        assert!(!p.accepts(&ack(2, 2, FrameOutcome::Drawn)));
        p.prepare_snapshot(3, 1, 2, true);
        assert!(p.ack(150, &ack(3, 2, FrameOutcome::Drawn), true));
        assert_eq!(p.snapshot(160).latency_other_fps.p95_ms, Some(50));
        p.reset_session(200);
        let result = p.snapshot(205);
        assert_eq!(result.session_duration_ms, 5);
        assert_eq!(result.startup_frame_ms, Some(60));
        assert_eq!(result.reactions_started, 0);
        assert_eq!(result.latency_other_fps.samples, 0);
        assert!(!p.accepts(&ack(3, 2, FrameOutcome::Drawn)));
    }
}
