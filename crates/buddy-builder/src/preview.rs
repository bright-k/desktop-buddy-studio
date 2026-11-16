use crate::{process, write_project_snapshot, CancellationToken};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn default_preview_executable() -> Result<PathBuf, String> {
    let base = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("CLI executable directory unavailable")?
        .join("player-preview");
    if cfg!(target_os = "macos") {
        Ok(base.join("Desktop Buddy Preview.app/Contents/MacOS/buddy-player"))
    } else if cfg!(windows) {
        Ok(base.join("buddy-player.exe"))
    } else {
        Err("UNSUPPORTED_PREVIEW_HOST".into())
    }
}

pub fn preview(
    project: &Path,
    development: bool,
    cancellation: CancellationToken,
    mut progress: impl FnMut(&str),
) -> Result<(), String> {
    if !crate::host_environment().meets_support_baseline && !development {
        return Err("UNSUPPORTED_HOST_BASELINE: preview requires a supported OS; --development is for development artifacts only".into());
    }
    let executable = default_preview_executable()?;
    if !executable.is_file() {
        return Err(
            "PREVIEW_PLAYER_MISSING: use the complete CLI distribution, including player-preview"
                .into(),
        );
    }
    let source = buddy_project::load_project(project).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix("buddy-preview-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let snapshot = write_project_snapshot(&source, &stage.path().join("project"))?;
    progress(
        "preview-start: 트레이에서 종료하면 미리보기가 끝납니다. Ctrl+C로도 중지할 수 있습니다.",
    );
    process::run(
        Command::new(executable)
            .arg("--project")
            .arg(snapshot.root.join(&snapshot.entry_relative_path)),
        &cancellation,
        &mut progress,
    )
}
