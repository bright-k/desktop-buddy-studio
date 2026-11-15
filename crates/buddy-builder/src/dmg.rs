use crate::{process, CancellationToken};
use std::{fs, path::Path, process::Command};

pub fn create(
    workspace: &Path,
    settings_file: &Path,
    settings: serde_json::Value,
    signing_identity: Option<&str>,
    cancellation: &CancellationToken,
    progress: &mut impl FnMut(&str),
) -> Result<(), String> {
    let output = settings["output"].as_str().ok_or("Missing DMG output")?;
    fs::write(
        settings_file,
        serde_json::to_vec(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    process::run(
        Command::new("python3")
            .arg(workspace.join("packaging/macos/create-dmg.py"))
            .arg(settings_file),
        cancellation,
        progress,
    )?;
    if let Some(identity) = signing_identity {
        process::run(
            Command::new("/usr/bin/codesign").args(["--sign", identity, output]),
            cancellation,
            progress,
        )?;
    }
    Ok(())
}
