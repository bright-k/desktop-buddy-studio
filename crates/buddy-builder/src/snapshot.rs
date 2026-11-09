use buddy_project::{load_project, sha256, LoadedProject};
use std::{fs, path::Path};

pub fn write_project_snapshot(
    source: &LoadedProject,
    destination: &Path,
) -> Result<LoadedProject, String> {
    if destination.exists() {
        return Err("Snapshot destination already exists".into());
    }
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for (name, file) in &source.files {
        verify_source(source, name, file)?;
        let output = destination.join(name);
        fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(output, &file.bytes).map_err(|e| e.to_string())?;
    }
    // Recheck the entry, manifest, and every referenced asset after copying.
    for (name, file) in &source.files {
        verify_source(source, name, file)?;
    }
    let validated =
        load_project(destination.join(&source.entry_relative_path)).map_err(|e| e.to_string())?;
    if validated.content_hash != source.content_hash {
        return Err("Snapshot content differs from the captured project".into());
    }
    Ok(validated)
}
fn verify_source(
    source: &LoadedProject,
    name: &str,
    file: &buddy_project::SourceAsset,
) -> Result<(), String> {
    use std::io::Read;
    let changed = || format!("PROJECT_CHANGED_DURING_SNAPSHOT: {name}");
    let path = buddy_project::checked_path(&source.root, name).map_err(|_| changed())?;
    let metadata = fs::metadata(&path).map_err(|_| changed())?;
    if path != file.path
        || metadata.len() != file.bytes.len() as u64
        || metadata.modified().ok() != file.modified
    {
        return Err(changed());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(file.bytes.len() as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| changed())?;
    if sha256(&bytes) != file.hash {
        return Err(changed());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/typing-cat")
    }
    #[test]
    fn snapshot_is_independent_after_final_validation() {
        let source = load_project(example()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let snapshot = write_project_snapshot(&source, &temp.path().join("snapshot")).unwrap();
        assert_ne!(source.root, snapshot.root);
        assert_eq!(source.content_hash, snapshot.content_hash);
        assert_eq!(
            load_project(&snapshot.root).unwrap().content_hash,
            source.content_hash
        );
    }
    #[test]
    fn explicit_entry_filename_is_preserved_through_snapshot_and_revalidation() {
        let original = load_project(example()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source");
        write_project_snapshot(&original, &source_dir).unwrap();
        fs::rename(
            source_dir.join("buddy.project.json"),
            source_dir.join("나의 친구.json"),
        )
        .unwrap();
        let source = load_project(source_dir.join("나의 친구.json")).unwrap();
        let snapshot = write_project_snapshot(&source, &temp.path().join("snapshot")).unwrap();
        assert_eq!(snapshot.entry_relative_path, "나의 친구.json");
        assert_eq!(snapshot.content_hash, source.content_hash);
        assert!(snapshot.root.join("나의 친구.json").is_file());
        assert!(!snapshot.root.join("buddy.project.json").exists());
        fs::remove_dir_all(source_dir).unwrap();
        assert_eq!(
            load_project(snapshot.root.join(&snapshot.entry_relative_path))
                .unwrap()
                .content_hash,
            snapshot.content_hash
        );
    }
    #[test]
    fn changed_source_is_rejected_and_existing_destination_is_preserved() {
        let original = load_project(example()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source");
        write_project_snapshot(&original, &source_dir).unwrap();
        let captured = load_project(&source_dir).unwrap();
        fs::write(source_dir.join("character/LICENSE.txt"), "changed").unwrap();
        assert!(
            write_project_snapshot(&captured, &temp.path().join("changed"))
                .unwrap_err()
                .contains("PROJECT_CHANGED")
        );
        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("keep"), "original").unwrap();
        assert!(write_project_snapshot(&original, &existing).is_err());
        assert_eq!(
            fs::read_to_string(existing.join("keep")).unwrap(),
            "original"
        );
    }
}
