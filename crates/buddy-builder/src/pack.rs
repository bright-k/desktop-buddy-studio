use buddy_project::sha256;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const EXPECTED_MANIFEST: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/template-manifest.json"));
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateManifest {
    pub format_version: u32,
    pub engine_version: String,
    pub files: Vec<TemplateFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
pub fn template_hash() -> String {
    sha256(EXPECTED_MANIFEST)
}
pub fn template_manifest() -> TemplateManifest {
    serde_json::from_slice(EXPECTED_MANIFEST).expect("embedded template manifest")
}

pub fn create_template_pack(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err("Template pack destination must not already exist".into());
    }
    let root = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix("buddy-template-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    for file in template_manifest().files {
        let path = buddy_project::checked_path(&root, &file.path).map_err(|e| e.to_string())?;
        let bytes = bounded_read(&path, file.size)?;
        if sha256(&bytes) != file.sha256 {
            return Err(format!("Template source differs from this Builder release: {}. Rebuild the CLI before packaging.",file.path));
        }
        let output = stage.path().join(&file.path);
        fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(output, bytes).map_err(|e| e.to_string())?;
    }
    fs::write(stage.path().join("pack-manifest.json"), EXPECTED_MANIFEST)
        .map_err(|e| e.to_string())?;
    fs::rename(stage.path(), destination).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn verify_template_pack(pack: &Path) -> Result<TemplateManifest, String> {
    let root = fs::canonicalize(pack).map_err(|e| e.to_string())?;
    let manifest = fs::read(root.join("pack-manifest.json")).map_err(|e| e.to_string())?;
    if sha256(&manifest) != template_hash() {
        return Err(
            "Template pack manifest is not the one embedded in this Builder release".into(),
        );
    }
    let manifest = template_manifest();
    for file in &manifest.files {
        let path = buddy_project::checked_path(&root, &file.path).map_err(|e| e.to_string())?;
        if sha256(&bounded_read(&path, file.size)?) != file.sha256 {
            return Err(format!("Template pack integrity failure: {}", file.path));
        }
    }
    Ok(manifest)
}
pub fn materialize_template(pack: &Path, destination: &Path) -> Result<(), String> {
    let manifest = verify_template_pack(pack)?;
    let root = fs::canonicalize(pack).map_err(|e| e.to_string())?;
    if destination.exists() {
        return Err("Template workspace destination already exists".into());
    }
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for file in manifest.files {
        let source = buddy_project::checked_path(&root, &file.path).map_err(|e| e.to_string())?;
        let bytes = bounded_read(&source, file.size)?;
        if sha256(&bytes) != file.sha256 {
            return Err(format!(
                "Template changed while materializing: {}",
                file.path
            ));
        }
        let output = destination.join(&file.path);
        fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(output, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn bounded_read(path: &Path, size: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if size > 64 * 1024 * 1024 {
        return Err("Unexpected template file size".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(size + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != size {
        return Err(format!("Template file size mismatch: {}", path.display()));
    }
    Ok(bytes)
}

pub fn default_template_pack() -> Result<PathBuf, String> {
    Ok(std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("CLI executable directory unavailable")?
        .join("player-template-pack"))
}
