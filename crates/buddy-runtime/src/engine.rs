use crate::{
    matcher::SequenceMatcher, Controls, FrameAck, FramePerformance, InputEvent,
    PerformanceSnapshot, ReactionEnd, Rect, PROTOCOL_VERSION,
};
use buddy_project::{
    is_modifier, is_supported_key, CursorDirection, Effect, IdleCursorMode, Interruption,
    KeyModifier, MotionVariation, MotionVariationMode, MouseButton, Point, PointerInput, Retrigger,
    SoundCue, SourceFacing, TimerCompletion, TimerPhase, Trigger, ValidatedProject, WheelDirection,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeSet, VecDeque},
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundCue>,
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
    CursorFollow,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiagnostics {
    pub epoch_resets: u32,
    pub rate_overflow_resets: u32,
}

struct OneShot {
    binding: usize,
    animation: String,
    ends_at: u64,
    exact_keys: bool,
}
struct MotionOrder {
    next: usize,
    deck: Vec<usize>,
    rng: fastrand::Rng,
}
impl MotionOrder {
    fn new(rng: fastrand::Rng) -> Self {
        Self {
            next: 0,
            deck: Vec::new(),
            rng,
        }
    }

    fn take(&mut self, variation: &MotionVariation) -> usize {
        let count = 1 + variation.alternatives.len();
        match variation.mode {
            MotionVariationMode::Cycle => {
                let choice = self.next;
                self.next = (self.next + 1) % count;
                choice
            }
            MotionVariationMode::Shuffle => {
                if self.next == self.deck.len() {
                    let previous = self.deck.last().copied();
                    self.deck.clear();
                    self.deck.extend(0..count);
                    self.rng.shuffle(&mut self.deck);
                    if Some(self.deck[0]) == previous {
                        self.deck.swap(0, self.rng.usize(1..count));
                    }
                    self.next = 0;
                }
                let choice = self.deck[self.next];
                self.next += 1;
                choice
            }
            MotionVariationMode::Weighted => {
                let weights = variation
                    .weights
                    .as_deref()
                    .expect("validated motion weights");
                let total = weights.iter().copied().map(u32::from).sum::<u32>();
                let mut ticket = self.rng.u32(..total);
                for (index, &weight) in weights.iter().enumerate() {
                    let weight = u32::from(weight);
                    if ticket < weight {
                        return index;
                    }
                    ticket -= weight;
                }
                unreachable!("ticket is below the total weight");
            }
        }
    }
}
struct PendingBand {
    band: Option<usize>,
    since: u64,
}

struct IdleTimer {
    binding: usize,
    interval: u64,
    due: u64,
}

struct TypingBurst {
    first_at: u64,
    last_at: u64,
    keys: u32,
}

struct TypingStop {
    binding: usize,
    min_typing_ms: u64,
    min_keys: u32,
    pause_ms: u64,
    min_interval_ms: u64,
    burst: Option<TypingBurst>,
    last_reaction: Option<u64>,
}

impl TypingStop {
    fn observe(&mut self, at: u64) {
        if let Some(burst) = &mut self.burst {
            if at <= burst.last_at.saturating_add(self.pause_ms) {
                burst.last_at = burst.last_at.max(at);
                burst.keys = (burst.keys + 1).min(self.min_keys);
                return;
            }
        }
        self.burst = Some(TypingBurst {
            first_at: at,
            last_at: at,
            keys: 1,
        });
    }

    fn deadline(&self) -> Option<u64> {
        let burst = self.burst.as_ref()?;
        (burst.keys >= self.min_keys && burst.last_at - burst.first_at >= self.min_typing_ms)
            .then(|| burst.last_at.saturating_add(self.pause_ms))
    }

    fn take_due(&mut self, now: u64) -> Option<usize> {
        let due = self.deadline()?;
        let overdue = now.checked_sub(due)?;
        self.burst = None;
        // A stalled worker should not replay a reaction long after the typing ended.
        (overdue <= 500
            && self
                .last_reaction
                .is_none_or(|last| due.saturating_sub(last) >= self.min_interval_ms))
        .then_some(self.binding)
    }
}

#[derive(Clone, Copy)]
enum TimerReaction {
    Started(TimerPhase),
    Completed(TimerCompletion),
}
impl TimerReaction {
    fn matches(self, trigger: &Trigger) -> bool {
        match self {
            Self::Started(phase) => trigger.matches_timer_start(phase),
            Self::Completed(phase) => trigger.matches_timer_completion(phase),
        }
    }
}

pub struct Runtime {
    data: ValidatedProject,
    controls: Controls,
    held: BTreeSet<String>,
    blocked: BTreeSet<String>,
    mouse_held: Vec<MouseButton>,
    mouse_blocked: Vec<MouseButton>,
    move_blocked: BTreeSet<String>,
    matchers: Vec<(usize, SequenceMatcher)>,
    last_start: Vec<Option<u64>>,
    motion_orders: Vec<Option<MotionOrder>>,
    rate_times: VecDeque<u64>,
    diagnostics: RuntimeDiagnostics,
    performance: FramePerformance,
    rate_band: Option<usize>,
    pending_band: Option<PendingBand>,
    rate_next: Option<u64>,
    typing_stop: Option<TypingStop>,
    one_shot: Option<OneShot>,
    visible: bool,
    ever_shown: bool,
    pending_show: Option<bool>,
    pending_timer: Option<(TimerReaction, u64)>,
    timer_phase: Option<TimerPhase>,
    idle_group: Option<usize>,
    idle_timers: Vec<IdleTimer>,
    timing_idle: bool,
    now: u64,
    epoch: u32,
    bounds: Rect,
    size: Point,
    position: Point,
    moving: bool,
    dragging: bool,
    shot_release: Option<InputEvent>,
    cursor: Option<Point>,
    drag_cursor: Option<Point>,
    // Host input is separate from optional global mouse event capture.
    native_follow: Option<(bool, Option<Point>, Point)>,
    follow_config: buddy_project::CursorFollow,
    movement_facing_left: Option<bool>,
    facing_left: bool,
    continuous_binding: Option<usize>,
    display: String,
    display_source: AnimationSource,
    display_binding: Option<usize>,
    animation_started_at: u64,
    action_instance: u32,
}

impl Runtime {
    pub fn new(data: ValidatedProject, bounds: Rect, position: Point, size: Point) -> Self {
        let matchers = data
            .project
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| match &binding.trigger {
                Trigger::KeySequence { timeout_ms, .. } => Some((index, *timeout_ms)),
                _ => None,
            })
            .flat_map(|(index, timeout_ms)| {
                data.sequences[index]
                    .iter()
                    .chain(&data.sequence_alternatives[index])
                    .map(move |steps| (index, SequenceMatcher::new(steps.clone(), timeout_ms)))
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
        let display = data.character.default_animation.clone();
        let follow_config = data.project.cursor_follow.clone().unwrap_or_default();
        let motion_orders = data
            .project
            .bindings
            .iter()
            .map(|binding| {
                matches!(
                    binding.effect,
                    Effect::PlayAnimation {
                        variation: Some(_),
                        ..
                    }
                )
                .then(|| MotionOrder::new(fastrand::Rng::new()))
            })
            .collect();
        let idle_timers = data
            .project
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(binding, entry)| {
                let Trigger::IdleFor { seconds } = entry.trigger else {
                    return None;
                };
                let interval = u64::from(seconds) * 1000;
                Some(IdleTimer {
                    binding,
                    interval,
                    due: interval,
                })
            })
            .collect();
        let typing_stop = data
            .project
            .bindings
            .iter()
            .enumerate()
            .find_map(|(binding, entry)| match entry.trigger {
                Trigger::TypingStopped {
                    min_typing_seconds,
                    min_keys,
                    pause_ms,
                    min_interval_seconds,
                } => Some(TypingStop {
                    binding,
                    min_typing_ms: u64::from(min_typing_seconds) * 1000,
                    min_keys,
                    pause_ms: u64::from(pause_ms),
                    min_interval_ms: u64::from(min_interval_seconds) * 1000,
                    burst: None,
                    last_reaction: None,
                }),
                _ => None,
            });
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
            motion_orders,
            rate_times: VecDeque::new(),
            diagnostics: RuntimeDiagnostics::default(),
            performance: FramePerformance::default(),
            rate_band: None,
            pending_band: None,
            rate_next: None,
            typing_stop,
            one_shot: None,
            // In-process simulations start on a desktop with the buddy visible.
            // The native overlay explicitly starts hidden before its first draw.
            visible: true,
            ever_shown: false,
            pending_show: None,
            pending_timer: None,
            timer_phase: None,
            idle_group: None,
            idle_timers,
            timing_idle: true,
            now: 0,
            epoch: 0,
            bounds,
            size,
            position: bounds.clamp(position.x, position.y, size),
            moving: false,
            dragging: false,
            shot_release: None,
            cursor: None,
            drag_cursor: None,
            native_follow: None,
            follow_config,
            movement_facing_left: None,
            facing_left: false,
            continuous_binding: None,
            display,
            display_source: AnimationSource::Idle,
            display_binding: None,
            animation_started_at: 0,
            action_instance: 1,
        }
    }
    pub fn project(&self) -> &ValidatedProject {
        &self.data
    }
    /// Resume an event's selection order in a fresh Studio simulation.
    pub fn prime_motion_variation(&mut self, binding_id: &str, seed: u32, starts: u32) -> bool {
        if starts > 10_000 {
            return false;
        }
        let Some(index) = self
            .data
            .project
            .bindings
            .iter()
            .position(|b| b.id == binding_id)
        else {
            return false;
        };
        let Effect::PlayAnimation {
            variation: Some(variation),
            ..
        } = &self.data.project.bindings[index].effect
        else {
            return false;
        };
        let mut order = MotionOrder::new(fastrand::Rng::with_seed(u64::from(seed)));
        for _ in 0..starts {
            order.take(variation);
        }
        self.motion_orders[index] = Some(order);
        true
    }
    pub fn displayed_binding_id(&self) -> Option<&str> {
        self.display_binding
            .map(|index| self.data.project.bindings[index].id.as_str())
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
        for (_, matcher) in &mut self.matchers {
            matcher.reset();
        }
        self.rate_times.clear();
        self.rate_band = None;
        self.pending_band = None;
        self.rate_next = None;
        if let Some(typing) = &mut self.typing_stop {
            typing.burst = None;
        }
        for (last_start, binding) in self.last_start.iter_mut().zip(&self.data.project.bindings) {
            if !binding.trigger.is_automatic() {
                *last_start = None;
            }
        }
        self.one_shot = None;
        self.shot_release = None;
        self.dragging = false;
        self.cursor = None;
        self.continuous_binding = None;
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
            if !controls.active {
                self.pending_show = None;
            }
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
        let position = bounds.clamp(position.x, position.y, size);
        if position != self.position {
            self.timing_idle = false;
        }
        self.position = position;
        self.sync_display(false);
    }
    pub fn set_drag_cursor(&mut self, cursor: Point) {
        self.drag_cursor = Some(cursor);
        self.sync_display(false);
    }
    pub fn set_native_cursor_follow(
        &mut self,
        enabled: bool,
        target: Option<Point>,
        center: Point,
    ) {
        self.native_follow = Some((enabled, target, center));
    }
    pub fn set_overlay_visible(&mut self, now: u64, visible: bool) {
        if self.visible == visible && (!visible || self.ever_shown) {
            return;
        }
        self.advance(now);
        self.visible = visible;
        if visible {
            self.pending_show = Some(!self.ever_shown);
            self.ever_shown = true;
        } else {
            self.pending_show = None;
            self.pending_timer = None;
            if let Some(typing) = &mut self.typing_stop {
                typing.burst = None;
            }
        }
        self.sync_display(false);
    }
    pub fn cancel_timer_reaction(&mut self) {
        self.pending_timer = None;
    }
    pub fn timer_reaction_pending(&self) -> bool {
        self.pending_timer.is_some()
    }
    pub fn set_timer_phase(&mut self, now: u64, phase: Option<TimerPhase>) {
        if self.timer_phase == phase {
            return;
        }
        self.advance(now);
        self.timer_phase = phase;
        self.idle_group = phase.and_then(|phase| {
            self.data
                .project
                .idle_groups
                .iter()
                .position(|group| group.states.contains(&phase))
        });
        if matches!(self.pending_timer, Some((TimerReaction::Started(started), _)) if Some(started) != phase)
        {
            self.pending_timer = None;
        }
        if self.one_shot.as_ref().is_some_and(|shot| {
            matches!(
                self.data.project.bindings[shot.binding].trigger,
                Trigger::TimerStarted { .. }
            )
        }) {
            self.performance.end(
                self.now,
                self.epoch,
                self.action_instance,
                ReactionEnd::Cancelled,
            );
            self.one_shot = None;
            self.shot_release = None;
        }
        self.sync_display(false);
    }
    pub fn handle(&mut self, now: u64, event: InputEvent) {
        self.handle_received(now, now, event);
    }
    pub fn handle_received(&mut self, now: u64, received_at: u64, event: InputEvent) {
        if let InputEvent::DiagonalWheel { left, up } = event {
            // Keep horizontal-before-vertical arbitration for distinct bindings.
            let started = self.handle_single_input(
                now,
                received_at,
                InputEvent::Wheel {
                    direction: if left {
                        WheelDirection::Left
                    } else {
                        WheelDirection::Right
                    },
                },
                None,
            );
            self.handle_single_input(
                now,
                received_at,
                InputEvent::Wheel {
                    direction: if up {
                        WheelDirection::Up
                    } else {
                        WheelDirection::Down
                    },
                },
                started,
            );
        } else {
            self.handle_single_input(now, received_at, event, None);
        }
    }

    fn handle_single_input(
        &mut self,
        now: u64,
        received_at: u64,
        event: InputEvent,
        started_in_sample: Option<usize>,
    ) -> Option<usize> {
        if let InputEvent::OverlayVisibility { visible } = event {
            self.set_overlay_visible(now, visible);
            return None;
        }
        if let InputEvent::TimerPhaseChanged { phase } = event {
            if self
                .data
                .project
                .timer
                .as_ref()
                .is_some_and(|timer| timer.enabled)
            {
                self.set_timer_phase(now, phase);
            }
            return None;
        }
        self.advance(now);
        let timer_reaction = match event {
            InputEvent::TimerStarted { phase } => Some(TimerReaction::Started(phase)),
            InputEvent::TimerCompleted { phase } => Some(TimerReaction::Completed(phase)),
            _ => None,
        };
        if let Some(reaction) = timer_reaction {
            if self.visible
                && self
                    .data
                    .project
                    .timer
                    .as_ref()
                    .is_some_and(|timer| timer.enabled)
                && self
                    .data
                    .project
                    .bindings
                    .iter()
                    .any(|binding| reaction.matches(&binding.trigger))
            {
                self.pending_timer = Some((reaction, self.now.saturating_add(3000)));
            }
            return None;
        }
        if matches!(event, InputEvent::DragDropped) {
            self.timing_idle = false;
        }
        if matches!(event, InputEvent::DragStart | InputEvent::DragEnd) {
            self.dragging = matches!(event, InputEvent::DragStart);
            if !self.dragging {
                self.drag_cursor = None;
            }
            self.sync_display(false);
            return None;
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
            return None;
        }
        if matches!(event, InputEvent::Reset) {
            self.reset(now);
            return None;
        }
        if let InputEvent::KeyUp { code } = &event {
            self.move_blocked.remove(code);
            if self.blocked.remove(code) {
                self.held.remove(code);
                if self.movement_vector() == Point::default() {
                    self.moving = false;
                }
                self.sync_display(false);
                return None;
            }
        }
        if let InputEvent::MouseUp { button } = &event {
            self.mouse_held.retain(|held| held != button);
            if self.mouse_blocked.contains(button) {
                self.mouse_blocked.retain(|blocked| blocked != button);
                return None;
            }
        }
        if matches!(&event, InputEvent::MouseDown { button } if self.mouse_blocked.contains(button))
        {
            return None;
        }
        if !self.controls.active {
            return None;
        }
        let keyboard = matches!(
            &event,
            InputEvent::KeyDown { .. } | InputEvent::KeyUp { .. }
        );
        let direct = matches!(
            &event,
            InputEvent::BuddyClick | InputEvent::BuddyLongPress | InputEvent::DragDropped
        );
        if (keyboard && !self.controls.keyboard)
            || (!keyboard && !direct && !self.controls.mouse)
            || (direct && self.dragging)
        {
            return None;
        }
        let mut completed = BTreeSet::new();
        match &event {
            InputEvent::KeyDown { code, repeat } => {
                if !is_supported_key(code) {
                    self.reset_matchers();
                    return None;
                }
                if self.blocked.contains(code) || (*repeat && !self.held.contains(code)) {
                    return None;
                }
                let fresh_press = !repeat && self.held.insert(code.clone());
                if !repeat && !is_modifier(code) {
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
                let shortcut = self.held.iter().chain(self.blocked.iter()).any(|key| {
                    key.starts_with("Control") || key.starts_with("Alt") || key.starts_with("Meta")
                });
                if fresh_press && !shortcut && self.visible && !self.dragging && is_typing_key(code)
                {
                    if let Some(typing) = &mut self.typing_stop {
                        typing.observe(received_at.min(self.now));
                    }
                }
                if shortcut || matches!(code.as_str(), "Backspace" | "Delete") {
                    self.reset_matchers();
                } else if !repeat && !is_modifier(code) {
                    let shift = ["ShiftLeft", "ShiftRight"]
                        .iter()
                        .any(|key| self.held.contains(*key) || self.blocked.contains(*key));
                    for (index, matcher) in &mut self.matchers {
                        if matcher.push(code, shift, self.now) {
                            completed.insert(*index);
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
            InputEvent::BuddyClick | InputEvent::BuddyLongPress => self.reset_matchers(),
            InputEvent::CursorMove { x, y } if x.is_finite() && y.is_finite() => {
                self.cursor = Some(Point { x: *x, y: *y })
            }
            _ => {}
        }
        let mut matched_count = 0;
        let mut cooldown_count = 0;
        let mut retrigger_count = 0;
        let active_modifiers = self.active_modifiers();
        let pointer_input = match &event {
            InputEvent::MouseDown { button } => Some(PointerInput::MouseDown { button: *button }),
            InputEvent::MouseUp { button } => Some(PointerInput::MouseUp { button: *button }),
            InputEvent::Wheel { direction } => Some(PointerInput::Wheel {
                direction: *direction,
            }),
            InputEvent::BuddyClick => Some(PointerInput::BuddyClick),
            _ => None,
        };
        let mut candidates: Vec<(usize, bool)> = self
            .data
            .project
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                if !matches!(binding.effect, Effect::PlayAnimation { .. }) {
                    return None;
                }
                let exact_keys = match (&binding.trigger, &event) {
                    (
                        Trigger::KeyDown {
                            repeat: allowed, ..
                        },
                        InputEvent::KeyDown {
                            code: input,
                            repeat,
                        },
                    ) if !repeat || *allowed => {
                        key_match_rank(&binding.trigger, input, active_modifiers)
                    }
                    (Trigger::KeyUp { .. }, InputEvent::KeyUp { code: input }) => {
                        key_match_rank(&binding.trigger, input, active_modifiers)
                    }
                    (Trigger::MouseDown { button }, InputEvent::MouseDown { button: input })
                    | (Trigger::MouseUp { button }, InputEvent::MouseUp { button: input }) => {
                        (button == input).then_some(false)
                    }
                    (Trigger::BuddyClick, InputEvent::BuddyClick) => Some(false),
                    (Trigger::BuddyLongPress { .. }, InputEvent::BuddyLongPress) => Some(false),
                    (Trigger::DragDropped, InputEvent::DragDropped) => Some(false),
                    (Trigger::Wheel { direction }, InputEvent::Wheel { direction: input }) => {
                        (direction == input).then_some(false)
                    }
                    (Trigger::MouseInput { inputs }, _) => pointer_input
                        .is_some_and(|input| inputs.contains(&input))
                        .then_some(false),
                    (Trigger::KeySequence { .. }, _) => completed.contains(&index).then_some(false),
                    _ => None,
                }?;
                matched_count += 1;
                // An already started binding still competes on the other axis.
                // The winning binding is only started once per scroll sample.
                if started_in_sample == Some(index) {
                    return Some((index, exact_keys));
                }
                if let Effect::PlayAnimation { retrigger, .. } = binding.effect {
                    if self.cooling_down(index) {
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
                Some((index, exact_keys))
            })
            .collect();
        self.performance.candidates(
            matched_count,
            cooldown_count,
            retrigger_count,
            candidates.len().saturating_sub(1) as u32,
        );
        candidates.sort_by_key(|&(index, exact_keys)| Reverse(self.priority(index, exact_keys)));
        let mut started = None;
        if let Some(&(index, exact_keys)) = candidates
            .first()
            .filter(|(index, _)| Some(*index) != started_in_sample)
        {
            started = self
                .try_start(index, exact_keys, received_at.min(self.now))
                .then_some(index);
            if started.is_some() {
                if let Effect::PlayAnimation {
                    stop_on_release: true,
                    interruption,
                    ..
                } = self.data.project.bindings[index].effect
                {
                    if interruption != Some(Interruption::Finish) {
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
        }
        if self.movement_vector() == Point::default() {
            self.moving = false;
        }
        self.evaluate_rate();
        self.sync_display(false);
        started
    }
    pub fn advance(&mut self, now: u64) {
        let now = now.max(self.now);
        let delta = now.saturating_sub(self.now).min(50) as f64 / 1000.0;
        self.now = now;
        self.performance.advance(now);
        let velocity = self.movement_vector();
        let step = if velocity != Point::default() {
            Point {
                x: velocity.x * delta,
                y: velocity.y * delta,
            }
        } else if let Some(follow) = self.follow_motion() {
            let distance = (follow.speed * delta).min(follow.remaining);
            Point {
                x: follow.direction.x * distance,
                y: follow.direction.y * distance,
            }
        } else {
            Point::default()
        };
        let next = self.bounds.clamp(
            self.position.x + step.x,
            self.position.y + step.y,
            self.size,
        );
        let dx = next.x - self.position.x;
        if delta > 0.0 {
            self.moving = velocity != Point::default() && next != self.position;
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
            self.shot_release = None;
        }
        self.evaluate_rate();
        if dx.abs() > f64::EPSILON {
            self.movement_facing_left = Some(dx < 0.0);
        }
        self.sync_display(false);
    }
    /// Evaluate automatic events after applying all input in the current batch.
    pub fn tick(&mut self, now: u64) {
        self.advance(now);
        let appearance = self.pending_show.take();
        let timer = self.pending_timer.and_then(|(reaction, expires)| {
            if self.now >= expires {
                self.pending_timer = None;
                return None;
            }
            let available = self.visible
                && self.controls.active
                && !self.dragging
                && !self.movement_available()
                && (matches!(
                    self.display_source,
                    AnimationSource::Idle | AnimationSource::TypingRate
                ) || self.one_shot.as_ref().is_some_and(|shot| {
                    self.data.project.bindings[shot.binding]
                        .trigger
                        .is_automatic()
                }));
            if !available {
                return None;
            }
            let candidate = self
                .data
                .project
                .bindings
                .iter()
                .enumerate()
                .filter(|(index, binding)| {
                    reaction.matches(&binding.trigger) && !self.cooling_down(*index)
                })
                .map(|(index, _)| index)
                .max_by_key(|&index| self.priority(index, false));
            if candidate.is_none() {
                self.pending_timer = None;
            }
            candidate
        });
        let typing = self
            .typing_stop
            .as_mut()
            .and_then(|typing| typing.take_due(self.now));
        let idle_due =
            self.timing_idle && self.idle_timers.iter().any(|timer| self.now >= timer.due);
        if timer.is_none() && typing.is_none() && appearance.is_none() && !idle_due {
            return;
        }
        let can_start = self.visible && self.controls.active && !self.movement_available();
        let typing = typing.filter(|&index| {
            can_start
                && self.controls.keyboard
                && matches!(
                    self.display_source,
                    AnimationSource::Idle | AnimationSource::TypingRate
                )
                && !self.cooling_down(index)
        });
        let bindings = &self.data.project.bindings;
        let show = appearance.into_iter().flat_map(|first| {
            bindings
                .iter()
                .enumerate()
                .filter_map(move |(index, binding)| {
                    matches!(
                        (&binding.trigger, first),
                        (Trigger::FirstShown, true) | (Trigger::ShownAgain, false)
                    )
                    .then_some(index)
                })
        });
        let idle = self
            .idle_timers
            .iter()
            .filter(|timer| idle_due && self.now >= timer.due)
            .map(|timer| timer.binding);
        let winner = typing.or_else(|| {
            if can_start && self.display_source == AnimationSource::Idle {
                show.chain(idle)
                    .filter(|&index| !self.cooling_down(index))
                    .max_by_key(|&index| self.priority(index, false))
            } else {
                None
            }
        });
        if idle_due {
            for timer in &mut self.idle_timers {
                if self.now >= timer.due {
                    timer.due = self.now.saturating_add(timer.interval);
                }
            }
        }
        if timer.is_some_and(|index| self.try_start(index, false, self.now)) {
            self.pending_timer = None;
        } else if let Some(index) = winner {
            if self.try_start(index, false, self.now) {
                if typing == Some(index) {
                    if let Some(typing) = &mut self.typing_stop {
                        typing.last_reaction = Some(self.now);
                    }
                }
                let ends_at = self.one_shot.as_ref().unwrap().ends_at;
                if let Some(timer) = self
                    .idle_timers
                    .iter_mut()
                    .find(|timer| timer.binding == index)
                {
                    timer.due = ends_at.saturating_add(timer.interval);
                }
            }
        }
    }
    fn cooling_down(&self, index: usize) -> bool {
        let Effect::PlayAnimation { cooldown_ms, .. } = self.data.project.bindings[index].effect
        else {
            return false;
        };
        self.last_start[index]
            .is_some_and(|start| self.now.saturating_sub(start) < u64::from(cooldown_ms))
    }
    fn movement_available(&self) -> bool {
        if self.moving {
            return true;
        }
        let velocity = self.movement_vector();
        if velocity == Point::default() {
            return self.follow_motion().is_some();
        }
        // Held movement takes precedence over following, even at an edge.
        self.bounds.clamp(
            self.position.x + velocity.x * 0.016,
            self.position.y + velocity.y * 0.016,
            self.size,
        ) != self.position
    }
    fn follow_motion(&self) -> Option<FollowMotion> {
        let config = &self.follow_config;
        let (enabled, cursor, center) = self.native_follow.unwrap_or((
            config.enabled,
            self.cursor.filter(|_| self.controls.mouse),
            Point { x: 0.5, y: 0.5 },
        ));
        if !enabled || !self.controls.active || self.dragging {
            return None;
        }
        let cursor = cursor?;
        let destination = Point {
            x: cursor.x - center.x * self.size.x,
            y: cursor.y - center.y * self.size.y,
        };
        if self.bounds.clamp(destination.x, destination.y, self.size) == self.position {
            return None;
        }
        let dx = destination.x - self.position.x;
        let dy = destination.y - self.position.y;
        let distance = dx.hypot(dy);
        let remaining = distance - config.stop_distance_dip;
        if remaining <= 1e-6 {
            return None;
        }
        let band = config
            .bands
            .iter()
            .rposition(|band| distance >= band.min_distance_dip)?;
        Some(FollowMotion {
            direction: Point {
                x: dx / distance,
                y: dy / distance,
            },
            remaining,
            band,
            speed: config.bands[band].speed_dip_per_second,
        })
    }
    fn movement_vector(&self) -> Point {
        if !self.controls.active || !self.controls.keyboard || !self.controls.movement_enabled {
            return Point::default();
        }
        let (mut x, mut y, mut speed): (f64, f64, f64) = (0.0, 0.0, 0.0);
        let active_modifiers = self.active_modifiers();
        for binding in &self.data.project.bindings {
            if let (
                Trigger::WhileKeyDown { .. },
                Effect::Move {
                    x: dx,
                    y: dy,
                    speed_dip_per_second,
                    ..
                },
            ) = (&binding.trigger, &binding.effect)
            {
                if self
                    .held_key_rank(&binding.trigger, active_modifiers, true)
                    .is_some()
                {
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
    fn active_modifiers(&self) -> u8 {
        self.held
            .iter()
            .chain(&self.blocked)
            .filter_map(|code| KeyModifier::from_code(code))
            .fold(0, |mask, modifier| mask | modifier.bit())
    }
    fn held_key_rank(&self, trigger: &Trigger, active: u8, movement: bool) -> Option<bool> {
        trigger
            .key_conditions()
            .filter_map(|(code, modifiers)| {
                (self.held.contains(code)
                    && (!movement || !self.move_blocked.contains(code))
                    && modifiers_match(modifiers, code, active))
                .then_some(modifiers.is_some())
            })
            .max()
    }
    fn priority(&self, index: usize, exact_keys: bool) -> (i16, bool, Reverse<usize>) {
        let binding = &self.data.project.bindings[index];
        // Unconfigured continuous reactions retain their previous precedence.
        let priority = match binding.effect {
            Effect::PlayAnimation { priority, .. } => i16::from(priority),
            Effect::SustainAnimation { priority, .. } => priority.map_or_else(
                || {
                    if matches!(binding.trigger, Trigger::WhileKeyDown { .. }) {
                        -1
                    } else {
                        -2
                    }
                },
                i16::from,
            ),
            Effect::Move {
                priority,
                ref animation,
                ..
            } => priority.map_or(if animation.is_some() { -3 } else { -4 }, i16::from),
            Effect::FaceCursor { .. } => -5,
        };
        (priority, exact_keys, Reverse(index))
    }
    fn can_interrupt(&self, incoming: usize, exact_keys: bool, current: &OneShot) -> bool {
        if self.data.project.bindings[current.binding]
            .trigger
            .is_automatic()
        {
            return true;
        }
        let Effect::PlayAnimation { interruption, .. } =
            self.data.project.bindings[current.binding].effect
        else {
            return true;
        };
        match interruption.unwrap_or_default() {
            Interruption::Finish => false,
            Interruption::Replace => true,
            Interruption::ByPriority => {
                incoming == current.binding
                    || self.priority(incoming, exact_keys)
                        >= self.priority(current.binding, current.exact_keys)
            }
        }
    }
    fn try_start(&mut self, index: usize, exact_keys: bool, received_at: u64) -> bool {
        let incoming = self.priority(index, exact_keys);
        let allowed = if let Some(current) = &self.one_shot {
            self.can_interrupt(index, exact_keys, current)
        } else {
            self.continuous_display()
                .is_none_or(|(active, _, exact)| incoming >= self.priority(active, exact))
        };
        let follow_wins = self
            .follow_config
            .priority
            .is_some_and(|priority| i16::from(priority) > incoming.0)
            && self.display_follow_motion().is_some();
        if !allowed || follow_wins {
            self.performance.priority_rejected();
            return false;
        }
        if let Effect::PlayAnimation {
            animation,
            variation,
            duration_ms,
            ..
        } = &self.data.project.bindings[index].effect
        {
            let animation = if let Some(variation) = variation {
                let order = self.motion_orders[index]
                    .as_mut()
                    .expect("validated motion order");
                let choice = order.take(variation);
                if choice == 0 {
                    animation
                } else {
                    &variation.alternatives[choice - 1]
                }
            } else {
                animation
            };
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
                animation: animation.clone(),
                ends_at: self.now.saturating_add(duration),
                exact_keys,
            });
            self.last_start[index] = Some(self.now);
            self.sync_display(true);
            self.performance
                .start(received_at, self.epoch, self.action_instance, fps);
            return true;
        }
        false
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
    fn reset_matchers(&mut self) {
        for (_, matcher) in &mut self.matchers {
            matcher.reset();
        }
    }
    fn continuous_display(&self) -> Option<(usize, AnimationSource, bool)> {
        let active_modifiers = self.active_modifiers();
        self.data
            .project
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                let (source, exact_keys) = match (&binding.trigger, &binding.effect) {
                    (Trigger::WhileKeyDown { .. }, Effect::SustainAnimation { .. })
                        if self.controls.active && self.controls.keyboard =>
                    {
                        (
                            AnimationSource::HeldKey,
                            self.held_key_rank(&binding.trigger, active_modifiers, false)?,
                        )
                    }
                    (Trigger::TypingRate { .. }, Effect::SustainAnimation { .. })
                        if self.rate_band == Some(index) =>
                    {
                        (AnimationSource::TypingRate, false)
                    }
                    (Trigger::WhileKeyDown { .. }, Effect::Move { .. }) if self.moving => (
                        AnimationSource::Movement,
                        self.held_key_rank(&binding.trigger, active_modifiers, true)?,
                    ),
                    _ => return None,
                };
                Some((index, source, exact_keys))
            })
            .max_by_key(|&(index, _, exact_keys)| self.priority(index, exact_keys))
    }

    fn cursor_direction(&self, cursor: Point) -> CursorDirection {
        use CursorDirection::*;
        let axis = |point: f64, start: f64, size: f64| {
            if point < start + size / 3.0 {
                0
            } else if point > start + 2.0 * size / 3.0 {
                2
            } else {
                1
            }
        };
        let column = axis(cursor.x, self.position.x, self.size.x);
        let row = axis(cursor.y, self.position.y, self.size.y);
        [
            [UpLeft, Up, UpRight],
            [Left, Center, Right],
            [DownLeft, Down, DownRight],
        ][row][column]
    }

    fn display_follow_motion(&self) -> Option<FollowMotion> {
        (self.movement_vector() == Point::default())
            .then(|| self.follow_motion())
            .flatten()
    }

    fn sync_display(&mut self, force: bool) {
        let follow = self.display_follow_motion();
        let follow_priority = follow.as_ref().and(self.follow_config.priority);
        let continuous = self.continuous_display().filter(|&(index, _, exact)| {
            follow_priority
                .is_none_or(|priority| self.priority(index, exact).0 >= i16::from(priority))
        });
        let continuous_binding = continuous.map(|(index, _, _)| index);
        let newly_active =
            continuous.filter(|(index, _, _)| Some(*index) != self.continuous_binding);
        self.continuous_binding = continuous_binding;
        // A held key or rate band can interrupt on activation, not on every tick.
        // A continuous condition that arrived with a one-shot is resolved at its start.
        if self.one_shot.as_ref().is_some_and(|shot| {
            let automatic = self.data.project.bindings[shot.binding]
                .trigger
                .is_automatic();
            (automatic && (!self.visible || self.dragging || self.movement_available()))
                || (!force
                    && newly_active.is_some_and(|(incoming, _, exact_keys)| {
                        self.can_interrupt(incoming, exact_keys, shot)
                    }))
        }) {
            self.performance.end(
                self.now,
                self.epoch,
                self.action_instance,
                ReactionEnd::Interrupted,
            );
            self.one_shot = None;
            self.shot_release = None;
        }
        let (binding, source) = if self.dragging {
            let drag = self
                .data
                .project
                .bindings
                .iter()
                .position(|binding| matches!(binding.trigger, Trigger::Drag));
            (drag, AnimationSource::Drag)
        } else if let Some(shot) = &self.one_shot {
            (Some(shot.binding), AnimationSource::OneShot)
        } else if let Some((index, source, _)) =
            continuous.filter(|_| follow.is_none() || follow_priority.is_some())
        {
            (Some(index), source)
        } else if follow.is_some() {
            (None, AnimationSource::CursorFollow)
        } else {
            (None, AnimationSource::Idle)
        };
        let idle_group = self
            .idle_group
            .filter(|_| source == AnimationSource::Idle)
            .map(|index| &self.data.project.idle_groups[index]);
        let mut animation = binding
            .and_then(|index| match &self.data.project.bindings[index].effect {
                Effect::PlayAnimation { .. } => {
                    self.one_shot.as_ref().map(|shot| shot.animation.as_str())
                }
                Effect::SustainAnimation { animation, .. } => Some(animation.as_str()),
                Effect::Move { animation, .. } => animation.as_deref(),
                Effect::FaceCursor { .. } => None,
            })
            .or_else(|| {
                if source == AnimationSource::CursorFollow {
                    follow.as_ref().and_then(|follow| {
                        self.follow_config.bands[follow.band].animation.as_deref()
                    })
                } else {
                    None
                }
            })
            .or_else(|| idle_group.and_then(|group| group.animation.as_deref()))
            .unwrap_or(&self.data.character.default_animation);
        let mut target_left = self.movement_facing_left;
        let mut mirror_override = None;
        if source == AnimationSource::CursorFollow {
            let x = follow.unwrap().direction.x;
            if x != 0.0 {
                target_left = Some(x < 0.0);
            }
        }
        let cursor = if self.dragging {
            self.drag_cursor
        } else {
            self.cursor
                .filter(|_| self.controls.active && self.controls.mouse)
        };
        if let Some(cursor) = cursor {
            if let Some(index) = binding {
                if self.data.project.bindings[index].follow_cursor == Some(true) {
                    target_left = Some(cursor.x < self.position.x + self.size.x / 2.0);
                }
            } else if source == AnimationSource::Idle {
                let directions = match idle_group.map(|group| group.cursor_mode) {
                    Some(IdleCursorMode::Off) => None,
                    Some(IdleCursorMode::Custom) => idle_group.map(|group| Some(&group.directions)),
                    _ => match &self.data.project.idle {
                        Some(idle) => idle.cursor_enabled.then_some(Some(&idle.directions)),
                        None => {
                            self.data.project.bindings.iter().find_map(|binding| {
                                match &binding.effect {
                                    Effect::FaceCursor { directions } => Some(directions.as_ref()),
                                    _ => None,
                                }
                            })
                        }
                    },
                };
                if let Some(directions) = directions {
                    let direction = self.cursor_direction(cursor);
                    let pose = directions.and_then(|poses| poses.get(&direction));
                    animation = pose
                        .and_then(|pose| pose.animation.as_deref())
                        .unwrap_or(animation);
                    mirror_override = pose.and_then(|pose| pose.mirror);
                    target_left = Some(matches!(
                        direction,
                        CursorDirection::UpLeft | CursorDirection::Left | CursorDirection::DownLeft
                    ));
                }
            }
        }
        let source_left =
            self.data.character.animations[animation].source_facing == Some(SourceFacing::Left);
        self.facing_left =
            mirror_override.unwrap_or(target_left.is_some_and(|left| left != source_left));
        if force
            || self.display != animation
            || self.display_source != source
            || self.display_binding != binding
        {
            self.display = animation.into();
            self.display_source = source;
            self.display_binding = binding;
            self.animation_started_at = self.now;
            self.action_instance = self.action_instance.wrapping_add(1);
        }
        if !self.idle_timers.is_empty() {
            // Direction-specific idle clips and fidgets belong to the same
            // idle stretch. A real action or movement starts a fresh stretch.
            let idle = self.controls.active
                && self.visible
                && !self.movement_available()
                && (source == AnimationSource::Idle
                    || self.one_shot.as_ref().is_some_and(|shot| {
                        matches!(
                            self.data.project.bindings[shot.binding].trigger,
                            Trigger::IdleFor { .. }
                        )
                    }));
            if idle && !self.timing_idle {
                for timer in &mut self.idle_timers {
                    timer.due = self.now.saturating_add(timer.interval);
                }
            }
            self.timing_idle = idle;
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
            sound: self
                .display_binding
                .and_then(|index| self.data.project.bindings[index].sound.clone()),
            facing_left: self.facing_left,
            position: self.position,
            typing_band_id: self
                .rate_band
                .map(|index| self.data.project.bindings[index].id.clone()),
        }
    }
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let movement = (self.movement_vector() != Point::default()
            || self.follow_motion().is_some())
        .then_some(self.now + 16);
        [
            movement,
            self.rate_next,
            self.typing_stop.as_ref().and_then(TypingStop::deadline),
            self.pending_timer.map(|(_, expires)| expires),
            self.one_shot.as_ref().map(|shot| shot.ends_at),
            self.performance.next_deadline(),
            self.timing_idle
                .then(|| self.idle_timers.iter().map(|timer| timer.due).min())
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

struct FollowMotion {
    direction: Point,
    remaining: f64,
    band: usize,
    speed: f64,
}

fn key_matches(pattern: &str, input: &str) -> bool {
    pattern == input || (pattern == "*" && is_supported_key(input) && !is_modifier(input))
}

fn is_typing_key(code: &str) -> bool {
    // The caller already checked that this is a supported physical key.
    code.starts_with("Key")
        || code.starts_with("Digit")
        || matches!(
            code,
            "Space"
                | "Backquote"
                | "Minus"
                | "Equal"
                | "BracketLeft"
                | "BracketRight"
                | "Backslash"
                | "Semicolon"
                | "Quote"
                | "Comma"
                | "Period"
                | "Slash"
        )
}

fn key_match_rank(trigger: &Trigger, input: &str, active: u8) -> Option<bool> {
    trigger
        .key_conditions()
        .filter_map(|(code, modifiers)| {
            (key_matches(code, input) && modifiers_match(modifiers, input, active))
                .then_some(modifiers.is_some())
        })
        .max()
}

fn modifiers_match(required: Option<&[KeyModifier]>, code: &str, active: u8) -> bool {
    required.is_none_or(|modifiers| {
        let expected = modifiers
            .iter()
            .fold(0, |mask, modifier| mask | modifier.bit());
        let own_modifier = KeyModifier::from_code(code).map_or(0, KeyModifier::bit);
        expected == active & !own_modifier
    })
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
