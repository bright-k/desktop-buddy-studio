use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// The Builder writes this small descriptor outside the project. Project
/// filenames and hashes can remain unchanged, including explicit entry files.
pub fn bundled_project_path(resources: &Path) -> Result<Option<PathBuf>, String> {
    let descriptor = resources.join("project-entry.json");
    let project = resources.join("project");
    let name = if descriptor.exists() {
        let mut bytes = Vec::new();
        fs::File::open(&descriptor)
            .and_then(|file| file.take(1025).read_to_end(&mut bytes))
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 {
            return Err("Bundled project entry descriptor is too large".into());
        }
        let name: String = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid bundled project entry: {e}"))?;
        if Path::new(&name).file_name().and_then(|name| name.to_str()) != Some(name.as_str()) {
            return Err("Bundled project entry must be a filename".into());
        }
        name
    } else {
        if !project.join("buddy.project.json").is_file() {
            return Ok(None);
        }
        "buddy.project.json".into()
    };
    let project = fs::canonicalize(project).map_err(|e| e.to_string())?;
    crate::checked_path(&project, &name)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_entry_descriptor_and_canonical_fallback_resolve_inside_project() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let resources = root.as_path();
        assert!(bundled_project_path(resources).unwrap().is_none());
        fs::create_dir(resources.join("project")).unwrap();
        fs::write(resources.join("project/buddy.project.json"), b"{}").unwrap();
        assert_eq!(
            bundled_project_path(resources).unwrap().unwrap(),
            resources.join("project/buddy.project.json")
        );
        fs::write(resources.join("project/나의 친구.json"), b"{}").unwrap();
        fs::write(
            resources.join("project-entry.json"),
            serde_json::to_vec("나의 친구.json").unwrap(),
        )
        .unwrap();
        assert_eq!(
            bundled_project_path(resources).unwrap().unwrap(),
            resources.join("project/나의 친구.json")
        );
    }
    #[test]
    fn invalid_descriptors_fail_without_falling_back_to_another_project() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("project")).unwrap();
        fs::write(temp.path().join("project/buddy.project.json"), b"{}").unwrap();
        for name in [
            "../outside.json",
            "/outside.json",
            "other/entry.json",
            "other\\entry.json",
            "missing.json",
        ] {
            fs::write(
                temp.path().join("project-entry.json"),
                serde_json::to_vec(name).unwrap(),
            )
            .unwrap();
            assert!(bundled_project_path(temp.path()).is_err(), "{name}");
        }
        fs::write(temp.path().join("project-entry.json"), vec![b' '; 1025]).unwrap();
        assert!(bundled_project_path(temp.path())
            .unwrap_err()
            .contains("too large"));
        fs::write(temp.path().join("project-entry.json"), b"{\"entry\":42}").unwrap();
        assert!(bundled_project_path(temp.path()).is_err());
    }
}
