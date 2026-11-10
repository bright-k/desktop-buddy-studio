use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf, process::ExitCode};
mod commands;

#[derive(Parser)]
#[command(
    name = "buddy",
    version,
    about = "Build independent desktop character apps from data"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Open a validated snapshot in the common Player supplied with this CLI.
    Preview {
        project: PathBuf,
        #[arg(long)]
        development: bool,
    },
    /// Assemble this CLI release's verified, self-contained Player template pack.
    TemplatePack {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify a frozen template before transferring its files to a build worker.
    VerifyTemplatePack { path: PathBuf },
    /// Build and package an independent branded app from a validated snapshot.
    Build {
        project: PathBuf,
        #[arg(long)]
        template_pack: Option<PathBuf>,
        #[arg(long, default_value = "artifacts/builds")]
        output: PathBuf,
        #[arg(long,value_parser=["macos-arm64","windows-x64"])]
        target: Option<String>,
        #[arg(long)]
        development: bool,
        #[arg(long)]
        signing_identity: Option<String>,
        #[arg(long, hide = true)]
        job_id: Option<uuid::Uuid>,
    },
    #[command(hide = true)]
    ArchiveBuild {
        source: PathBuf,
        directory: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Create a data-only project with a fresh product identity.
    New {
        directory: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "typing-cat", value_parser = ["empty", "typing-cat", "click-mascot"])]
        template: String,
    },
    /// Report toolchain, supported-host, and input-permission status.
    Doctor,
    /// Observe native input without logging key values (no implicit OS prompt).
    InputProbe {
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=60))]
        seconds: u64,
        #[arg(long)]
        request_permission: bool,
    },
    /// Run synthetic timed events through the production Rust behavior engine.
    Simulate {
        project: PathBuf,
        events: PathBuf,
        #[arg(long, requires = "timeline")]
        from_ms: Option<u64>,
        #[arg(long)]
        until_ms: Option<u64>,
        #[arg(long, requires = "timeline")]
        motion_preview: Option<String>,
        #[arg(long, requires = "timeline")]
        include_binding_ids: bool,
        #[arg(long)]
        timeline: bool,
    },
    /// Decode a resource under the same limits used by Player and Builder.
    MediaInfo {
        path: PathBuf,
        #[arg(long, requires = "frame_output")]
        frame: Option<usize>,
        #[arg(long, requires = "frame")]
        frame_output: Option<PathBuf>,
    },
    /// Inspect a validated project and optionally export normalized render frames.
    Inspect {
        project: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, hide = true)]
        allow_empty: bool,
    },
    /// Export only validated project files, preserving identity and content hash.
    Snapshot {
        project: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate a project, referenced files, and behavior contracts.
    Validate {
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Export JSON Schemas and TypeScript DTOs from the Rust contracts.
    ExportContracts {
        #[arg(long, default_value = "packages/contracts/generated")]
        output: PathBuf,
    },
}
fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Preview {
            project,
            development,
        } => {
            let cancellation = buddy_builder::CancellationToken::default();
            let token = cancellation.clone();
            ctrlc::set_handler(move || token.cancel()).map_err(|e| e.to_string())?;
            buddy_builder::preview(&project, development, cancellation, |line| {
                eprintln!("{line}")
            })
        }
        Command::TemplatePack { source, output } => {
            buddy_builder::create_template_pack(&source, &output)?;
            println!(
                "Created {} (template {})",
                output.display(),
                buddy_builder::template_hash()
            );
            Ok(())
        }
        Command::VerifyTemplatePack { path } => {
            buddy_builder::verify_template_pack(&path)?;
            println!(
                "{}",
                serde_json::json!({"templateHash": buddy_builder::template_hash()})
            );
            Ok(())
        }
        Command::Build {
            project,
            template_pack,
            output,
            target,
            development,
            signing_identity,
            job_id,
        } => {
            let target = match target.as_deref() {
                Some("windows-x64") => buddy_project::Target::WindowsX64,
                Some("macos-arm64") => buddy_project::Target::MacosArm64,
                _ => buddy_project::Target::current().ok_or("Unsupported build host")?,
            };
            let cancellation = buddy_builder::CancellationToken::default();
            let token = cancellation.clone();
            ctrlc::set_handler(move || token.cancel()).map_err(|e| e.to_string())?;
            let result = buddy_builder::build(
                buddy_builder::BuildOptions {
                    project,
                    template_pack,
                    output,
                    target,
                    development,
                    signing_identity,
                    job_id,
                },
                cancellation,
                |line| eprintln!("{line}"),
            )?;
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            Ok(())
        }
        Command::ArchiveBuild {
            source,
            directory,
            output,
        } => buddy_builder::write_build_archive(&source, &directory, &output)
            .map_err(|error| format!("Could not archive the build result: {error}")),
        Command::New {
            directory,
            name,
            template,
        } => commands::new_project(&directory, name, &template),
        Command::Doctor => commands::doctor(),
        Command::InputProbe {
            seconds,
            request_permission,
        } => commands::input_probe(seconds, request_permission),
        Command::Simulate {
            project,
            events,
            from_ms,
            until_ms,
            motion_preview,
            include_binding_ids,
            timeline,
        } => commands::simulate(
            &project,
            &events,
            from_ms.unwrap_or(0),
            until_ms,
            timeline,
            motion_preview.as_deref(),
            include_binding_ids,
        ),
        Command::MediaInfo {
            path,
            frame,
            frame_output,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .and_then(|f| f.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes))
                .map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("Resource exceeds 16 MiB".into());
            }
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if matches!(extension, "wav" | "mp3") {
                if frame.is_some() {
                    return Err("Audio resources do not have image frames".into());
                }
                buddy_project::validate_sound(extension, &bytes)?;
                println!(
                    "{}",
                    serde_json::json!({ "kind": "audio", "format": extension })
                );
                return Ok(());
            }
            let (media, png) = buddy_project::probe_media(&bytes, frame)?;
            if let (Some(output), Some(png)) = (frame_output, png) {
                fs::write(output, png).map_err(|e| e.to_string())?;
            }
            println!(
                "{}",
                serde_json::json!({"width":media.width,"height":media.height,"frames":media.frames,"animated":media.animated,"durationMs":media.duration_ms})
            );
            Ok(())
        }
        Command::Inspect {
            project,
            output,
            allow_empty,
        } => {
            let loaded = if allow_empty {
                buddy_project::load_editable_project(project)
            } else {
                buddy_project::load_project(project)
            }
            .map_err(|e| e.to_string())?;
            if let Some(dir) = output {
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                for (name, bytes) in &loaded.render_assets {
                    fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
                }
            }
            println!(
                "{}",
                serde_json::json!({"project": loaded.data.project, "character": loaded.data.character, "renderPlan": loaded.data.render_plan, "contentHash": loaded.content_hash, "sequences": loaded.data.sequences, "sequenceAlternatives": loaded.data.sequence_alternatives, "idleHeightDipAtScaleOne": loaded.idle_height_at_scale_one(), "images": loaded.render_assets.keys().collect::<Vec<_>>()})
            );
            Ok(())
        }
        Command::Snapshot { project, output } => {
            let source = buddy_project::load_project(project).map_err(|e| e.to_string())?;
            let snapshot = buddy_builder::write_project_snapshot(&source, &output)?;
            println!(
                "{}",
                serde_json::json!({"contentHash": snapshot.content_hash})
            );
            Ok(())
        }
        Command::Validate { project, json } => match buddy_project::load_project(project) {
            Ok(loaded) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"valid":true,"identifier":loaded.data.project.product.identifier,"contentHash":loaded.content_hash,"files":loaded.files.len()})
                    );
                } else {
                    println!(
                        "Valid: {} ({})\nContent hash: {}",
                        loaded.data.project.product.name,
                        loaded.data.project.product.identifier,
                        loaded.content_hash
                    );
                }
                Ok(())
            }
            Err(error) => {
                if json {
                    println!("{}", serde_json::json!({"valid":false,"errors":error.0}));
                    return Err("Project validation failed".into());
                }
                Err(error.to_string())
            }
        },
        Command::ExportContracts { output } => {
            fs::create_dir_all(&output).map_err(|e| e.to_string())?;
            for (name, schema) in [
                ("project.schema.json", schemars_project()),
                ("character.schema.json", schemars_character()),
            ] {
                write_contract(
                    output.join(name),
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&schema).map_err(|e| e.to_string())?
                    ),
                )
                .map_err(|e| e.to_string())?;
            }
            let declarations = format!(
                "// Generated by buddy export-contracts. Do not edit.\n{}{}",
                buddy_project::typescript_declarations(),
                buddy_runtime::typescript_declarations()
            );
            write_contract(output.join("index.ts"), declarations).map_err(|e| e.to_string())?;
            println!("Exported contracts to {}", output.display());
            Ok(())
        }
    }
}
fn schemars_project() -> serde_json::Value {
    buddy_project::project_schema()
}
fn schemars_character() -> serde_json::Value {
    buddy_project::character_schema()
}
fn write_contract(path: PathBuf, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    if fs::read(&path).ok().as_deref() == Some(content.as_ref()) {
        return Ok(());
    }
    fs::write(path, content)
}
