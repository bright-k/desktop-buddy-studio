use buddy_platform::{InputOptions, InputSession};
use buddy_project::{Point, Project};
use buddy_runtime::{InputEvent, Rect, Runtime};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub fn doctor() -> Result<(), String> {
    let toolchain = buddy_builder::diagnose_toolchain(buddy_project::Target::current());
    println!("{}",serde_json::to_string_pretty(&json!({"engineVersion":buddy_project::ENGINE_VERSION,"templateHash":buddy_builder::template_hash(),"host":buddy_builder::host_environment(),"tools":toolchain.tools,"toolchainPolicy":{
        "rustToolchain":toolchain.rust_toolchain,"automaticInstallation":toolchain.automatic_installation,"target":toolchain.target,"targetAvailable":toolchain.target_available,"ready":toolchain.ready,"problems":toolchain.problems
    },"inputPermission":buddy_platform::permission_state(),"releaseValidation":"not-performed"})).unwrap());
    toolchain.require_ready()
}

pub fn input_probe(seconds: u64, request_permission: bool) -> Result<(), String> {
    if request_permission {
        buddy_platform::request_input_permission();
    }
    let mut session = InputSession::start(InputOptions::default())?;
    let status = session.status();
    if !status.any_running() {
        println!(
            "{}",
            json!({"status":status,"observedEvents":0,"storesRawInput":false})
        );
        return Err("Native input capture is unavailable; review the reported permission/status and retry explicitly".into());
    }
    let start = Instant::now();
    let mut counts = std::collections::BTreeMap::<&str, u64>::new();
    while start.elapsed() < Duration::from_secs(seconds) {
        while let Some(event) = session.poll() {
            let kind = match event {
                InputEvent::KeyDown { .. } => "keyDown",
                InputEvent::KeyUp { .. } => "keyUp",
                InputEvent::MouseDown { .. } => "mouseDown",
                InputEvent::MouseUp { .. } => "mouseUp",
                InputEvent::Wheel { .. } | InputEvent::DiagonalWheel { .. } => "wheel",
                InputEvent::CursorMove { .. } => "cursorMove",
                InputEvent::Reset => "reset",
                InputEvent::DragStart | InputEvent::DragEnd | InputEvent::DragDropped => "drag",
                InputEvent::BuddyClick => "buddyClick",
                InputEvent::BuddyLongPress => "buddyLongPress",
                InputEvent::OverlayVisibility { .. } => "overlayVisibility",
                InputEvent::Rearm { .. } => "rearm",
                InputEvent::TimerCompleted { .. } => "timerCompleted",
                InputEvent::TimerStarted { .. } => "timerStarted",
                InputEvent::TimerPhaseChanged { .. } => "timerPhaseChanged",
            };
            *counts.entry(kind).or_default() += 1;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "{}",
        json!({"status":session.status(),"durationMs":start.elapsed().as_millis(),"eventCounts":counts,"storesRawInput":false})
    );
    Ok(())
}

pub fn new_project(directory: &Path, name: Option<String>, template: &str) -> Result<(), String> {
    if directory.exists() {
        return Err("Destination already exists; choose a new project directory".into());
    }
    let mascot = template == "click-mascot";
    let empty = template == "empty";
    let source = if mascot {
        include_bytes!("../../../examples/click-mascot/buddy.project.json").as_slice()
    } else {
        include_bytes!("../../../examples/typing-cat/buddy.project.json").as_slice()
    };
    let mut project: Project =
        buddy_project::parse_json(source, "template").map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4();
    project.project_id = id.to_string();
    project.product.identifier = format!("io.desktopbuddy.app.p{}", id.simple());
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("my-buddy")
        .to_ascii_lowercase();
    if !(2..=64).contains(&slug.len())
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Project directory name must be a 2..64 character lowercase ASCII slug".into());
    }
    project.product.slug = slug;
    if let Some(name) = name {
        project.product.name = name;
    }
    let manifest = if mascot {
        include_bytes!("../../../examples/click-mascot/character/manifest.json").as_slice()
    } else {
        include_bytes!("../../../examples/typing-cat/character/manifest.json").as_slice()
    };
    let mut character: buddy_project::Character =
        buddy_project::parse_json(manifest, "template manifest").map_err(|e| e.to_string())?;
    let blank_manifest;
    let manifest = if empty {
        project.bindings.clear();
        character.id = "character".into();
        character.name = project.product.name.clone();
        character.image = None;
        character.default_animation.clear();
        character.animations.clear();
        character.sheet.width = character.sheet.frame_width;
        character.sheet.height = character.sheet.frame_height;
        character.sheet.frame_count = 1;
        blank_manifest = serde_json::to_vec_pretty(&character).map_err(|e| e.to_string())?;
        &blank_manifest
    } else {
        manifest
    };
    buddy_project::validate_editable_parts(project.clone(), character)
        .map_err(|e| e.to_string())?;
    let mut files: Vec<(&str, &[u8])> = vec![
        ("character/manifest.json", manifest),
        (
            "assets/app-icon.png",
            if mascot {
                include_bytes!("../../../examples/click-mascot/assets/app-icon.png")
            } else {
                include_bytes!("../../../examples/typing-cat/assets/app-icon.png")
            },
        ),
        (
            "character/LICENSE.txt",
            include_bytes!("../../../examples/typing-cat/character/LICENSE.txt"),
        ),
    ];
    if !empty {
        files.push((
            "character/sprite.png",
            if mascot {
                include_bytes!("../../../examples/click-mascot/character/sprite.png")
            } else {
                include_bytes!("../../../examples/typing-cat/character/sprite.png")
            },
        ));
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    for (name, bytes) in files {
        let path = directory.join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    fs::write(
        directory.join("buddy.project.json"),
        format!("{}\n", serde_json::to_string_pretty(&project).unwrap()),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "Created {} ({})",
        directory.display(),
        project.product.identifier
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimedEvent {
    at_ms: u64,
    event: InputEvent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MotionPreview {
    binding_id: String,
    seed: u32,
    starts: u32,
}

pub fn simulate(
    project: &Path,
    events: &Path,
    from_ms: u64,
    until_ms: Option<u64>,
    timeline: bool,
    motion_preview: Option<&str>,
    include_binding_ids: bool,
) -> Result<(), String> {
    let loaded = buddy_project::load_project(project).map_err(|e| e.to_string())?;
    let bytes = fs::read(events).map_err(|e| e.to_string())?;
    let events: Vec<TimedEvent> = buddy_project::parse_json(&bytes, &events.display().to_string())
        .map_err(|e| e.to_string())?;
    if !events.windows(2).all(|pair| pair[0].at_ms <= pair[1].at_ms) {
        return Err("Synthetic event times must be monotonic".into());
    }
    let last_event = events.last().map_or(0, |event| event.at_ms);
    let until = until_ms.unwrap_or(last_event.max(from_ms));
    if from_ms > 3_600_000 || until < last_event || !(from_ms..=from_ms + 60_000).contains(&until) {
        return Err("Simulation requires a start within one hour and a window of at most 60 seconds containing the final event".into());
    }
    let mut runtime = Runtime::new(
        loaded.data,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        },
        Point { x: 100.0, y: 100.0 },
        Point { x: 256.0, y: 256.0 },
    );
    if let Some(spec) = motion_preview {
        let preview: MotionPreview =
            serde_json::from_str(spec).map_err(|e| format!("Invalid motion preview: {e}"))?;
        if !runtime.prime_motion_variation(&preview.binding_id, preview.seed, preview.starts) {
            return Err(
                "Motion preview requires an existing variation and at most 10000 previous starts"
                    .into(),
            );
        }
    }
    let print = |at_ms: u64, runtime: &Runtime| {
        if include_binding_ids || motion_preview.is_some() {
            println!(
                "{}",
                json!({"atMs": at_ms - from_ms, "state": runtime.state(), "bindingId": runtime.displayed_binding_id()})
            );
        } else if timeline {
            println!(
                "{}",
                serde_json::json!({"atMs": at_ms - from_ms, "state": runtime.state()})
            );
        } else {
            println!("{}", serde_json::to_string(&runtime.state()).unwrap());
        }
    };
    let mut events = events.into_iter().peekable();
    let mut now = 0;
    loop {
        let had_event = events.peek().is_some_and(|event| event.at_ms == now);
        while events.peek().is_some_and(|event| event.at_ms == now) {
            let event = events.next().unwrap();
            runtime.handle(now, event.event);
            if !timeline && events.peek().is_some_and(|event| event.at_ms == now) {
                print(now, &runtime);
            }
        }
        runtime.tick(now);
        if (timeline && now >= from_ms)
            || (!timeline && (had_event || (until_ms.is_some() && now >= last_event && now > 0)))
        {
            print(now, &runtime);
        }
        if now == until {
            break;
        }
        let next_event = events.peek().map_or(until, |event| event.at_ms);
        let limit = if now < from_ms {
            from_ms
        } else if timeline || (until_ms.is_some() && now >= last_event) {
            now + 10
        } else {
            next_event
        };
        let deadline = runtime
            .next_deadline_ms()
            .filter(|&due| due > now)
            .unwrap_or(until);
        now = next_event.min(limit).min(deadline).min(until);
    }
    Ok(())
}
