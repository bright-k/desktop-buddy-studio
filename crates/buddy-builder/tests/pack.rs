use buddy_builder::*;
use std::{fs, path::Path};
#[test]
fn pack_is_anchored_to_builder_and_can_materialize_without_original_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let pack = temp.path().join("pack");
    create_template_pack(&root, &pack).unwrap();
    let manifest = verify_template_pack(&pack).unwrap();
    assert!(manifest.files.iter().any(|file| file.path == "Cargo.lock"));
    let workspace = temp.path().join("workspace");
    materialize_template(&pack, &workspace).unwrap();
    assert!(workspace
        .join("apps/player/src-tauri/src/main.rs")
        .is_file());
    assert!(workspace.join("crates/buddy-runtime/src/lib.rs").is_file());
    fs::write(pack.join("crates/buddy-runtime/src/lib.rs"), "tampered").unwrap();
    assert!(
        verify_template_pack(&pack)
            .unwrap_err()
            .contains("mismatch")
            || verify_template_pack(&pack)
                .unwrap_err()
                .contains("integrity")
    );
}
#[test]
fn editing_a_manifest_cannot_reauthorize_modified_factory_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let pack = temp.path().join("pack");
    create_template_pack(&root, &pack).unwrap();
    let path = pack.join("pack-manifest.json");
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
    assert!(verify_template_pack(&pack)
        .unwrap_err()
        .contains("embedded"));
}
