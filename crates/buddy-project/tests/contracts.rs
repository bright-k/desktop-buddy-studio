use buddy_project::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/typing-cat")
}
fn parts() -> (Project, Character) {
    (
        parse_json(
            include_bytes!("../../../examples/typing-cat/buddy.project.json"),
            "buddy.project.json",
        )
        .unwrap(),
        parse_json(
            include_bytes!("../../../examples/typing-cat/character/manifest.json"),
            "character/manifest.json",
        )
        .unwrap(),
    )
}
fn copy_example() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "buddy.project.json",
        "assets/app-icon.png",
        "character/manifest.json",
        "character/sprite.png",
        "character/LICENSE.txt",
    ] {
        let destination = dir.path().join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(example().join(name), destination).unwrap();
    }
    dir
}

#[test]
fn two_examples_validate_and_have_different_identity_and_content() {
    let cat = load_project(example()).unwrap();
    let mascot = load_project(example().with_file_name("click-mascot")).unwrap();
    assert_ne!(
        cat.data.project.product.identifier,
        mascot.data.project.product.identifier
    );
    assert_ne!(cat.content_hash, mascot.content_hash);
    assert_eq!(cat.files.len(), 5);
    assert_eq!(cat.data.sequences.iter().flatten().count(), 1);
}
#[test]
fn semantic_errors_name_the_explicit_entry_file() {
    let directory = copy_example();
    let entry = directory.path().join("친구 설정.json");
    fs::rename(directory.path().join("buddy.project.json"), &entry).unwrap();
    let mut project: serde_json::Value =
        serde_json::from_slice(&fs::read(&entry).unwrap()).unwrap();
    project["product"]["slug"] = serde_json::json!("X");
    fs::write(&entry, serde_json::to_vec(&project).unwrap()).unwrap();
    let error = load_project(&entry).unwrap_err();
    let diagnostic = error
        .0
        .iter()
        .find(|diagnostic| diagnostic.path == "$.product.slug")
        .unwrap();
    assert_eq!(diagnostic.file, "친구 설정.json");
}
#[test]
fn duplicate_json_and_unknown_fields_are_errors_with_locations() {
    assert!(parse_json::<Project>(
        br#"{"schemaVersion":1,"nested":{"x":1,"x":2}}"#,
        "project.json"
    )
    .unwrap_err()
    .to_string()
    .contains("duplicate JSON field"));
    let (project, _) = parts();
    let mut value = serde_json::to_value(project).unwrap();
    value["product"]["shellCommand"] = "unexpected".into();
    let error =
        parse_json::<Project>(&serde_json::to_vec(&value).unwrap(), "project.json").unwrap_err();
    assert!(error.to_string().contains("project.json:$.product"));
    assert!(error.to_string().contains("unknown field"));
}
#[test]
fn defaults_are_filled_but_invalid_values_are_not_replaced() {
    let (project, character) = parts();
    let mut value = serde_json::to_value(project).unwrap();
    value["defaults"] = serde_json::json!({"input":{"mouse":false}});
    let project: Project = parse_json(&serde_json::to_vec(&value).unwrap(), "project").unwrap();
    assert!(project.defaults.input.keyboard);
    assert!(!project.defaults.input.mouse);
    assert_eq!(project.defaults.scale, 1.0);
    value["defaults"]["scale"] = 0.01.into();
    let project = parse_json(&serde_json::to_vec(&value).unwrap(), "project").unwrap();
    assert!(validate_parts(project, character)
        .unwrap_err()
        .to_string()
        .contains("defaults.scale"));
}
#[test]
fn loops_references_and_frame_count_are_semantically_checked() {
    let (mut project, mut character) = parts();
    character.animations.get_mut("jump").unwrap().looping = true;
    character.animations.get_mut("walk").unwrap().frames = vec![16];
    if let Effect::Move { animation, .. } = &mut project.bindings.last_mut().unwrap().effect {
        *animation = Some("missing".into());
    }
    let error = validate_parts(project, character).unwrap_err().to_string();
    assert!(error.contains("non-loop"));
    assert!(error.contains("frameCount"));
    assert!(error.contains("move must reference"));
}
#[test]
fn overlapping_ranges_and_duplicate_compiled_sequences_fail() {
    let (mut project, character) = parts();
    if let Trigger::TypingRate { min_kps, .. } = &mut project.bindings[1].trigger {
        *min_kps = 2.0;
    }
    let mut duplicate = project
        .bindings
        .iter()
        .find(|b| matches!(b.trigger, Trigger::KeySequence { .. }))
        .unwrap()
        .clone();
    duplicate.id = "duplicate-pattern".into();
    if let Trigger::KeySequence {
        keyword,
        layout,
        steps,
        ..
    } = &mut duplicate.trigger
    {
        *keyword = None;
        *layout = None;
        *steps = Some(compile_keyword("고양이", KeyboardLayout::Korean2Set).unwrap());
    }
    project.bindings.push(duplicate);
    let error = validate_parts(project, character).unwrap_err().to_string();
    assert!(error.contains("overlap"));
    assert!(error.contains("duplicate compiled"));
}
#[test]
fn zero_and_extreme_sheet_dimensions_fail_without_panicking() {
    let (project, mut character) = parts();
    character.sheet.width = u32::MAX - 15;
    character.sheet.height = u32::MAX - 15;
    assert!(validate_parts(project.clone(), character.clone()).is_err());
    character.sheet.frame_width = 0;
    assert!(validate_parts(project, character).is_err());
}
#[test]
fn unsafe_paths_and_reserved_names_are_rejected() {
    let root = fs::canonicalize(example()).unwrap();
    for value in [
        "../buddy.project.json",
        "/etc/passwd",
        "assets/../app-icon.png",
        "C:\\icon.png",
        "assets/CON.png",
        "assets/nul.txt",
        "assets/com1.png",
        "assets/trailing.",
    ] {
        assert!(checked_path(&root, value).is_err(), "{value}");
    }
}
#[cfg(unix)]
#[test]
fn symlink_cannot_escape_the_project() {
    let dir = copy_example();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::remove_file(dir.path().join("assets/app-icon.png")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("assets/app-icon.png")).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("escapes project root"));
}
#[test]
fn image_header_budget_is_enforced_before_decoding() {
    let dir = copy_example();
    let file = dir.path().join("character/sprite.png");
    let mut bytes = fs::read(&file).unwrap();
    bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    fs::write(file, bytes).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("IMAGE_TOO_LARGE"));
}
#[test]
fn json_size_and_future_schema_are_rejected() {
    assert!(parse_json::<Project>(&vec![b' '; 1024 * 1024 + 1], "large")
        .unwrap_err()
        .to_string()
        .contains("JSON_TOO_LARGE"));
    let (mut project, character) = parts();
    project.schema_version = 2;
    assert!(validate_parts(project, character)
        .unwrap_err()
        .to_string()
        .contains("schemaVersion"));
}

#[test]
fn product_names_must_also_work_as_installer_and_app_filenames() {
    for name in [
        "CON",
        "buddy.",
        "buddy ",
        "bad:name",
        "bad\"name",
        "bad/name",
    ] {
        let (mut project, character) = parts();
        project.product.name = name.into();
        assert!(
            validate_parts(project, character).is_err(),
            "accepted invalid name {name}"
        );
    }
    let (mut project, character) = parts();
    project.product.name = "😀".repeat(80);
    assert!(validate_parts(project, character).is_err());
    let (mut project, character) = parts();
    project.product.name = "나의 친구's Buddy".into();
    assert!(validate_parts(project, character).is_ok());
}
