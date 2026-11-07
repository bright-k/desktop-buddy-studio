use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, path: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(path).expect("template source directory") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| {
                [
                    "rs", "toml", "ts", "css", "html", "json", "mjs", "png", "ico", "icns", "txt",
                    "py", "nsi", "crate", "gif", "jpg", "jpeg", "webp", "bmp",
                ]
                .contains(&ext)
            })
        {
            files.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let root = fs::canonicalize(root).unwrap();
    let mut files = vec![
        "Cargo.toml".into(),
        "Cargo.lock".into(),
        "rust-toolchain.toml".into(),
        "package.json".into(),
        "package-lock.json".into(),
        "LICENSE".into(),
        ".github/workflows/studio-build.yml".into(),
        "apps/studio/index.html".into(),
        "apps/studio/tsconfig.json".into(),
        "apps/studio/vite.config.ts".into(),
        "apps/player/index.html".into(),
        "apps/player/tsconfig.json".into(),
        "apps/player/vite.config.ts".into(),
        "apps/player/src-tauri/Cargo.toml".into(),
        "apps/player/src-tauri/build.rs".into(),
        "apps/player/src-tauri/tauri.conf.json".into(),
        "apps/player/src-tauri/tauri.dev.json".into(),
    ];
    for dir in [
        "crates",
        "studio",
        "apps/studio/src",
        "tools/buddy/src",
        "apps/player/src",
        "apps/player/src-tauri/src",
        "apps/player/src-tauri/icons",
        "apps/player/src-tauri/capabilities",
        "packages/renderer/src",
        "packages/contracts/generated",
        "examples",
        "scripts",
        "packaging",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
        collect(&root, &root.join(dir), &mut files);
    }
    files.push("tools/buddy/Cargo.toml".into());
    files.sort();
    files.dedup();
    let records:Vec<_>=files.into_iter().map(|name|{
        let path=root.join(&name);println!("cargo:rerun-if-changed={}",path.display());
        let bytes=fs::read(path).expect("all template inputs must exist");
        serde_json::json!({"path":name,"size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))})
    }).collect();
    let manifest = serde_json::json!({"formatVersion":1,"engineVersion":std::env::var("CARGO_PKG_VERSION").unwrap(),"files":records});
    fs::write(
        PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("template-manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
