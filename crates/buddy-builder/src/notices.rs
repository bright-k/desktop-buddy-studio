use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoticeManifest {
    format_version: u32,
    generator: String,
    targets: Vec<String>,
    input_hashes: BTreeMap<String, String>,
    crate_count: usize,
    npm_count: usize,
    source_archive_count: usize,
    files: Vec<NoticeFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeFile {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesSummary {
    pub status: &'static str,
    pub generator: String,
    pub rust_packages: usize,
    pub web_tool_packages: usize,
    pub source_archives: usize,
    pub inventory_sha256: String,
    pub manifest_sha256: String,
}

pub fn copy_verified_notices(
    workspace: &Path,
    destination: &Path,
) -> Result<NoticesSummary, String> {
    let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    let source = root.join("packaging/notices");
    let manifest_bytes =
        fs::read(source.join("manifest.json")).map_err(|e| format!("NOTICES_MISSING: {e}"))?;
    let manifest: NoticeManifest =
        buddy_project::parse_json(&manifest_bytes, "notices/manifest.json")
            .map_err(|e| e.to_string())?;
    if manifest.format_version != 1
        || manifest.crate_count == 0
        || manifest.files.len() > 1024
        || !["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
            .iter()
            .all(|target| manifest.targets.iter().any(|t| t == target))
    {
        return Err("NOTICES_INVALID: unsupported or incomplete catalog".into());
    }
    for required in [
        "Cargo.lock",
        "package-lock.json",
        "LICENSE",
        "scripts/prepare-notices.mjs",
    ] {
        if !manifest.input_hashes.contains_key(required) {
            return Err(format!("NOTICES_INVALID: missing input {required}"));
        }
    }
    for (name, expected) in &manifest.input_hashes {
        let path = buddy_project::checked_path(&root, name).map_err(|e| e.to_string())?;
        if buddy_project::sha256(&fs::read(path).map_err(|e| e.to_string())?) != *expected {
            return Err(format!(
                "NOTICES_STALE: {name} differs from the prepared dependency notices"
            ));
        }
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for file in &manifest.files {
        total = total.checked_add(file.bytes).ok_or("NOTICES_TOO_LARGE")?;
        if total > 64 * 1024 * 1024 {
            return Err("NOTICES_TOO_LARGE".into());
        }
        let path = buddy_project::checked_path(&source, &file.path).map_err(|e| e.to_string())?;
        use std::io::Read;
        let mut data = Vec::new();
        fs::File::open(path)
            .and_then(|f| f.take(file.bytes + 1).read_to_end(&mut data))
            .map_err(|e| e.to_string())?;
        if data.len() as u64 != file.bytes || buddy_project::sha256(&data) != file.sha256 {
            return Err(format!("NOTICES_INTEGRITY: {}", file.path));
        }
        if files.insert(file.path.clone(), data).is_some() {
            return Err("NOTICES_INVALID: duplicate path".into());
        }
    }
    for required in [
        "inventory.json",
        "THIRD_PARTY_NOTICES.txt",
        "ENGINE-LICENSE.txt",
    ] {
        if !files.contains_key(required) {
            return Err(format!("NOTICES_INVALID: missing {required}"));
        }
    }
    if destination.exists() {
        return Err("Notice destination already exists".into());
    }
    let inventory_sha256 = buddy_project::sha256(&files["inventory.json"]);
    for (name, bytes) in files {
        let target = destination.join(name);
        fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    fs::write(destination.join("manifest.json"), &manifest_bytes).map_err(|e| e.to_string())?;
    Ok(NoticesSummary {
        status: "bundled-and-verified",
        generator: manifest.generator,
        rust_packages: manifest.crate_count,
        web_tool_packages: manifest.npm_count,
        source_archives: manifest.source_archive_count,
        inventory_sha256,
        manifest_sha256: buddy_project::sha256(&manifest_bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) {
        for (name, data) in [
            ("Cargo.lock", "locked"),
            ("package-lock.json", "locked"),
            ("LICENSE", "MIT"),
            ("scripts/prepare-notices.mjs", "generator"),
            ("packaging/notices/inventory.json", "{}"),
            ("packaging/notices/THIRD_PARTY_NOTICES.txt", "terms"),
            ("packaging/notices/ENGINE-LICENSE.txt", "MIT"),
        ] {
            fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
            fs::write(root.join(name), data).unwrap();
        }
        let input_hashes: BTreeMap<_, _> = [
            "Cargo.lock",
            "package-lock.json",
            "LICENSE",
            "scripts/prepare-notices.mjs",
        ]
        .into_iter()
        .map(|name| {
            (
                name,
                buddy_project::sha256(&fs::read(root.join(name)).unwrap()),
            )
        })
        .collect();
        let files: Vec<_> = ["inventory.json", "THIRD_PARTY_NOTICES.txt", "ENGINE-LICENSE.txt"].into_iter().map(|name| { let data = fs::read(root.join("packaging/notices").join(name)).unwrap(); serde_json::json!({"path":name,"bytes":data.len(),"sha256":buddy_project::sha256(&data)}) }).collect();
        fs::write(root.join("packaging/notices/manifest.json"), serde_json::to_vec(&serde_json::json!({"formatVersion":1,"generator":"test","targets":["aarch64-apple-darwin","x86_64-pc-windows-msvc"],"inputHashes":input_hashes,"crateCount":1,"npmCount":0,"sourceArchiveCount":0,"files":files})).unwrap()).unwrap();
    }
    #[test]
    fn changed_lock_or_notice_is_rejected_before_copying() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        fs::write(root.path().join("Cargo.lock"), "changed").unwrap();
        let destination = root.path().join("output");
        assert!(copy_verified_notices(root.path(), &destination)
            .unwrap_err()
            .contains("NOTICES_STALE"));
        assert!(!destination.exists());
        fixture(root.path());
        fs::write(
            root.path()
                .join("packaging/notices/THIRD_PARTY_NOTICES.txt"),
            "tampered",
        )
        .unwrap();
        assert!(copy_verified_notices(root.path(), &destination)
            .unwrap_err()
            .contains("NOTICES_INTEGRITY"));
        assert!(!destination.exists());
    }
    #[test]
    fn only_verified_files_are_published_and_existing_output_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        fs::write(
            root.path().join("packaging/notices/unlisted.txt"),
            "not part of the catalog",
        )
        .unwrap();
        let destination = root.path().join("output");
        assert_eq!(
            copy_verified_notices(root.path(), &destination)
                .unwrap()
                .rust_packages,
            1
        );
        assert!(!destination.join("unlisted.txt").exists());
        assert!(copy_verified_notices(root.path(), &destination).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("THIRD_PARTY_NOTICES.txt")).unwrap(),
            "terms"
        );
    }
}
