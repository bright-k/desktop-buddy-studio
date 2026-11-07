use buddy_project::*;
use buddy_runtime::*;
use serde_json::{json, Value};

fn data(bindings: Value) -> ValidatedProject {
    let mut project: Project = parse_json(
        include_bytes!("../../../examples/typing-cat/buddy.project.json"),
        "project",
    )
    .unwrap();
    project.defaults.movement_enabled = true;
    project.bindings = serde_json::from_value(bindings).unwrap();
    let character = parse_json(
        include_bytes!("../../../examples/typing-cat/character/manifest.json"),
        "manifest",
    )
    .unwrap();
    validate_parts(project, character).unwrap()
}
fn runtime(bindings: Value) -> Runtime {
    Runtime::new(
        data(bindings),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        },
        Point { x: 100.0, y: 100.0 },
        Point { x: 100.0, y: 100.0 },
    )
}
fn shot(id: &str, key: &str, animation: &str, priority: u8) -> Value {
    json!({"id":id,"trigger":{"type":"keyDown","code":key},"effect":{"type":"playAnimation","animation":animation,"priority":priority}})
}
fn movement(key: &str, x: f64, y: f64) -> Value {
    json!({"id":key,"trigger":{"type":"whileKeyDown","code":key},"effect":{"type":"move","x":x,"y":y,"speedDipPerSecond":100,"animation":"walk"}})
}
fn down(runtime: &mut Runtime, now: u64, code: &str) {
    runtime.handle(
        now,
        InputEvent::KeyDown {
            code: code.into(),
            repeat: false,
        },
    );
}
fn up(runtime: &mut Runtime, now: u64, code: &str) {
    runtime.handle(now, InputEvent::KeyUp { code: code.into() });
}
fn tap(runtime: &mut Runtime, now: u64, code: &str) {
    down(runtime, now, code);
    up(runtime, now, code);
}
fn pattern(word: &str, timeout: u32) -> Value {
    json!({"id":"secret","trigger":{"type":"keySequence","keyword":word,"layout":"ko-KR-2set","timeoutMs":timeout},"effect":{"type":"playAnimation","animation":"celebrate","priority":95}})
}

#[test]
fn idle_has_no_rust_tick_deadline() {
    let runtime = runtime(json!([]));
    assert_eq!(runtime.state().animation, "idle");
    assert_eq!(runtime.next_deadline_ms(), None);
}

#[test]
fn frame_latency_includes_native_queue_delay_and_keyword_uses_final_key() {
    let mut runtime = runtime(json!([pattern("고양이", 5000)]));
    let codes = ["KeyR", "KeyH", "KeyD", "KeyI", "KeyD", "KeyD", "KeyL"];
    for (i, code) in codes.iter().enumerate() {
        let received = (i as u64) * 20;
        runtime.handle_received(
            received + 70,
            received,
            InputEvent::KeyDown {
                code: (*code).into(),
                repeat: false,
            },
        );
        up(&mut runtime, received + 71, code);
    }
    let state = runtime.state();
    assert_eq!(state.animation, "celebrate");
    runtime.prepare_frame(1, &state, true);
    let ack = FrameAck {
        seq: 1,
        epoch: state.epoch,
        action_instance_id: state.action_instance_id,
        outcome: FrameOutcome::Drawn,
    };
    assert!(runtime.ack_frame(220, &ack, true));
    let stats = runtime.performance(220);
    assert_eq!(stats.candidates, 1);
    assert_eq!(stats.latency_other_fps.p95_ms, Some(100));
    assert_eq!(stats.latency_30_fps.samples, 0);
}

#[test]
fn performance_accounts_for_all_matched_candidates_without_changing_arbitration() {
    let mut first = shot("first", "KeyA", "click", 50);
    first["effect"]["retrigger"] = json!("ignore");
    let mut wildcard = shot("wildcard", "*", "jump", 20);
    wildcard["effect"]["cooldownMs"] = json!(1000);
    let mut runtime = runtime(json!([first, wildcard]));
    tap(&mut runtime, 0, "KeyB"); // wildcard starts, then first interrupts.
    tap(&mut runtime, 10, "KeyA"); // wildcard ignored by cooldown.
    tap(&mut runtime, 20, "KeyA"); // same reaction ignored by retrigger.
    tap(&mut runtime, 1020, "KeyA"); // both eligible, one wins.
    tap(&mut runtime, 1030, "KeyC"); // wildcard loses to current priority.
    let stats = runtime.performance(1040);
    assert_eq!(stats.candidates, 8);
    assert_eq!(stats.ignored_cooldown, 2);
    assert_eq!(stats.ignored_retrigger, 1);
    assert_eq!(stats.suppressed_same_input, 1);
    assert_eq!(stats.ignored_priority, 1);
    assert_eq!(stats.reactions_started, 3);
    assert_eq!(
        stats.candidates,
        stats.ignored_cooldown
            + stats.ignored_retrigger
            + stats.suppressed_same_input
            + stats.ignored_priority
            + stats.reactions_started
    );
    assert_eq!(stats.reactions_interrupted, 1);
    assert_eq!(stats.ended_before_snapshot, 2);
}
#[test]
fn rust_ends_one_shot_without_renderer_ack() {
    let mut runtime = runtime(json!([shot("jump", "Space", "jump", 80)]));
    down(&mut runtime, 0, "Space");
    assert_eq!(runtime.state().animation, "jump");
    runtime.advance(333);
    assert_eq!(runtime.state().animation, "jump");
    runtime.advance(334);
    assert_eq!(runtime.state().animation, "idle");
    assert_eq!(runtime.state().animation_elapsed_ms, 0.0);
}
#[test]
fn higher_priority_interrupts_and_interrupted_shot_never_resumes() {
    let mut runtime = runtime(json!([
        shot("click", "KeyA", "click", 30),
        shot("jump", "Space", "jump", 80)
    ]));
    down(&mut runtime, 0, "KeyA");
    down(&mut runtime, 50, "Space");
    assert_eq!(runtime.state().animation, "jump");
    runtime.advance(384);
    assert_eq!(runtime.state().animation, "idle");
}
#[test]
fn same_priority_uses_definition_order_against_current_shot() {
    let mut runtime = runtime(json!([
        shot("first", "KeyB", "click", 50),
        shot("second", "KeyA", "jump", 50)
    ]));
    down(&mut runtime, 0, "KeyB");
    down(&mut runtime, 10, "KeyA");
    assert_eq!(runtime.state().animation, "click");
    runtime.advance(200);
    down(&mut runtime, 210, "KeyA");
    down(&mut runtime, 220, "KeyB");
    assert_eq!(runtime.state().animation, "click");
}
#[test]
fn rejected_input_does_not_consume_cooldown() {
    let mut rejected = shot("low", "KeyA", "click", 30);
    rejected["effect"]["cooldownMs"] = 10000.into();
    let mut runtime = runtime(json!([rejected, shot("jump", "Space", "jump", 80)]));
    down(&mut runtime, 0, "Space");
    down(&mut runtime, 10, "KeyA");
    down(&mut runtime, 350, "KeyA");
    assert_eq!(runtime.state().animation, "click");
}
#[test]
fn restart_changes_instance_and_ignore_does_not() {
    for (retrigger, changed) in [("restart", true), ("ignore", false)] {
        let mut definition = shot("jump", "Space", "jump", 80);
        definition["effect"]["retrigger"] = retrigger.into();
        let mut runtime = runtime(json!([definition]));
        down(&mut runtime, 0, "Space");
        let instance = runtime.state().action_instance_id;
        down(&mut runtime, 100, "Space");
        assert_eq!(instance != runtime.state().action_instance_id, changed);
    }
}
#[test]
fn repeat_needs_opt_in_and_cannot_seed_held_after_reset() {
    let mut runtime = runtime(json!([shot("jump", "Space", "jump", 80)]));
    down(&mut runtime, 0, "Space");
    let instance = runtime.state().action_instance_id;
    runtime.handle(
        20,
        InputEvent::KeyDown {
            code: "Space".into(),
            repeat: true,
        },
    );
    assert_eq!(instance, runtime.state().action_instance_id);
    runtime.reset(30);
    runtime.handle(
        40,
        InputEvent::KeyDown {
            code: "Space".into(),
            repeat: true,
        },
    );
    assert_eq!(runtime.state().animation, "idle");
    up(&mut runtime, 50, "Space");
    down(&mut runtime, 60, "Space");
    assert_eq!(runtime.state().animation, "jump");
}
#[test]
fn diagonal_is_normalized_and_opposite_keys_cancel() {
    let mut runtime = runtime(json!([
        movement("ArrowRight", 1.0, 0.0),
        movement("ArrowDown", 0.0, 1.0),
        movement("ArrowLeft", -1.0, 0.0)
    ]));
    down(&mut runtime, 0, "ArrowRight");
    down(&mut runtime, 0, "ArrowDown");
    runtime.advance(50);
    let position = runtime.state().position;
    assert!(((position.x - 100.0).hypot(position.y - 100.0) - 5.0).abs() < 1e-8);
    down(&mut runtime, 50, "ArrowLeft");
    up(&mut runtime, 50, "ArrowDown");
    let old = runtime.state().position;
    runtime.advance(100);
    assert_eq!(runtime.state().position, old);
    assert_eq!(runtime.state().animation, "idle");
}
#[test]
fn long_delta_does_not_teleport_and_bounds_stop_walk() {
    let mut runtime = runtime(json!([movement("ArrowRight", 1.0, 0.0)]));
    down(&mut runtime, 0, "ArrowRight");
    runtime.advance(10000);
    assert_eq!(runtime.state().position.x, 105.0);
    runtime.place(
        10000,
        Point { x: 900.0, y: 100.0 },
        Point { x: 100.0, y: 100.0 },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        },
    );
    runtime.advance(10050);
    assert_eq!(runtime.state().position.x, 900.0);
    assert_eq!(runtime.state().animation, "idle");
}
#[test]
fn movement_continues_during_reaction_and_returns_to_walk() {
    let mut runtime = runtime(json!([
        movement("ArrowRight", 1.0, 0.0),
        shot("jump", "Space", "jump", 80)
    ]));
    down(&mut runtime, 0, "ArrowRight");
    runtime.advance(20);
    assert_eq!(runtime.state().animation, "walk");
    down(&mut runtime, 20, "Space");
    runtime.advance(70);
    assert_eq!(runtime.state().animation, "jump");
    assert!(runtime.state().position.x > 102.0);
    runtime.advance(354);
    assert_eq!(runtime.state().animation, "walk");
}
#[test]
fn pause_clears_held_and_resume_requires_new_press() {
    let mut runtime = runtime(json!([movement("ArrowRight", 1.0, 0.0)]));
    down(&mut runtime, 0, "ArrowRight");
    let mut controls = runtime.controls();
    controls.active = false;
    runtime.set_controls(20, controls);
    let position = runtime.state().position;
    runtime.advance(500);
    assert_eq!(runtime.state().position, position);
    controls.active = true;
    runtime.set_controls(500, controls);
    runtime.handle(
        510,
        InputEvent::KeyDown {
            code: "ArrowRight".into(),
            repeat: true,
        },
    );
    runtime.advance(550);
    assert_eq!(runtime.state().position, position);
    up(&mut runtime, 560, "ArrowRight");
    down(&mut runtime, 570, "ArrowRight");
    runtime.advance(600);
    assert!(runtime.state().position.x > position.x);
}
#[test]
fn typing_rate_stabilizes_and_returns_idle_at_the_configured_short_timeout() {
    let mut runtime = runtime(json!([
        {"id":"slow","trigger":{"type":"typingRate","minKps":0.5,"maxKps":3.0},"effect":{"type":"sustainAnimation","animation":"typing"}},
        {"id":"fast","trigger":{"type":"typingRate","minKps":3.0},"effect":{"type":"sustainAnimation","animation":"typingFast"}}
    ]));
    tap(&mut runtime, 0, "KeyA");
    runtime.advance(99);
    assert_eq!(runtime.state().animation, "idle");
    runtime.advance(100);
    assert_eq!(runtime.state().animation, "typing");
    for time in [110, 120, 130, 140, 150] {
        tap(&mut runtime, time, "KeyA");
    }
    runtime.advance(200);
    runtime.advance(299);
    assert_eq!(runtime.state().animation, "typing");
    runtime.advance(300);
    assert_eq!(runtime.state().animation, "typingFast");
    runtime.advance(399);
    assert_eq!(runtime.state().animation, "typingFast");
    runtime.advance(400);
    assert_eq!(runtime.state().animation, "idle");
    assert_eq!(runtime.next_deadline_ms(), None);
}
#[test]
fn modifiers_and_os_repeat_do_not_increase_rate() {
    let mut runtime = runtime(
        json!([{"id":"typing","trigger":{"type":"typingRate","minKps":1.0},"effect":{"type":"sustainAnimation","animation":"typing"}}]),
    );
    down(&mut runtime, 0, "ShiftLeft");
    down(&mut runtime, 0, "KeyA");
    for time in [10, 20, 30, 40] {
        runtime.handle(
            time,
            InputEvent::KeyDown {
                code: "KeyA".into(),
                repeat: true,
            },
        );
    }
    runtime.advance(300);
    assert_eq!(runtime.state().animation, "idle");
}
#[test]
fn korean_sequence_and_specific_final_key_compete_once() {
    let mut runtime = runtime(json!([
        shot("letter", "KeyL", "click", 30),
        pattern("고양이", 5000),
        shot("wildcard", "*", "jump", 20)
    ]));
    for (index, letter) in "rhdiddl".chars().enumerate() {
        tap(
            &mut runtime,
            index as u64 * 20,
            &format!("Key{}", letter.to_ascii_uppercase()),
        );
    }
    assert_eq!(runtime.state().animation, "celebrate");
}
#[test]
fn sequence_shift_timeout_editing_and_shortcuts_are_enforced() {
    for reset in ["Backspace", "ControlLeft", "Unknown"] {
        let mut runtime = runtime(json!([pattern("고", 500)]));
        tap(&mut runtime, 0, "KeyR");
        tap(&mut runtime, 10, reset);
        tap(&mut runtime, 20, "KeyH");
        assert_ne!(runtime.state().animation, "celebrate");
    }
    let mut runtime = runtime(json!([pattern("고", 500)]));
    tap(&mut runtime, 0, "KeyR");
    tap(&mut runtime, 501, "KeyH");
    assert_ne!(runtime.state().animation, "celebrate");
    tap(&mut runtime, 600, "KeyR");
    down(&mut runtime, 610, "ShiftLeft");
    tap(&mut runtime, 620, "KeyH");
    assert_ne!(runtime.state().animation, "celebrate");
}
#[test]
fn shifted_korean_and_overlapping_patterns_work() {
    let mut runtime = runtime(json!([pattern("까", 5000)]));
    down(&mut runtime, 0, "ShiftRight");
    tap(&mut runtime, 1, "KeyR");
    up(&mut runtime, 2, "ShiftRight");
    tap(&mut runtime, 3, "KeyK");
    assert_eq!(runtime.state().animation, "celebrate");
    let mut runtime = Runtime::new(
        data(json!([pattern("aba", 5000)])),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        },
        Point::default(),
        Point { x: 100.0, y: 100.0 },
    );
    for (i, key) in ["KeyA", "KeyB", "KeyA"].iter().enumerate() {
        tap(&mut runtime, i as u64 * 10, key);
    }
    let instance = runtime.state().action_instance_id;
    tap(&mut runtime, 30, "KeyB");
    tap(&mut runtime, 40, "KeyA");
    assert!(runtime.state().action_instance_id > instance);
}
#[test]
fn reset_clears_sequence_and_rate_without_exposing_input_in_state() {
    let mut runtime = runtime(json!([pattern("고", 5000)]));
    tap(&mut runtime, 0, "KeyR");
    runtime.reset(10);
    tap(&mut runtime, 20, "KeyH");
    assert_eq!(runtime.state().animation, "idle");
    let state = serde_json::to_value(runtime.state()).unwrap();
    assert!(
        state.get("held").is_none()
            && state.get("keyword").is_none()
            && state.get("inputHistory").is_none()
    );
}

#[test]
fn native_rearm_suppresses_existing_presses_but_accepts_the_first_new_press() {
    let mut runtime = runtime(json!([
        movement("ArrowRight", 1.0, 0.0),
        shot("jump", "Space", "jump", 80)
    ]));
    down(&mut runtime, 0, "Space");
    let mut controls = runtime.controls();
    controls.active = false;
    runtime.set_controls(10, controls);
    // Space was released while the native adapter was stopped. ArrowRight is
    // still physically down at resume; neither history nor held is restored.
    controls.active = true;
    runtime.set_controls(20, controls);
    runtime.handle(
        20,
        InputEvent::Rearm {
            keys: vec!["ArrowRight".into()],
            buttons: vec![],
        },
    );
    down(&mut runtime, 21, "ArrowRight");
    runtime.advance(70);
    assert_eq!(runtime.state().position.x, 100.0);
    down(&mut runtime, 71, "Space");
    assert_eq!(runtime.state().animation, "jump");
    up(&mut runtime, 72, "ArrowRight");
    down(&mut runtime, 73, "ArrowRight");
    runtime.advance(100);
    assert!(runtime.state().position.x > 100.0);
}

#[test]
fn keyboard_suspension_keeps_mouse_reactions_and_rearm_suppresses_old_button_release() {
    let mut runtime = runtime(json!([
        {"id":"click","trigger":{"type":"mouseDown","button":"primary"},"effect":{"type":"playAnimation","animation":"click"}},
        {"id":"release","trigger":{"type":"mouseUp","button":"primary"},"effect":{"type":"playAnimation","animation":"jump"}}
    ]));
    let mut controls = runtime.controls();
    controls.keyboard = false;
    runtime.set_controls(0, controls);
    runtime.handle(
        0,
        InputEvent::Rearm {
            keys: vec![],
            buttons: vec![buddy_project::MouseButton::Primary],
        },
    );
    runtime.handle(
        1,
        InputEvent::MouseUp {
            button: buddy_project::MouseButton::Primary,
        },
    );
    assert_eq!(runtime.state().animation, "idle");
    runtime.handle(
        2,
        InputEvent::MouseDown {
            button: buddy_project::MouseButton::Primary,
        },
    );
    assert_eq!(runtime.state().animation, "click");
}

#[test]
fn modifiers_held_at_capture_start_keep_their_sequence_meaning_without_triggering_actions() {
    let mut shifted = runtime(json!([pattern("까", 5000)]));
    shifted.handle(
        0,
        InputEvent::Rearm {
            keys: vec!["ShiftLeft".into()],
            buttons: vec![],
        },
    );
    tap(&mut shifted, 1, "KeyR");
    up(&mut shifted, 2, "ShiftLeft");
    tap(&mut shifted, 3, "KeyK");
    assert_eq!(shifted.state().animation, "celebrate");
    let mut shortcut = runtime(json!([pattern("고", 5000)]));
    shortcut.handle(
        0,
        InputEvent::Rearm {
            keys: vec!["ControlLeft".into()],
            buttons: vec![],
        },
    );
    tap(&mut shortcut, 1, "KeyR");
    tap(&mut shortcut, 2, "KeyH");
    assert_eq!(shortcut.state().animation, "idle");
    up(&mut shortcut, 3, "ControlLeft");
    tap(&mut shortcut, 4, "KeyR");
    tap(&mut shortcut, 5, "KeyH");
    assert_eq!(shortcut.state().animation, "celebrate");
}

#[test]
fn typing_aggregation_is_bounded_even_without_rate_bindings_and_reports_only_counters() {
    let mut runtime = runtime(json!([]));
    for _ in 0..2001 {
        tap(&mut runtime, 0, "KeyA");
    }
    assert_eq!(runtime.diagnostics().rate_overflow_resets, 1);
    runtime.reset(1);
    let diagnostics = serde_json::to_value(runtime.diagnostics()).unwrap();
    assert_eq!(diagnostics["epochResets"], 1);
    assert_eq!(diagnostics.as_object().unwrap().len(), 2);
    runtime.advance(2000);
    assert_eq!(runtime.next_deadline_ms(), None);
}

#[test]
fn resource_duration_override_release_and_drag_take_effect_without_waiting_for_clip_end() {
    let mut binding = shot("press", "KeyA", "jump", 80);
    binding["effect"]["durationMs"] = 3000.into();
    binding["effect"]["stopOnRelease"] = true.into();
    let mut runtime = runtime(json!([binding,
        {"id":"held","trigger":{"type":"whileKeyDown","code":"KeyB"},"effect":{"type":"sustainAnimation","animation":"typing"}},
        {"id":"drag","trigger":{"type":"drag"},"effect":{"type":"sustainAnimation","animation":"walk"}}
    ]));
    down(&mut runtime, 0, "KeyA");
    runtime.advance(1000);
    assert_eq!(runtime.state().animation, "jump");
    up(&mut runtime, 1001, "KeyA");
    assert_eq!(runtime.state().animation, "idle");
    down(&mut runtime, 1100, "KeyB");
    assert_eq!(runtime.state().animation, "typing");
    runtime.handle(1200, InputEvent::DragStart);
    assert_eq!(runtime.state().animation, "walk");
    runtime.handle(1300, InputEvent::DragEnd);
    assert_eq!(runtime.state().animation, "typing");
    up(&mut runtime, 1301, "KeyB");
    assert_eq!(runtime.state().animation, "idle");
}
#[test]
fn gif_prepared_durations_control_one_shot_completion() {
    let mut data = data(json!([shot("gif", "KeyA", "jump", 50)]));
    let clip = data.render_plan.animations.get_mut("jump").unwrap();
    clip.frames.truncate(2);
    clip.frames[0].duration_ms = 80.0;
    clip.frames[1].duration_ms = 240.0;
    let mut runtime = Runtime::new(
        data,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        },
        Point::default(),
        Point { x: 100.0, y: 100.0 },
    );
    down(&mut runtime, 0, "KeyA");
    runtime.advance(319);
    assert_eq!(runtime.state().animation, "jump");
    runtime.advance(320);
    assert_eq!(runtime.state().animation, "idle");
}

#[test]
fn user_timing_override_recalculates_speed_and_idle_deadline_and_rejects_invalid_values() {
    let mut runtime = runtime(
        json!([{ "id":"rate", "trigger":{"type":"typingRate","minKps":1.0}, "effect":{"type":"sustainAnimation","animation":"typing"} }]),
    );
    let timing = Timing {
        rate_window_ms: 1000,
        rate_evaluation_ms: 50,
        rate_stability_ms: 0,
        idle_timeout_ms: 150,
    };
    runtime.set_timing(0, timing);
    tap(&mut runtime, 0, "KeyA");
    assert_eq!(runtime.state().animation, "typing");
    runtime.advance(149);
    assert_eq!(runtime.state().animation, "typing");
    runtime.advance(150);
    assert_eq!(runtime.state().animation, "idle");
    runtime.set_timing(
        200,
        Timing {
            idle_timeout_ms: 0,
            ..timing
        },
    );
    assert_eq!(runtime.project().project.defaults.timing, timing);
}
