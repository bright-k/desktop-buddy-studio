use crate::{
    default_template_pack, materialize_template, process, template_hash, write_project_snapshot,
    CancellationToken,
};
use buddy_project::{LoadedProject, MacSigning, Target, WindowsSigning};
use fs2::FileExt;
use serde::Serialize;
use serde_json::json;
use std::{
    cell::RefCell,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

pub struct BuildOptions {
    pub project: PathBuf,
    pub template_pack: Option<PathBuf>,
    pub output: PathBuf,
    pub target: Target,
    pub development: bool,
    pub signing_identity: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pub job_id: String,
    pub directory: PathBuf,
    pub content_hash: String,
    pub template_hash: String,
    pub artifacts: Vec<Artifact>,
    pub development: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageRecord {
    name: String,
    started_ms: u128,
    ended_ms: Option<u128>,
    result: &'static str,
}
fn start_stage(
    name: &str,
    started: Instant,
    stages: &mut Vec<StageRecord>,
    progress: &mut impl FnMut(&str),
) {
    finish_stage(started, stages, "passed");
    stages.push(StageRecord {
        name: name.into(),
        started_ms: started.elapsed().as_millis(),
        ended_ms: None,
        result: "running",
    });
    progress(name);
}
fn finish_stage(started: Instant, stages: &mut [StageRecord], result: &'static str) {
    if let Some(stage) = stages.last_mut().filter(|stage| stage.ended_ms.is_none()) {
        stage.ended_ms = Some(started.elapsed().as_millis());
        stage.result = result;
    }
}

pub fn build(
    options: BuildOptions,
    cancellation: CancellationToken,
    mut progress: impl FnMut(&str),
) -> Result<BuildResult, String> {
    let started = Instant::now();
    let mut stages = Vec::new();
    let job = uuid::Uuid::new_v4().to_string();
    if Target::current() != Some(options.target) {
        return Err("UNSUPPORTED_BUILD_HOST: Windows apps require Windows x64; macOS apps require Apple Silicon macOS".into());
    }
    start_stage("validate-source", started, &mut stages, &mut progress);
    let source = buddy_project::load_project(&options.project).map_err(|e| e.to_string())?;
    let host = crate::host_environment();
    if !host.meets_support_baseline && !options.development {
        return Err("UNSUPPORTED_HOST_BASELINE: production builds require Windows 11 25H2 x64 or macOS 15 arm64; use --development only for a development artifact".into());
    }
    if !source.data.project.build.targets.contains(&options.target) {
        return Err("Requested target is not enabled by this project".into());
    }
    let pack = options
        .template_pack
        .clone()
        .map(Ok)
        .unwrap_or_else(default_template_pack)?;
    fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    let output = fs::canonicalize(&options.output).map_err(|e| e.to_string())?;
    let build_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(output.join(".buddy-build.lock"))
        .map_err(|e| e.to_string())?;
    build_lock.try_lock_exclusive().map_err(|_| {
        "BUILD_BUSY: another build is using this output/cache directory".to_string()
    })?;
    crate::identity::check_known_products(&output, &source.data.project)?;
    let work = tempfile::Builder::new()
        .prefix(".buddy-build-")
        .tempdir_in(&output)
        .map_err(|e| e.to_string())?;
    let log_name = format!("build-{job}.log");
    let log_path = output.join(&log_name);
    let log = RefCell::new(BufWriter::new(
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&log_path)
            .map_err(|e| e.to_string())?,
    ));
    let log_error = RefCell::new(None::<String>);
    let mut caller_progress = progress;
    let mut progress = |line: &str| {
        if log_error.borrow().is_none() {
            if let Err(error) = writeln!(
                log.borrow_mut(),
                "[{}ms] {line}",
                started.elapsed().as_millis()
            ) {
                *log_error.borrow_mut() = Some(error.to_string());
            }
        }
        caller_progress(line);
    };
    progress(&format!("job-id: {job}"));
    let result = (|| {
        start_stage("diagnose-toolchain", started, &mut stages, &mut progress);
        let toolchain = crate::diagnose_toolchain(Some(options.target));
        toolchain.require_ready()?;
        start_stage("snapshot", started, &mut stages, &mut progress);
        let snapshot = write_project_snapshot(&source, &work.path().join("project"))?;
        if cancellation.is_cancelled() {
            return Err("BUILD_CANCELLED".into());
        }
        start_stage("verify-template", started, &mut stages, &mut progress);
        let workspace = work.path().join("workspace");
        materialize_template(&pack, &workspace)?;
        start_stage("configure-product", started, &mut stages, &mut progress);
        let notices = configure(&workspace, &snapshot, &options)?;
        start_stage(
            "install-web-dependencies",
            started,
            &mut stages,
            &mut progress,
        );
        process::run(
            Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
                .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
                .current_dir(&workspace),
            &cancellation,
            &mut progress,
        )?;
        let tauri = workspace.join("node_modules/@tauri-apps/cli/tauri.js");
        start_stage("generate-icons", started, &mut stages, &mut progress);
        process::run(
            Command::new("node")
                .arg(&tauri)
                .arg("icon")
                .arg(workspace.join("apps/player/src-tauri/icons/source.png"))
                .arg("--output")
                .arg(workspace.join("apps/player/src-tauri/icons"))
                .current_dir(workspace.join("apps/player")),
            &cancellation,
            &mut progress,
        )?;
        start_stage("build-web", started, &mut stages, &mut progress);
        process::run(
            Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
                .args(["run", "build:player"])
                .current_dir(&workspace),
            &cancellation,
            &mut progress,
        )?;
        start_stage("compile-and-package", started, &mut stages, &mut progress);
        let cache = output
            .join(".buddy-cache")
            .join(template_hash())
            .join(options.target.rust_target());
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let mut package_command = Command::new("node");
        package_command.arg(&tauri).args([
            "build",
            "--target",
            options.target.rust_target(),
            "--bundles",
            if options.target == Target::MacosArm64 {
                "app,dmg"
            } else {
                "nsis"
            },
        ]);
        if options.development {
            package_command.arg("--debug");
        }
        package_command
            .args(["--", "--locked"])
            .env("CARGO_TARGET_DIR", &cache)
            .current_dir(workspace.join("apps/player"));
        process::run(&mut package_command, &cancellation, &mut progress)?;
        start_stage("inspect-artifacts", started, &mut stages, &mut progress);
        let staged = work.path().join("result");
        fs::create_dir(&staged).map_err(|e| e.to_string())?;
        let bundle = cache
            .join(options.target.rust_target())
            .join(if options.development {
                "debug/bundle"
            } else {
                "release/bundle"
            });
        if options.target == Target::MacosArm64 {
            let app = bundle
                .join("macos")
                .join(format!("{}.app", snapshot.data.project.product.name));
            if !app.is_dir() {
                return Err("Missing macOS .app bundle".into());
            }
            let staged_app = staged.join(app.file_name().unwrap());
            copy_tree(&app, &staged_app)?;
            let info = plist::Value::from_file(staged_app.join("Contents/Info.plist"))
                .map_err(|e| e.to_string())?;
            let dictionary = info.as_dictionary().ok_or("Invalid app Info.plist")?;
            for (key, expected) in [
                (
                    "CFBundleIdentifier",
                    &snapshot.data.project.product.identifier,
                ),
                (
                    "CFBundleShortVersionString",
                    &snapshot.data.project.product.version,
                ),
                ("CFBundleExecutable", &snapshot.data.project.product.slug),
            ] {
                if dictionary.get(key).and_then(plist::Value::as_string) != Some(expected.as_str())
                {
                    return Err(format!("App metadata mismatch: {key}"));
                }
            }
            verify_bundled_project(&staged_app.join("Contents/Resources"), &snapshot)?;
            copy_matching(
                &bundle.join("dmg"),
                &staged,
                "dmg",
                &format!(
                    "{}_{}_",
                    snapshot.data.project.product.name, snapshot.data.project.product.version
                ),
            )?;
            process::run(
                Command::new("codesign")
                    .args(["--verify", "--deep", "--strict"])
                    .arg(&staged_app),
                &cancellation,
                &mut progress,
            )?;
        } else {
            copy_matching(
                &bundle.join("nsis"),
                &staged,
                "exe",
                &format!(
                    "{}_{}_",
                    snapshot.data.project.product.name, snapshot.data.project.product.version
                ),
            )?;
        }
        let sign = match options.target {
            Target::MacosArm64 => match snapshot.data.project.build.signing.macos {
                MacSigning::AdHoc => "ad-hoc",
                MacSigning::SelfSigned => "self-signed",
            },
            Target::WindowsX64 => match snapshot.data.project.build.signing.windows {
                WindowsSigning::None => "none",
                WindowsSigning::SelfSigned => "self-signed",
            },
        };
        fs::write(
            staged.join("INSTALL.txt"),
            installation_guide(&snapshot, &options),
        )
        .map_err(|e| e.to_string())?;
        let licenses = staged.join("LICENSES");
        fs::create_dir(&licenses).map_err(|e| e.to_string())?;
        let char_dir = Path::new(&snapshot.data.project.character)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let license = char_dir
            .join(&snapshot.data.character.license)
            .to_string_lossy()
            .replace('\\', "/");
        fs::write(
            licenses.join("character.txt"),
            &snapshot.files[&license].bytes,
        )
        .map_err(|e| e.to_string())?;
        crate::copy_verified_notices(&workspace, &licenses.join("engine"))?;
        if let Some(error) = log_error.borrow().as_ref() {
            return Err(format!("BUILD_LOG_FAILED: {error}"));
        }
        log.borrow_mut()
            .flush()
            .map_err(|e| format!("BUILD_LOG_FAILED: {e}"))?;
        fs::copy(&log_path, staged.join("build.log")).map_err(|e| e.to_string())?;
        finish_stage(started, &mut stages, "passed");
        let mut artifacts = Vec::new();
        collect_artifacts(&staged, &staged, &mut artifacts)?;
        artifacts.sort_by(|a, b| a.path.cmp(&b.path));
        let data_identity = buddy_runtime::preference_identity(
            &snapshot.data.project.product.identifier,
            &snapshot.data.project.project_id,
            options.development,
        );
        let report = json!({"formatVersion":1,"jobId":job,"projectId":snapshot.data.project.project_id,"identifier":snapshot.data.project.product.identifier,"version":snapshot.data.project.product.version,"projectHash":snapshot.content_hash,"engineVersion":buddy_project::ENGINE_VERSION,"templateHash":template_hash(),"target":options.target,"host":host,"signing":sign,"notarized":false,"development":options.development,"artifacts":artifacts,"durationMs":started.elapsed().as_millis(),"staticInspection":"passed","toolchain":toolchain.tools,"toolchainPolicy":{"rustToolchain":toolchain.rust_toolchain,"automaticInstallation":toolchain.automatic_installation,"target":toolchain.target,"targetAvailable":toolchain.target_available,"ready":toolchain.ready,"problems":toolchain.problems},"lockHashes":{"cargo":buddy_project::sha256(&fs::read(workspace.join("Cargo.lock")).map_err(|e|e.to_string())?),"npm":buddy_project::sha256(&fs::read(workspace.join("package-lock.json")).map_err(|e|e.to_string())?)},"protocolVersion":buddy_runtime::PROTOCOL_VERSION,"projectSchemaVersion":1,"stages":stages,"log":"build.log","cache":{"kind":"cargo-dependencies","key":format!("{}-{:?}",template_hash(),options.target),"appArtifactCache":false},"platformIdentity":{"appIdentifier":snapshot.data.project.product.identifier,"binaryName":snapshot.data.project.product.slug,"dataIdentity":data_identity,"dataKey":buddy_project::sha256(data_identity.as_bytes()),"windows":crate::WindowsIdentity::new(&snapshot.data.project.product),"windowsInstallerIdentityValidation":"not-performed"},"thirdPartyNotices":notices,"externalDeviceValidation":"not-performed","platformRuntimeValidation":"not-performed"});
        fs::write(
            staged.join("build-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let mut sums = artifacts
            .iter()
            .map(|a| format!("{}  {}\n", a.sha256, a.path))
            .collect::<String>();
        sums.push_str(&format!(
            "{}  build-report.json\n",
            buddy_project::sha256(
                &fs::read(staged.join("build-report.json")).map_err(|e| e.to_string())?
            )
        ));
        fs::write(staged.join("SHA256SUMS"), sums).map_err(|e| e.to_string())?;
        if cancellation.is_cancelled() {
            return Err("BUILD_CANCELLED".into());
        }
        let destination = output.join(format!(
            "{}-{}-{}",
            snapshot.data.project.product.slug, snapshot.data.project.product.version, job
        ));
        fs::rename(&staged, &destination).map_err(|e| e.to_string())?;
        progress("complete");
        Ok(BuildResult {
            job_id: job.clone(),
            directory: destination,
            content_hash: snapshot.content_hash,
            template_hash: template_hash(),
            artifacts,
            development: options.development,
        })
    })();
    if let Err(error) = &result {
        finish_stage(
            started,
            &mut stages,
            if cancellation.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            },
        );
        let _ = log.borrow_mut().flush();
        let report = json!({"log":log_name,"stages":stages,"jobId":job,"result":if cancellation.is_cancelled(){"cancelled"}else{"failed"},"error":error,"durationMs":started.elapsed().as_millis()});
        let _ = fs::write(
            output.join(format!("failed-{job}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
    }
    result
}

fn verify_bundled_project(resources: &Path, snapshot: &LoadedProject) -> Result<(), String> {
    let entry =
        buddy_project::bundled_project_path(resources)?.ok_or("Bundled project is missing")?;
    if buddy_project::load_project(entry)
        .map_err(|e| e.to_string())?
        .content_hash
        != snapshot.content_hash
    {
        return Err("Bundled project differs from validated snapshot".into());
    }
    Ok(())
}

fn configure(
    workspace: &Path,
    snapshot: &LoadedProject,
    options: &BuildOptions,
) -> Result<crate::NoticesSummary, String> {
    let product = &snapshot.data.project.product;
    let tauri_dir = workspace.join("apps/player/src-tauri");
    let project = tauri_dir.join("resources/project");
    fs::create_dir_all(&project).map_err(|e| e.to_string())?;
    for (name, file) in &snapshot.files {
        let path = project.join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, &file.bytes).map_err(|e| e.to_string())?;
    }
    fs::write(
        tauri_dir.join("resources/project-entry.json"),
        serde_json::to_vec(&snapshot.entry_relative_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let notices =
        crate::copy_verified_notices(workspace, &tauri_dir.join("resources/third-party"))?;
    let config_path = tauri_dir.join("tauri.conf.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    config["productName"] = product.name.clone().into();
    config["identifier"] = product.identifier.clone().into();
    config["version"] = product.version.clone().into();
    config["mainBinaryName"] = product.slug.clone().into();
    config["bundle"]["resources"] = json!({"resources/project/":"project/","resources/project-entry.json":"project-entry.json","resources/third-party/":"third-party/"});
    config["bundle"]["icon"] = json!([
        "icons/32x32.png",
        "icons/128x128.png",
        "icons/128x128@2x.png",
        "icons/icon.icns",
        "icons/icon.ico"
    ]);
    config["bundle"]["targets"] = json!(if options.target == Target::MacosArm64 {
        vec!["app", "dmg"]
    } else {
        vec!["nsis"]
    });
    if options.target == Target::MacosArm64 {
        config["bundle"]["macOS"]["minimumSystemVersion"] =
            if options.development { "14.0" } else { "15.0" }.into();
        config["bundle"]["macOS"]["signingIdentity"] =
            match snapshot.data.project.build.signing.macos {
                MacSigning::AdHoc => "-".into(),
                MacSigning::SelfSigned => options
                    .signing_identity
                    .clone()
                    .ok_or("Self-signed mode requires an explicit local signing identity")?
                    .into(),
            };
    } else {
        let identity = crate::identity::configure_installer(workspace, product)?;
        config["bundle"]["windows"] = json!({"allowDowngrades":false,"nsis":{"installMode":"currentUser","template":"buddy-installer.nsi","startMenuFolder":identity.start_menu_folder}});
        if snapshot.data.project.build.signing.windows == WindowsSigning::SelfSigned {
            let identity = options
                .signing_identity
                .as_ref()
                .ok_or("Self-signed mode requires a local certificate thumbprint")?;
            if identity.len() != 40 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Expected a 40-character certificate thumbprint".into());
            }
            config["bundle"]["windows"]["certificateThumbprint"] = identity.clone().into();
            config["bundle"]["windows"]["digestAlgorithm"] = "sha256".into();
        }
    }
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap())
        .map_err(|e| e.to_string())?;
    let cargo_path = tauri_dir.join("Cargo.toml");
    let mut cargo: toml::Value =
        toml::from_str(&fs::read_to_string(&cargo_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut binary = toml::map::Map::new();
    binary.insert("name".into(), toml::Value::String(product.slug.clone()));
    binary.insert("path".into(), toml::Value::String("src/main.rs".into()));
    cargo
        .as_table_mut()
        .ok_or("Factory Cargo manifest is not a TOML table")?
        .insert(
            "bin".into(),
            toml::Value::Array(vec![toml::Value::Table(binary)]),
        );
    fs::write(
        cargo_path,
        toml::to_string_pretty(&cargo).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        tauri_dir.join("icons/source.png"),
        &snapshot.files[&product.icon].bytes,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        tauri_dir.join("icons/icon.png"),
        &snapshot.files[&product.icon].bytes,
    )
    .map_err(|e| e.to_string())?;
    Ok(notices)
}
fn copy_matching(
    source: &Path,
    destination: &Path,
    extension: &str,
    expected_prefix: &str,
) -> Result<(), String> {
    let mut count = 0;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) == Some(extension)
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(expected_prefix))
        {
            fs::copy(&path, destination.join(path.file_name().unwrap()))
                .map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!("Missing {extension} installer"));
    }
    Ok(())
}
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err("Unexpected symlink in generated bundle".into());
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn collect_artifacts(
    root: &Path,
    path: &Path,
    artifacts: &mut Vec<Artifact>,
) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_artifacts(root, &path, artifacts)?;
        } else {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            artifacts.push(Artifact {
                path: path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                bytes: bytes.len() as u64,
                sha256: buddy_project::sha256(&bytes),
            });
        }
    }
    Ok(())
}
fn installation_guide(snapshot: &LoadedProject, options: &BuildOptions) -> String {
    format!("{} {}\n\nTarget: {:?}\nDevelopment artifact: {}\n\nInstall the app using the supplied platform installer. macOS: copy the app to a stable location before granting Input Monitoring in System Settings. If a different app has the same display name, use separate folders and do not replace that app. Quit this product before replacing its app for an update, and keep its installation location stable. The tray menu opens settings and permission retry. Windows: the unsigned installer may show OS trust warnings; WebView2 may require an official download. OS approval and input permission are separate.\n\nKeyboard reactions work while other apps are focused. Korean keywords match configured two-set physical key sequences regardless of current IME mode. No text is entered, recorded, or sent over the network.\n\nUpdate using the same product identifier and slug. User preferences are preserved. Uninstall only this product; user data is preserved by default.\n\nExternal-device installation and permissions: NOT VERIFIED.\n",snapshot.data.project.product.name,snapshot.data.project.product.version,options.target,options.development)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_inspection_resolves_explicit_entry_and_rejects_changed_assets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let original = buddy_project::load_project(root.join("examples/typing-cat")).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        crate::write_project_snapshot(&original, &source).unwrap();
        fs::rename(
            source.join("buddy.project.json"),
            source.join("친구 설정.json"),
        )
        .unwrap();
        let snapshot = buddy_project::load_project(source.join("친구 설정.json")).unwrap();
        let resources = temp.path().join("resources");
        crate::write_project_snapshot(&snapshot, &resources.join("project")).unwrap();
        fs::write(
            resources.join("project-entry.json"),
            serde_json::to_vec(&snapshot.entry_relative_path).unwrap(),
        )
        .unwrap();
        verify_bundled_project(&resources, &snapshot).unwrap();
        fs::write(
            resources.join("project/character/LICENSE.txt"),
            "changed but valid UTF-8",
        )
        .unwrap();
        assert!(verify_bundled_project(&resources, &snapshot)
            .unwrap_err()
            .contains("differs from validated snapshot"));
    }
    #[test]
    fn previous_products_installers_are_not_published_from_shared_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("cache");
        let destination = temp.path().join("result");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("Typing Cat_0.1.0_aarch64.dmg"), "old product").unwrap();
        fs::write(
            source.join("Click Mascot_0.1.0_aarch64.dmg"),
            "current product",
        )
        .unwrap();
        copy_matching(&source, &destination, "dmg", "Click Mascot_0.1.0_").unwrap();
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        assert_eq!(
            fs::read_to_string(destination.join("Click Mascot_0.1.0_aarch64.dmg")).unwrap(),
            "current product"
        );
    }
    #[test]
    fn product_configuration_preserves_self_contained_workspace_and_resource_identity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let temp = tempfile::tempdir().unwrap();
        let pack = temp.path().join("pack");
        crate::create_template_pack(&root, &pack).unwrap();
        let workspace = temp.path().join("workspace");
        crate::materialize_template(&pack, &workspace).unwrap();
        let source = buddy_project::load_project(root.join("examples/click-mascot")).unwrap();
        let snapshot =
            crate::write_project_snapshot(&source, &temp.path().join("snapshot")).unwrap();
        let options = BuildOptions {
            project: source.root.clone(),
            template_pack: Some(pack),
            output: temp.path().join("out"),
            target: Target::MacosArm64,
            development: true,
            signing_identity: None,
        };
        configure(&workspace, &snapshot, &options).unwrap();
        let tauri = workspace.join("apps/player/src-tauri");
        let cargo: toml::Value =
            toml::from_str(&fs::read_to_string(tauri.join("Cargo.toml")).unwrap()).unwrap();
        assert_eq!(cargo["bin"][0]["name"].as_str(), Some("click-mascot"));
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(tauri.join("tauri.conf.json")).unwrap()).unwrap();
        assert_eq!(config["identifier"], "com.example.buddy.clickmascot");
        assert_eq!(config["mainBinaryName"], "click-mascot");
        assert_eq!(
            config["bundle"]["resources"]["resources/third-party/"],
            "third-party/"
        );
        assert!(tauri
            .join("resources/third-party/THIRD_PARTY_NOTICES.txt")
            .is_file());
        assert_eq!(
            buddy_project::load_project(tauri.join("resources/project"))
                .unwrap()
                .content_hash,
            snapshot.content_hash
        );
        let windows_workspace = temp.path().join("windows-workspace");
        crate::materialize_template(options.template_pack.as_ref().unwrap(), &windows_workspace)
            .unwrap();
        let windows_options = BuildOptions {
            target: Target::WindowsX64,
            ..options
        };
        configure(&windows_workspace, &snapshot, &windows_options).unwrap();
        let windows_tauri = windows_workspace.join("apps/player/src-tauri");
        let windows_config: serde_json::Value =
            serde_json::from_slice(&fs::read(windows_tauri.join("tauri.conf.json")).unwrap())
                .unwrap();
        assert_eq!(
            windows_config["bundle"]["windows"]["nsis"]["installMode"],
            "currentUser"
        );
        assert_eq!(
            windows_config["bundle"]["windows"]["allowDowngrades"],
            false
        );
        assert_eq!(
            windows_config["bundle"]["windows"]["nsis"]["template"],
            "buddy-installer.nsi"
        );
        assert!(windows_tauri.join("buddy-installer.nsi").is_file());
        assert!(!fs::read_to_string(tauri.join("Cargo.toml"))
            .unwrap()
            .contains(
                &fs::canonicalize(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            ));
    }
}
