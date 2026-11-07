use crate::{
    matcher::SequenceMatcher, Controls, FrameAck, FramePerformance, InputEvent,
    PerformanceSnapshot, ReactionEnd, Rect, PROTOCOL_VERSION,
};
use buddy_project::{
    is_modifier, is_supported_key, Effect, MouseButton, Point, Retrigger, Trigger, ValidatedProject,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CharacterState {
    pub protocol_version: u32,
    pub epoch: u32,
    pub action_instance_id: u32,
    pub animation: String,
    pub animation_elapsed_ms: f64,
    pub source: AnimationSource,
    pub facing_left: bool,
    pub position: Point,
    pub typing_band_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum AnimationSource {
    Idle,
    Movement,
    TypingRate,
    OneShot,
    HeldKey,
    Drag,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiagnostics {
    pub epoch_resets: u32,
    pub rate_overflow_resets: u32,
}

struct OneShot {
    binding: usize,
    ends_at: u64,
}
struct PendingBand {
    band: Option<usize>,
    since: u64,
}

pub struct Runtime {
    data: ValidatedProject,
    controls: Controls,
    held: BTreeSet<String>,
    blocked: BTreeSet<String>,
    mouse_held: Vec<MouseButton>,
    mouse_blocked: Vec<MouseButton>,
    move_blocked: BTreeSet<String>,
    matchers: Vec<Option<SequenceMatcher>>,
    last_start: Vec<Option<u64>>,
    rate_times: VecDeque<u64>,
    diagnostics: RuntimeDiagnostics,
    performance: FramePerformance,
    rate_band: Option<usize>,
    pending_band: Option<PendingBand>,
    rate_next: Option<u64>,
    one_shot: Option<OneShot>,
    now: u64,
    epoch: u32,
    bounds: Rect,
    size: Point,
    position: Point,
    moving: bool,
    dragging: bool,
    shot_release: Option<InputEvent>,
    cursor: Option<Point>,
    facing_left: bool,
    display: String,
    display_source: AnimationSource,
    display_binding: Option<usize>,
    animation_started_at: u64,
    action_instance: u32,
}

impl Runtime {
    pub fn new(data: ValidatedProject, bounds: Rect, position: Point, size: Point) -> Self {
        let matchers = data
            .sequences
            .iter()
            .zip(&data.project.bindings)
            .map(|(steps, binding)| {
                if let (Some(steps), Trigger::KeySequence { timeout_ms, .. }) =
                    (steps, &binding.trigger)
                {
                    Some(SequenceMatcher::new(steps.clone(), *timeout_ms))
                } else {
                    None
                }
            })
            .collect();
        let defaults = &data.project.defaults;
        let controls = Controls {
            active: true,
            keyboard: defaults.input.keyboard,
            mouse: defaults.input.mouse,
            movement_enabled: defaults.movement_enabled,
        };
        let count = data.project.bindings.len();
        Self {
            data,
            controls,
            held: BTreeSet::new(),
            blocked: BTreeSet::new(),
            mouse_held: Vec::new(),
            mouse_blocked: Vec::new(),
            move_blocked: BTreeSet::new(),
            matchers,
            last_start: vec![None; count],
            rate_times: VecDeque::new(),
            diagnostics: RuntimeDiagnostics::default(),
            performance: FramePerformance::default(),
            rate_band: None,
            pending_band: None,
            rate_next: None,
            one_shot: None,
            now: 0,
            epoch: 0,
            bounds,
            size,
            position: bounds.clamp(position.x, position.y, size),
            moving: false,
            dragging: false,
            shot_release: None,
            cursor: None,
            facing_left: false,
            display: "idle".into(),
            display_source: AnimationSource::Idle,
            display_binding: None,
            animation_started_at: 0,
            action_instance: 1,
        }
    }
    pub fn project(&self) -> &ValidatedProject {
        &self.data
    }
    pub fn set_timing(&mut self, now: u64, timing: buddy_project::Timing) {
        if timing.is_valid() && self.data.project.defaults.timing != timing {
            self.data.project.defaults.timing = timing;
            self.reset(now);
        }
    }
    pub fn controls(&self) -> Controls {
        self.controls
    }
    pub fn reset(&mut self, now: u64) {
        self.diagnostics.epoch_resets = self.diagnostics.epoch_resets.saturating_add(1);
        self.now = self.now.max(now);
        if self.one_shot.is_some() {
            self.performance.end(
                self.now,
                self.epoch,
                self.action_instance,
                ReactionEnd::Cancelled,
            );
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.blocked.extend(self.held.iter().cloned());
        self.held.clear();
        for button in self.mouse_held.drain(..) {
            if !self.mouse_blocked.contains(&button) {
                self.mouse_blocked.push(button);
            }
        }
        self.move_blocked.clear();
        for matcher in self.matchers.iter_mut().flatten() {
            matcher.reset();
        }
        self.rate_times.clear();
        self.rate_band = None;
        self.pending_band = None;
        self.rate_next = None;
        self.last_start.fill(None);
        self.one_shot = None;
        self.shot_release = None;
        self.dragging = false;
        self.cursor = None;
        self.moving = false;
        self.sync_display(true);
    }
    pub fn set_controls(&mut self, now: u64, controls: Controls) {
        self.advance(now);
        if self.controls.active != controls.active
            || self.controls.keyboard != controls.keyboard
            || self.controls.mouse != controls.mouse
        {
            self.controls = controls;
            self.reset(now);
        } else {
            if self.controls.movement_enabled != controls.movement_enabled {
                self.move_blocked.extend(self.held.iter().cloned());
                self.moving = false;
            }
            self.controls = controls;
            self.sync_display(false);
        }
    }
    pub fn place(&mut self, now: u64, position: Point, size: Point, bounds: Rect) {
        self.advance(now);
        self.bounds = bounds;
        self.size = size;
        self.position = bounds.clamp(position.x, position.y, size);
        self.update_facing(0.0);
        self.sync_display(false);
    }
    pub fn handle(&mut self, now: u64, event: InputEvent) {
        self.handle_received(now, now, event);
    }
    pub fn handle_received(&mut self, now: u64, received_at: u64, event: InputEvent) {
        self.advance(now);
        if matches!(event, InputEvent::DragStart | InputEvent::DragEnd) {
            self.dragging = matches!(event, InputEvent::DragStart);
            self.sync_display(false);
            return;
        }
        let released = matches!((&self.shot_release, &event),
            (Some(InputEvent::KeyUp { code: a }), InputEvent::KeyUp { code: b }) if a == b)
            || matches!((&self.shot_release, &event),
            (Some(InputEvent::MouseUp { button: a }), InputEvent::MouseUp { button: b }) if a == b);
        if released && self.one_shot.is_some() {
            self.performance.end(
                self.now,
                self.epoch,
                self.action_instance,
                ReactionEnd::Cancelled,
            );
            self.one_shot = None;
            self.shot_release = None;
        }
        if let InputEvent::Rearm { keys, buttons } = event {
            self.reset(now);
            self.blocked = keys
                .into_iter()
                .filter(|code| is_supported_key(code))
                .take(128)
                .collect();
            self.mouse_blocked = buttons.into_iter().take(3).collect();
            return;
        }
        if matches!(event, InputEvent::Reset) {
            self.reset(now);
            return;
        }
        if let InputEvent::KeyUp { code } = &event {
            self.move_blocked.remove(code);
            if self.blocked.remove(code) {
                self.held.remove(code);
                return;
            }
        }
        if let InputEvent::MouseUp { button } = &event {
            self.mouse_held.retain(|held| held != button);
            if self.mouse_blocked.contains(button) {
                self.mouse_blocked.retain(|blocked| blocked != button);
                return;
            }
        }
        if matches!(&event, InputEvent::MouseDown { button } if self.mouse_blocked.contains(button))
        {
            return;
        }
        if !self.controls.active {
            return;
        }
        let keyboard = matches!(
            &event,
            InputEvent::KeyDown { .. } | InputEvent::KeyUp { .. }
        );
        if (keyboard && !self.controls.keyboard) || (!keyboard && !self.controls.mouse) {
            return;
        }
        let mut completed = BTreeSet::new();
        match &event {
            InputEvent::KeyDown { code, repeat } => {
                if !is_supported_key(code) {
                    self.reset_matchers();
                    return;
                }
                if self.blocked.contains(code) || (*repeat && !self.held.contains(code)) {
                    return;
                }
                if !repeat {
                    self.held.insert(code.clone());
                    if !is_modifier(code) {
                        if self.rate_times.len() >= 2000 {
                            self.diagnostics.rate_overflow_resets =
                                self.diagnostics.rate_overflow_resets.saturating_add(1);
                            self.rate_times.clear();
                            self.rate_band = None;
                            self.pending_band = None;
                        }
                        self.rate_times.push_back(self.now);
                        if self.rate_next.is_none() {
                            self.rate_next = Some(self.now);
                        }
                    }
                }
                let shortcut = self.held.iter().chain(self.blocked.iter()).any(|key| {
                    key.starts_with("Control") || key.starts_with("Alt") || key.starts_with("Meta")
                });
                if shortcut || matches!(code.as_str(), "Backspace" | "Delete") {
                    self.reset_matchers();
                } else if !repeat && !is_modifier(code) {
                    let shift = ["ShiftLeft", "ShiftRight"]
                        .iter()
                        .any(|key| self.held.contains(*key) || self.blocked.contains(*key));
                    for (index, matcher) in self.matchers.iter_mut().enumerate() {
                        if matcher
                            .as_mut()
                            .is_some_and(|matcher| matcher.push(code, shift, self.now))
                        {
                            completed.insert(index);
                        }
                    }
                }
            }
            InputEvent::KeyUp { code } => {
                self.held.remove(code);
            }
            InputEvent::MouseDown { button } => {
                if !self.mouse_held.contains(button) {
                    self.mouse_held.push(*button);
                }
                self.reset_matchers();
            }
            InputEvent::CursorMove { x, y } if x.is_finite() && y.is_finite() => {
                self.cursor = Some(Point { x: *x, y: *y })
            }
            _ => {}
        }
        let mut matched_count = 0;
        let mut cooldown_count = 0;
        let mut retrigger_count = 0;
        let mut candidates: Vec<usize> = self
            .data
            .project
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                if !matches!(binding.effect, Effect::PlayAnimation { .. }) {
                    return None;
                }
                let matched = match (&binding.trigger, &event) {
                    (
                        Trigger::KeyDown {
                            code,
                            repeat: allowed,
                        },
                        InputEvent::KeyDown {
                            code: input,
                            repeat,
                        },
                    ) => key_matches(code, input) && (!repeat || *allowed),
                    (Trigger::KeyUp { code }, InputEvent::KeyUp { code: input }) => {
                        key_matches(code, input)
                    }
                    (Trigger::MouseDown { button }, InputEvent::MouseDown { button: input })
                    | (Trigger::MouseUp { button }, InputEvent::MouseUp { button: input }) => {
                        button == input
                    }
                    (Trigger::Wheel { direction }, InputEvent::Wheel { direction: input }) => {
                        direction == input
                    }
                    (Trigger::KeySequence { .. }, _) => completed.contains(&index),
                    _ => false,
                };
                if !matched {
                    return None;
                }
                matched_count += 1;
                if let Effect::PlayAnimation {
                    cooldown_ms,
                    retrigger,
                    ..
                } = binding.effect
                {
                    if self.last_start[index].is_some_and(|start| {
                        self.now.saturating_sub(start) < u64::from(cooldown_ms)
                    }) {
                        cooldown_count += 1;
                        return None;
                    }
                    if retrigger == Retrigger::Ignore
                        && self
                            .one_shot
                            .as_ref()
                            .is_some_and(|shot| shot.binding == index)
                    {
                        retrigger_count += 1;
                        return None;
                    }
                }
                Some(index)
            })
            .collect();
        self.performance.candidates(
            matched_count,
            cooldown_count,
            retrigger_count,
            candidates.len().saturating_sub(1) as u32,
        );
        candidates.sort_by_key(|index| (std::cmp::Reverse(self.priority(*index)), *index));
        if let Some(index) = candidates.first() {
            self.try_start(*index, received_at.min(self.now));
            if self.one_shot.as_ref().is_some_and(|s| s.binding == *index) {
                if let Effect::PlayAnimation {
                    stop_on_release: true,
                    ..
                } = self.data.project.bindings[*index].effect
                {
                    self.shot_release = match &event {
                        InputEvent::KeyDown { code, .. } => {
                            Some(InputEvent::KeyUp { code: code.clone() })
                        }
                        InputEvent::MouseDown { button } => {
                            Some(InputEvent::MouseUp { button: *button })
                        }
                        _ => None,
                    };
                }
            }
        }
        if self.movement_vector() == Point::default() {
            self.moving = false;
        }
        self.evaluate_rate();
        self.update_facing(0.0);
        self.sync_display(false);
    }
    pub fn advance(&mut self, now: u64) {
        let now = now.max(self.now);
        let delta = now.saturating_sub(self.now).min(50) as f64 / 1000.0;
        self.now = now;
        self.performance.advance(now);
        let velocity = self.movement_vector();
        let next = self.bounds.clamp(
            self.position.x + velocity.x * delta,
            self.position.y + velocity.y * delta,
            self.size,
        );
        let dx = next.x - self.position.x;
        if delta > 0.0 {
            self.moving = next != self.position && self.controls.active;
        } else if velocity == Point::default() {
            self.moving = false;
        }
        self.position = next;
        if self
            .one_shot
            .as_ref()
            .is_some_and(|shot| now >= shot.ends_at)
        {
            self.performance.end(
                self.now,
                self.epoch,
                self.action_instance,
                ReactionEnd::Completed,
            );
            self.one_shot = None;
        }
        self.evaluate_rate();
        self.update_facing(dx);
        self.sync_display(false);
    }
    fn movement_vector(&self) -> Point {
        if !self.controls.active || !self.controls.keyboard || !self.controls.movement_enabled {
            return Point::default();
        }
        let (mut x, mut y, mut speed): (f64, f64, f64) = (0.0, 0.0, 0.0);
        for binding in &self.data.project.bindings {
            if let (
                Trigger::WhileKeyDown { code },
                Effect::Move {
                    x: dx,
                    y: dy,
                    speed_dip_per_second,
                    ..
                },
            ) = (&binding.trigger, &binding.effect)
            {
                if self.held.contains(code) && !self.move_blocked.contains(code) {
                    x += dx;
                    y += dy;
                    speed = speed.max(*speed_dip_per_second);
                }
            }
        }
        x = x.clamp(-1.0, 1.0);
        y = y.clamp(-1.0, 1.0);
        let length = x.hypot(y).max(1.0);
        Point {
            x: x / length * speed,
            y: y / length * speed,
        }
    }
    fn priority(&self, index: usize) -> u8 {
        if let Effect::PlayAnimation { priority, .. } = self.data.project.bindings[index].effect {
            priority
        } else {
            0
        }
    }
    fn try_start(&mut self, index: usize, received_at: u64) {
        if let Some(current) = &self.one_shot {
            if self.priority(index) < self.priority(current.binding)
                || (self.priority(index) == self.priority(current.binding)
                    && index > current.binding)
            {
                self.performance.priority_rejected();
                return;
            }
        }
        if let Effect::PlayAnimation {
            animation,
            duration_ms,
            ..
        } = &self.data.project.bindings[index].effect
        {
            let clip = &self.data.render_plan.animations[animation];
            let duration = duration_ms.map(u64::from).unwrap_or_else(|| {
                clip.frames
                    .iter()
                    .map(|f| f.duration_ms)
                    .sum::<f64>()
                    .ceil() as u64
            });
            let fps = if clip
                .frames
                .iter()
                .all(|f| (f.duration_ms - 1000.0 / 30.0).abs() < 0.01)
            {
                30
            } else {
                0
            };
            self.shot_release = None;
            if self.one_shot.is_some() {
                self.performance.end(
                    self.now,
                    self.epoch,
                    self.action_instance,
                    ReactionEnd::Interrupted,
                );
            }
            self.one_shot = Some(OneShot {
                binding: index,
                ends_at: self.now.saturating_add(duration),
            });
            self.last_start[index] = Some(self.now);
            self.sync_display(true);
            self.performance
                .start(received_at, self.epoch, self.action_instance, fps);
        }
    }
    fn evaluate_rate(&mut self) {
        let timing = self.data.project.defaults.timing;
        if self
            .rate_times
            .back()
            .is_some_and(|last| self.now.saturating_sub(*last) >= u64::from(timing.idle_timeout_ms))
        {
            self.rate_times.clear();
            self.rate_band = None;
            self.pending_band = None;
            self.rate_next = None;
            return;
        }
        if self.rate_next.is_none_or(|due| self.now < due) {
            return;
        }
        while self
            .rate_times
            .front()
            .is_some_and(|time| self.now.saturating_sub(*time) >= u64::from(timing.rate_window_ms))
        {
            self.rate_times.pop_front();
        }
        let rate = self.rate_times.len() as f64 * 1000.0 / f64::from(timing.rate_window_ms);
        let candidate = self.data.project.bindings.iter().position(|binding| matches!(binding.trigger, Trigger::TypingRate { min_kps, max_kps } if rate >= min_kps && rate < max_kps.unwrap_or(f64::INFINITY)));
        if timing.rate_stability_ms == 0 {
            self.rate_band = candidate;
            self.pending_band = None;
        } else if candidate == self.rate_band {
            self.pending_band = None;
        } else if let Some(pending) = &self.pending_band {
            if pending.band == candidate
                && self.now.saturating_sub(pending.since) >= u64::from(timing.rate_stability_ms)
            {
                self.rate_band = candidate;
                self.pending_band = None;
            } else if pending.band != candidate {
                self.pending_band = Some(PendingBand {
                    band: candidate,
                    since: self.now,
                });
            }
        } else {
            self.pending_band = Some(PendingBand {
                band: candidate,
                since: self.now,
            });
        }
        self.rate_next = if !self.rate_times.is_empty()
            || self.rate_band.is_some()
            || self.pending_band.is_some()
        {
            Some(
                (self.now + u64::from(timing.rate_evaluation_ms))
                    .min(
                        self.rate_times.back().copied().unwrap_or(self.now)
                            + u64::from(timing.idle_timeout_ms),
                    )
                    .min(
                        self.pending_band
                            .as_ref()
                            .map_or(u64::MAX, |p| p.since + u64::from(timing.rate_stability_ms)),
                    ),
            )
        } else {
            None
        };
    }
    fn update_facing(&mut self, dx: f64) {
        if self.controls.active
            && self.controls.mouse
            && self
                .data
                .project
                .bindings
                .iter()
                .any(|binding| matches!(binding.effect, Effect::FaceCursor))
        {
            if let Some(cursor) = self.cursor {
                let difference = cursor.x - (self.position.x + self.size.x / 2.0);
                if difference.abs() > 1.0 {
                    self.facing_left = difference < 0.0;
                }
                return;
            }
        }
        if dx.abs() > f64::EPSILON {
            self.facing_left = dx < 0.0;
        }
    }
    fn reset_matchers(&mut self) {
        for matcher in self.matchers.iter_mut().flatten() {
            matcher.reset();
        }
    }
    fn sync_display(&mut self, force: bool) {
        let sustained = self
            .data
            .project
            .bindings
            .iter()
            .enumerate()
            .find_map(|(i, b)| {
                if let Effect::SustainAnimation { animation } = &b.effect {
                    if self.dragging && matches!(b.trigger, Trigger::Drag) {
                        return Some((animation.clone(), AnimationSource::Drag, Some(i)));
                    }
                    if !self.dragging && self.controls.active && self.controls.keyboard {
                        if let Trigger::WhileKeyDown { code } = &b.trigger {
                            if self.held.contains(code) {
                                return Some((
                                    animation.clone(),
                                    AnimationSource::HeldKey,
                                    Some(i),
                                ));
                            }
                        }
                    }
                }
                None
            });
        let (animation, source, binding) = if self.dragging {
            sustained.unwrap_or_else(|| ("idle".into(), AnimationSource::Drag, None))
        } else if let Some(shot) = &self.one_shot {
            if let Effect::PlayAnimation { animation, .. } =
                &self.data.project.bindings[shot.binding].effect
            {
                (
                    animation.clone(),
                    AnimationSource::OneShot,
                    Some(shot.binding),
                )
            } else {
                unreachable!()
            }
        } else if let Some(held) = sustained {
            held
        } else if let Some(index) = self.rate_band {
            if let Effect::SustainAnimation { animation } =
                &self.data.project.bindings[index].effect
            {
                (animation.clone(), AnimationSource::TypingRate, Some(index))
            } else {
                unreachable!()
            }
        } else if self.moving {
            let movement =
                self.data
                    .project
                    .bindings
                    .iter()
                    .enumerate()
                    .find_map(|(index, binding)| {
                        if let (
                            Trigger::WhileKeyDown { code },
                            Effect::Move {
                                animation: Some(name),
                                ..
                            },
                        ) = (&binding.trigger, &binding.effect)
                        {
                            if self.held.contains(code) && !self.move_blocked.contains(code) {
                                return Some((
                                    name.clone(),
                                    AnimationSource::Movement,
                                    Some(index),
                                ));
                            }
                        }
                        None
                    });
            movement.unwrap_or_else(|| ("idle".into(), AnimationSource::Idle, None))
        } else {
            ("idle".into(), AnimationSource::Idle, None)
        };
        if force
            || self.display != animation
            || self.display_source != source
            || self.display_binding != binding
        {
            self.display = animation;
            self.display_source = source;
            self.display_binding = binding;
            self.animation_started_at = self.now;
            self.action_instance = self.action_instance.wrapping_add(1);
        }
    }
    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        self.diagnostics.clone()
    }
    pub fn performance(&mut self, now: u64) -> PerformanceSnapshot {
        self.performance.snapshot(now)
    }
    pub fn reset_performance(&mut self, now: u64) {
        self.performance.reset_session(now);
    }
    pub fn prepare_frame(&mut self, seq: u32, state: &CharacterState, visible: bool) {
        self.performance
            .prepare_snapshot(seq, state.epoch, state.action_instance_id, visible);
    }
    pub fn frame_send_failed(&mut self, seq: u32) {
        self.performance.send_failed(seq);
    }
    pub fn frame_reconnect(&mut self) {
        self.performance.reconnect();
    }
    pub fn accepts_frame(&self, ack: &FrameAck) -> bool {
        self.performance.accepts(ack)
    }
    pub fn ack_frame(&mut self, now: u64, ack: &FrameAck, visible: bool) -> bool {
        self.performance.ack(now, ack, visible)
    }
    pub fn state(&self) -> CharacterState {
        CharacterState {
            protocol_version: PROTOCOL_VERSION,
            epoch: self.epoch,
            action_instance_id: self.action_instance,
            animation: self.display.clone(),
            animation_elapsed_ms: self.now.saturating_sub(self.animation_started_at) as f64,
            source: self.display_source,
            facing_left: self.facing_left,
            position: self.position,
            typing_band_id: self
                .rate_band
                .map(|index| self.data.project.bindings[index].id.clone()),
        }
    }
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let movement = (self.movement_vector() != Point::default()).then_some(self.now + 16);
        [
            movement,
            self.rate_next,
            self.one_shot.as_ref().map(|shot| shot.ends_at),
            self.performance.next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

fn key_matches(pattern: &str, input: &str) -> bool {
    pattern == input || (pattern == "*" && is_supported_key(input) && !is_modifier(input))
}

pub fn typescript_declarations() -> String {
    use ts_rs::TS;
    let mut declarations: String = [
        CharacterState::decl(),
        RuntimeDiagnostics::decl(),
        AnimationSource::decl(),
        Controls::decl(),
        Rect::decl(),
        InputEvent::decl(),
    ]
    .into_iter()
    .map(|line| format!("export {line}\n"))
    .collect();
    declarations.push_str(&crate::protocol::declarations());
    declarations.push_str(&crate::performance::declarations());
    declarations
}
