use buddy_project::{sha256, Product, Project};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsIdentity {
    pub key: String,
    pub install_directory: String,
    pub uninstall_registry_key: String,
    pub preferences_registry_key: String,
    pub shortcut_name: String,
    pub start_menu_folder: String,
}

pub(crate) fn check_known_products(output: &Path, project: &Project) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PriorBuild {
        project_id: String,
        identifier: String,
        #[serde(default)]
        platform_identity: serde_json::Value,
    }
    for entry in fs::read_dir(output).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let report = entry.path().join("build-report.json");
        if !report.is_file() {
            continue;
        }
        if fs::metadata(&report).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
            return Err("EXISTING_REPORT_INVALID: a prior build report is too large".into());
        }
        let previous: PriorBuild = buddy_project::parse_json(
            &fs::read(report).map_err(|e| e.to_string())?,
            "prior build-report.json",
        )
        .map_err(|_| {
            "EXISTING_REPORT_INVALID: cannot check product identity against a prior build"
                .to_string()
        })?;
        if previous.identifier != project.product.identifier {
            continue;
        }
        if previous.project_id != project.project_id {
            return Err("PRODUCT_IDENTITY_CONFLICT: this identifier was already built for another project; use a fresh identifier for an independent app".into());
        }
        if previous
            .platform_identity
            .get("binaryName")
            .and_then(|v| v.as_str())
            .is_some_and(|name| name != project.product.slug)
        {
            return Err("PRODUCT_SLUG_CHANGED: keep the executable slug for an update, or assign a new identifier for a new product".into());
        }
    }
    Ok(())
}
impl WindowsIdentity {
    pub fn new(product: &Product) -> Self {
        let key = sha256(product.identifier.as_bytes());
        Self {
            install_directory: format!(r"%LOCALAPPDATA%\Programs\DesktopBuddy\{key}"),
            uninstall_registry_key: format!(
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\DesktopBuddy-{key}"
            ),
            preferences_registry_key: format!(r"HKCU\Software\DesktopBuddy\{key}"),
            shortcut_name: format!("{} ({}).lnk", product.slug, &key[..8]),
            start_menu_folder: format!(r"Desktop Buddy\{} ({})", product.slug, &key[..8]),
            key,
        }
    }
}

pub(crate) fn configure_installer(
    workspace: &Path,
    product: &Product,
) -> Result<WindowsIdentity, String> {
    let identity = WindowsIdentity::new(product);
    let source = fs::read_to_string(workspace.join("packaging/windows/installer.nsi"))
        .map_err(|e| e.to_string())?;
    if source.matches("@BUDDY_IDENTITY@").count() != 1
        || source.matches("@BUDDY_SHORT_ID@").count() != 1
    {
        return Err(
            "INVALID_INSTALLER_TEMPLATE: identity placeholders differ from the pinned template"
                .into(),
        );
    }
    let text = source
        .replace("@BUDDY_IDENTITY@", &identity.key)
        .replace("@BUDDY_SHORT_ID@", &identity.key[..8]);
    fs::write(
        workspace.join("apps/player/src-tauri/buddy-installer.nsi"),
        text,
    )
    .map_err(|e| e.to_string())?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn product() -> Product {
        serde_json::from_value(serde_json::json!({"name":"Same Display Name","slug":"buddy-app","identifier":"io.example.buddy.first","version":"1.0.0","icon":"icon.png"})).unwrap()
    }
    #[test]
    fn rename_and_version_preserve_identity_but_different_products_are_isolated() {
        let first = product();
        let mut renamed = first.clone();
        renamed.name = "Renamed Display".into();
        renamed.version = "2.0.0".into();
        assert_eq!(
            serde_json::to_value(WindowsIdentity::new(&first)).unwrap(),
            serde_json::to_value(WindowsIdentity::new(&renamed)).unwrap()
        );
        let mut second = first.clone();
        second.identifier = "io.example.buddy.second".into();
        let a = WindowsIdentity::new(&first);
        let b = WindowsIdentity::new(&second);
        assert_ne!(a.install_directory, b.install_directory);
        assert_ne!(a.uninstall_registry_key, b.uninstall_registry_key);
        assert_ne!(a.shortcut_name, b.shortcut_name);
        assert_eq!(a.key.len(), 64);
    }
    #[test]
    fn installer_uses_only_its_identity_and_preserves_unrelated_links() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("packaging/windows")).unwrap();
        fs::create_dir_all(temp.path().join("apps/player/src-tauri")).unwrap();
        fs::copy(
            root.join("packaging/windows/installer.nsi"),
            temp.path().join("packaging/windows/installer.nsi"),
        )
        .unwrap();
        let identity = configure_installer(temp.path(), &product()).unwrap();
        let text = fs::read_to_string(
            temp.path()
                .join("apps/player/src-tauri/buddy-installer.nsi"),
        )
        .unwrap();
        assert!(text.contains(&format!("!define INSTALLIDENTITY \"{}\"", identity.key)));
        assert!(text.contains(r"Uninstall\DesktopBuddy-${INSTALLIDENTITY}"));
        assert!(!text.contains(r"Uninstall\${PRODUCTNAME}"));
        assert!(!text.contains("EnumRegKey"));
        assert!(text.contains("Call CheckBuddyInstallDirectory"));
        assert!(text.contains("${If} $0 = 1\n    CreateShortcut"));
        assert!(!text.contains("@BUDDY_"));
    }
    #[test]
    fn existing_builds_allow_renames_but_reject_identifier_reuse_and_slug_changes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut project = buddy_project::load_project(root.join("examples/typing-cat"))
            .unwrap()
            .data
            .project;
        let output = tempfile::tempdir().unwrap();
        let prior = output.path().join("previous");
        fs::create_dir(&prior).unwrap();
        fs::write(prior.join("build-report.json"), serde_json::to_vec(&serde_json::json!({"projectId":project.project_id,"identifier":project.product.identifier,"platformIdentity":{"binaryName":project.product.slug}})).unwrap()).unwrap();
        project.product.name = "Renamed".into();
        project.product.version = "2.0.0".into();
        check_known_products(output.path(), &project).unwrap();
        project.project_id = uuid::Uuid::new_v4().to_string();
        assert!(check_known_products(output.path(), &project)
            .unwrap_err()
            .contains("PRODUCT_IDENTITY_CONFLICT"));
        project.project_id = buddy_project::load_project(root.join("examples/typing-cat"))
            .unwrap()
            .data
            .project
            .project_id;
        project.product.slug = "different-slug".into();
        assert!(check_known_products(output.path(), &project)
            .unwrap_err()
            .contains("PRODUCT_SLUG_CHANGED"));
        project.product.identifier = "io.example.different".into();
        check_known_products(output.path(), &project).unwrap();
    }
}
