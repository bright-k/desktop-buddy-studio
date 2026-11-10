use buddy_project::{load_project, sha256, CanvasLayout, Effect, LoadedProject};
use std::{collections::BTreeSet, fs, path::Path};

/// Derive a deployable project from a frozen, already validated editing snapshot.
pub(crate) fn write_runtime_snapshot(
    source: &LoadedProject,
    destination: &Path,
) -> Result<LoadedProject, String> {
    let mut project = source.data.project.clone();
    let mut character = source.data.character.clone();
    let mut animations = BTreeSet::from([character.default_animation.clone()]);
    let mut sounds = BTreeSet::new();
    if let Some(idle) = &project.idle {
        animations.extend(
            idle.directions
                .values()
                .filter_map(|pose| pose.animation.clone()),
        );
    }
    for group in &project.idle_groups {
        animations.extend(group.animation.clone());
        animations.extend(
            group
                .directions
                .values()
                .filter_map(|pose| pose.animation.clone()),
        );
    }
    if let Some(follow) = &project.cursor_follow {
        animations.extend(
            follow
                .bands
                .iter()
                .filter_map(|band| band.animation.clone()),
        );
    }
    for binding in &mut project.bindings {
        match &binding.effect {
            Effect::PlayAnimation {
                animation,
                variation,
                ..
            } => {
                animations.insert(animation.clone());
                if let Some(variation) = variation {
                    animations.extend(variation.alternatives.iter().cloned());
                }
            }
            Effect::SustainAnimation { animation, .. } => {
                animations.insert(animation.clone());
            }
            Effect::Move { animation, .. } => animations.extend(animation.clone()),
            Effect::FaceCursor { directions } => {
                animations.extend(
                    directions
                        .iter()
                        .flat_map(|poses| poses.values())
                        .filter_map(|pose| pose.animation.clone()),
                );
            }
        }
        if let Some(cue) = &binding.sound {
            if cue.volume > 0.0 {
                sounds.insert(cue.resource.clone());
            } else {
                binding.sound = None;
            }
        }
    }
    let original_layout = CanvasLayout {
        horizontal_anchor_extent: character.horizontal_anchor_extent(),
        scale_envelope: character.animation_scale_envelope(),
    };
    character.animations.retain(|id, _| animations.contains(id));
    if original_layout.horizontal_anchor_extent > character.horizontal_anchor_extent()
        || original_layout.scale_envelope > character.animation_scale_envelope()
    {
        character.canvas_layout = Some(original_layout);
    }
    let resources: BTreeSet<_> = character
        .animations
        .values()
        .filter_map(|animation| animation.resource.as_ref())
        .collect();
    character.resources.retain(|id, _| resources.contains(id));
    character.sounds.retain(|id, _| sounds.contains(id));
    if character
        .animations
        .values()
        .all(|animation| animation.resource.is_some())
    {
        character.image = None;
    }

    let character_dir = Path::new(&project.character).parent().unwrap();
    let mut assets = BTreeSet::from([destination.join(&project.product.icon)]);
    assets.extend(
        character
            .image
            .iter()
            .chain(std::iter::once(&character.license))
            .chain(character.resources.values().map(|resource| &resource.image))
            .chain(character.sounds.values().map(|sound| &sound.audio))
            .map(|path| destination.join(character_dir).join(path)),
    );
    if let Some(dmg) = &project.build.macos_dmg {
        assets.extend(
            dmg.background
                .iter()
                .chain(&dmg.guide)
                .map(|path| destination.join(path)),
        );
    }
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    for output in assets {
        let relative = output
            .strip_prefix(destination)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let asset = source
            .files
            .get(&relative)
            .ok_or_else(|| format!("Missing snapshot asset: {relative}"))?;
        fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(output, &asset.bytes).map_err(|e| e.to_string())?;
    }
    let entry = destination.join(&source.entry_relative_path);
    let manifest = destination.join(&project.character);
    fs::create_dir_all(manifest.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(
        manifest,
        serde_json::to_vec(&character).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        &entry,
        serde_json::to_vec(&project).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    load_project(entry).map_err(|e| e.to_string())
}

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
    fn runtime_snapshot_keeps_every_reaction_candidate_and_leaves_editing_assets_intact() {
        use serde_json::{json, Value};
        let temp = tempfile::tempdir().unwrap();
        let editing = temp.path().join("editing");
        write_project_snapshot(&load_project(example()).unwrap(), &editing).unwrap();
        let manifest = editing.join("character/manifest.json");
        let mut character: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        let sprite = editing.join("character/sprite.png");
        for name in [
            "idle-facing",
            "group",
            "group-facing",
            "follow",
            "weighted",
            "unused",
        ] {
            fs::copy(&sprite, sprite.with_file_name(format!("{name}.png"))).unwrap();
            character["resources"][name] = json!({"image":format!("{name}.png")});
            character["animations"][name] = json!({
                "resource": name, "frames": [0], "loop": name != "weighted"
            });
        }
        character["animations"]["unused"]["anchorX"] = json!(0.0);
        character["animations"]["unused"]["scalePercent"] = json!(200);
        let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x40\x1f\0\0\x01\0\x08\0data\x02\0\0\0\x80\x80";
        for name in ["audible", "silent", "unlinked"] {
            fs::write(sprite.with_file_name(format!("{name}.wav")), wave).unwrap();
            character["sounds"][name] = json!({"audio":format!("{name}.wav")});
        }
        fs::write(manifest, serde_json::to_vec(&character).unwrap()).unwrap();
        let entry = editing.join("buddy.project.json");
        let mut project: Value = serde_json::from_slice(&fs::read(&entry).unwrap()).unwrap();
        project["idle"] =
            json!({"cursorEnabled":true,"directions":{"left":{"animation":"idle-facing"}}});
        project["idleGroups"] = json!([{"id":"work","states":["countdown","focus"],
            "animation":"group","cursorMode":"custom","directions":{"up":{"animation":"group-facing"}}}]);
        project["cursorFollow"] = json!({"enabled":false,"stopDistanceDip":0,
            "bands":[{"minDistanceDip":0,"speedDipPerSecond":40,"animation":"follow"}]});
        let bindings = project["bindings"].as_array_mut().unwrap();
        bindings[3]["sound"] = json!({"resource":"silent","volume":0});
        bindings.push(json!({"id":"timer-start","trigger":{"type":"timerStarted","phase":"focus","alternatives":["countdown"]},
            "effect":{"type":"playAnimation","animation":"jump","variation":{"mode":"weighted","alternatives":["weighted"],"weights":[2,1]}},
            "sound":{"resource":"audible","volume":1}}));
        fs::write(entry, serde_json::to_vec(&project).unwrap()).unwrap();

        let source = load_project(&editing).unwrap();
        let runtime = write_runtime_snapshot(&source, &temp.path().join("runtime")).unwrap();
        assert_ne!(source.content_hash, runtime.content_hash);
        assert_eq!(runtime.canvas_size(1.0), source.canvas_size(1.0));
        assert_eq!(runtime.footprints.idle, source.footprints.idle);
        assert_eq!(
            runtime
                .data
                .character
                .resources
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["follow", "group", "group-facing", "idle-facing", "weighted"]
        );
        assert_eq!(
            runtime.data.character.animations.len(),
            source.data.character.animations.len() - 1
        );
        assert_eq!(
            runtime
                .data
                .character
                .sounds
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["audible"]
        );
        assert!(runtime.data.project.bindings[3].sound.is_none());
        assert!(!runtime.root.join("character/unused.png").exists());
        assert!(!runtime.root.join("character/unlinked.wav").exists());
        assert!(runtime.root.join("character/sprite.png").exists());
        assert_eq!(
            load_project(&editing).unwrap().content_hash,
            source.content_hash
        );
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
