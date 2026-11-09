use buddy_project::Target;
use serde::Serialize;
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

pub fn pinned_rust_toolchain() -> &'static str {
    static CHANNEL: OnceLock<String> = OnceLock::new();
    CHANNEL.get_or_init(|| {
        let configuration: toml::Value =
            toml::from_str(include_str!("../../../rust-toolchain.toml"))
                .expect("the bundled Rust toolchain definition must be valid");
        configuration["toolchain"]["channel"]
            .as_str()
            .expect("the bundled Rust toolchain must name a channel")
            .to_owned()
    })
}

pub fn cargo_bin_directory() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".cargo"))
        })
        .map(|directory| directory.join("bin"))
}

/// Use the engine's pinned toolchain regardless of the caller's working
/// directory or RUSTUP_TOOLCHAIN override. Missing tools must not be installed.
pub fn configure_toolchain_environment(command: &mut Command) -> Result<(), String> {
    command
        .env("RUSTUP_TOOLCHAIN", pinned_rust_toolchain())
        .env("RUSTUP_AUTO_INSTALL", "0");
    let mut paths: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    if let Some(directory) = cargo_bin_directory() {
        paths.insert(0, directory);
    }
    command.env(
        "PATH",
        std::env::join_paths(paths).map_err(|e| e.to_string())?,
    );
    Ok(())
}

pub fn tool_command(name: &str) -> Command {
    let executable = if matches!(name, "cargo" | "rustc" | "rustup") {
        cargo_bin_directory()
            .map(|directory| {
                directory.join(if cfg!(windows) {
                    format!("{name}.exe")
                } else {
                    name.into()
                })
            })
            .filter(|path| path.is_file())
            .unwrap_or_else(|| name.into())
    } else {
        PathBuf::from(if cfg!(windows) && name == "npm" {
            "npm.cmd"
        } else {
            name
        })
    };
    let mut command = Command::new(executable);
    command
        .env("RUSTUP_TOOLCHAIN", pinned_rust_toolchain())
        .env("RUSTUP_AUTO_INSTALL", "0");
    command
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub tool: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainStatus {
    pub rust_toolchain: String,
    pub automatic_installation: bool,
    pub target: Option<String>,
    pub target_available: Option<bool>,
    pub tools: Vec<ToolStatus>,
    pub ready: bool,
    pub problems: Vec<String>,
}
impl ToolchainStatus {
    pub fn require_ready(&self) -> Result<(), String> {
        if self.ready {
            Ok(())
        } else {
            Err(format!("TOOLCHAIN_NOT_READY: {}", self.problems.join("; ")))
        }
    }
}

pub fn diagnose_toolchain(target: Option<Target>) -> ToolchainStatus {
    diagnose_with(target, tool_command)
}

fn diagnose_with(
    target: Option<Target>,
    mut command_for: impl FnMut(&str) -> Command,
) -> ToolchainStatus {
    let tools: Vec<_> = ["cargo", "rustc", "node", "npm"]
        .into_iter()
        .map(|name| match command_for(name).arg("--version").output() {
            Ok(output) => ToolStatus {
                tool: name.into(),
                available: output.status.success(),
                version: output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()),
                error: (!output.status.success())
                    .then(|| String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            },
            Err(error) => ToolStatus {
                tool: name.into(),
                available: false,
                version: None,
                error: Some(error.to_string()),
            },
        })
        .collect();
    let mut problems = Vec::new();
    for tool in &tools {
        if !tool.available {
            let remedy = if matches!(tool.tool.as_str(), "cargo" | "rustc") {
                format!("run rustup toolchain install {} --profile minimal --component rustfmt --component clippy", pinned_rust_toolchain())
            } else {
                "install Node.js 22 or newer with npm, then run buddy doctor again".into()
            };
            problems.push(format!("{} is unavailable; {remedy}", tool.tool));
        } else if matches!(tool.tool.as_str(), "cargo" | "rustc") {
            let actual = tool
                .version
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .nth(1);
            if actual != Some(pinned_rust_toolchain()) {
                problems.push(format!("{} must be {}; run rustup toolchain install {} --profile minimal --component rustfmt --component clippy", tool.tool, pinned_rust_toolchain(), pinned_rust_toolchain()));
            }
        } else if tool.tool == "node"
            && tool
                .version
                .as_deref()
                .and_then(|value| {
                    value
                        .trim_start_matches('v')
                        .split('.')
                        .next()?
                        .parse::<u32>()
                        .ok()
                })
                .is_none_or(|major| major < 22)
        {
            problems.push(
                "Node.js 22 or newer is required; install Node.js and run buddy doctor again"
                    .into(),
            );
        }
    }
    let target_available = target.map(|target| {
        let found = command_for("rustc")
            .args(["--print", "target-libdir", "--target", target.rust_target()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|directory| PathBuf::from(directory.trim()))
            .filter(|directory| directory.is_dir())
            .and_then(|directory| fs::read_dir(directory).ok())
            .is_some_and(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("libstd-") && name.ends_with(".rlib"))
                })
            });
        if !found {
            problems.push(format!(
                "Rust target {} is missing; run rustup target add --toolchain {} {}",
                target.rust_target(),
                pinned_rust_toolchain(),
                target.rust_target()
            ));
        }
        found
    });
    ToolchainStatus {
        rust_toolchain: pinned_rust_toolchain().into(),
        automatic_installation: false,
        target: target.map(|target| target.rust_target().into()),
        target_available,
        tools,
        ready: problems.is_empty(),
        problems,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn caller_toolchain_file_does_not_override_the_engine_pin() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"0.0.0\"\n",
        )
        .unwrap();
        let result = tool_command("rustc")
            .arg("--version")
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout)
                .split_whitespace()
                .nth(1),
            Some(pinned_rust_toolchain())
        );
    }

    #[test]
    fn preflight_rejects_wrong_versions_missing_programs_and_missing_target() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("tools.sh");
        fs::write(&script, "#!/bin/sh\ncase \"$1\" in\n cargo) echo 'cargo 1.98.0';;\n rustc) if [ \"$2\" = '--version' ]; then echo 'rustc 1.98.0'; else echo '/not-an-installed-rust-target'; fi;;\n node) echo 'v20.0.0';;\n npm) echo 'npm unavailable' >&2; exit 1;;\nesac\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let report = diagnose_with(Some(Target::MacosArm64), |name| {
            let mut command = Command::new(&script);
            command.arg(name);
            command
        });
        assert!(!report.ready);
        assert_eq!(report.problems.len(), 5);
        assert_eq!(report.target_available, Some(false));
        assert!(report
            .require_ready()
            .unwrap_err()
            .starts_with("TOOLCHAIN_NOT_READY:"));
        assert!(report
            .tools
            .iter()
            .find(|tool| tool.tool == "npm")
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("npm unavailable"));
    }
}
