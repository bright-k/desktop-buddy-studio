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
                InputEvent::DragStart | InputEvent::DragEnd => "drag",
                InputEvent::BuddyClick => "buddyClick",
                InputEvent::Rearm { .. } => "rearm",
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
    let character =
        buddy_project::parse_json(manifest, "template manifest").map_err(|e| e.to_string())?;
    buddy_project::validate_parts(project.clone(), character).map_err(|e| e.to_string())?;
    let files: [(&str, &[u8]); 4] = [
        ("character/manifest.json", manifest),
        (
            "character/sprite.png",
            if mascot {
                include_bytes!("../../../examples/click-mascot/character/sprite.png")
            } else {
                include_bytes!("../../../examples/typing-cat/character/sprite.png")
            },
        ),
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
pub fn simulate(
    project: &Path,
    events: &Path,
    until_ms: Option<u64>,
    timeline: bool,
) -> Result<(), String> {
    let loaded = buddy_project::load_project(project).map_err(|e| e.to_string())?;
    let bytes = fs::read(events).map_err(|e| e.to_string())?;
    let events: Vec<TimedEvent> = buddy_project::parse_json(&bytes, &events.display().to_string())
        .map_err(|e| e.to_string())?;
    if !events.windows(2).all(|pair| pair[0].at_ms <= pair[1].at_ms) {
        return Err("Synthetic event times must be monotonic".into());
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
    let print = |at_ms: u64, runtime: &Runtime| {
        if timeline {
            println!(
                "{}",
                serde_json::json!({"atMs": at_ms, "state": runtime.state()})
            );
        } else {
            println!("{}", serde_json::to_string(&runtime.state()).unwrap());
        }
    };
    let mut now = 0;
    if timeline {
        print(0, &runtime);
    }
    for event in events {
        if event.at_ms > 60000 {
            return Err("simulation maximum 60 seconds".into());
        }
        if timeline {
            while now + 10 < event.at_ms {
                now += 10;
                runtime.advance(now);
                print(now, &runtime);
            }
        }
        now = event.at_ms;
        runtime.handle(now, event.event);
        print(now, &runtime);
    }
    if let Some(until) = until_ms {
        if until > 60000 || until < now {
            return Err("invalid until-ms (maximum 60000)".into());
        }
        while now < until {
            now = (now + 10).min(until);
            runtime.advance(now);
            print(now, &runtime);
        }
    }
    Ok(())
}
