use buddy_runtime::PreferenceOverrides;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const PREFERENCES_LIMIT: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    schema_version: u32,
    overrides: PreferenceOverrides,
}
pub struct PreferencesStore {
    path: PathBuf,
    pub read_only: bool,
    pub notice: Option<String>,
}
impl PreferencesStore {
    pub fn open(directory: &Path) -> Result<(Self, PreferenceOverrides), String> {
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let path = directory.join("preferences.json");
        let mut store = Self {
            path,
            read_only: false,
            notice: None,
        };
        if !store.path.exists() {
            return Ok((store, PreferenceOverrides::default()));
        }
        let bytes = read_preferences(&store.path)?;
        if bytes.len() as u64 > PREFERENCES_LIMIT {
            store.read_only = true;
            store.notice = Some("설정 파일이 1 MiB를 초과해 원본과 백업을 보존하고 입력을 끈 상태로 시작했습니다. 설정 초기화로 복구할 수 있습니다.".into());
            return Ok((store, disabled()));
        }
        let raw: Result<serde_json::Value, _> =
            buddy_project::parse_json(&bytes, "preferences.json");
        if raw
            .as_ref()
            .ok()
            .and_then(|v| v.get("schemaVersion"))
            .and_then(|v| v.as_u64())
            .is_some_and(|v| v > 1)
        {
            store.read_only = true;
            store.notice=Some("새 버전 설정을 보존하고 읽기 전용으로 시작했습니다. 초기화 전에는 저장하지 않습니다.".into());
            return Ok((store, disabled()));
        }
        if let Ok(document) = read_document(&bytes) {
            return Ok((store, document.overrides));
        }
        let preserved = store
            .path
            .with_extension(format!("corrupt-{}.json", uuid::Uuid::new_v4()));
        fs::copy(&store.path, &preserved).map_err(|e| e.to_string())?;
        if let Ok(backup) = read_preferences(&store.path.with_extension("backup.json")) {
            if let Ok(document) = read_document(&backup) {
                store.notice = Some(
                    "손상된 설정을 보존하고 직전 정상 백업을 읽었습니다. 저장하면 복구됩니다."
                        .into(),
                );
                return Ok((store, document.overrides));
            }
        }
        store.read_only = true;
        store.notice = Some(
            "설정과 백업을 읽지 못해 입력을 끄고 시작했습니다. 설정 초기화로 복구할 수 있습니다."
                .into(),
        );
        Ok((store, disabled()))
    }
    pub fn save(&mut self, overrides: &PreferenceOverrides) -> Result<(), String> {
        if self.read_only {
            return Err("현재 설정은 보존 모드입니다. 명시적인 설정 초기화가 필요합니다.".into());
        }
        if let Ok(bytes) = read_preferences(&self.path) {
            if read_document(&bytes).is_ok() {
                atomic_write(&self.path.with_extension("backup.json"), &bytes)?;
            }
        }
        let bytes = serde_json::to_vec_pretty(&Document {
            schema_version: 1,
            overrides: overrides.clone(),
        })
        .map_err(|e| e.to_string())?;
        read_document(&bytes)?;
        atomic_write(&self.path, &bytes)?;
        self.notice = None;
        Ok(())
    }
    pub fn reset(&mut self) -> Result<PreferenceOverrides, String> {
        if self.path.exists() {
            fs::copy(
                &self.path,
                self.path
                    .with_extension(format!("reset-{}.json", uuid::Uuid::new_v4())),
            )
            .map_err(|e| e.to_string())?;
        }
        let was_read_only = self.read_only;
        self.read_only = false;
        let overrides = disabled();
        if let Err(error) = self.save(&overrides) {
            self.read_only = was_read_only;
            return Err(error);
        }
        Ok(overrides)
    }
}
fn read_preferences(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(PREFERENCES_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn disabled() -> PreferenceOverrides {
    PreferenceOverrides {
        keyboard: Some(false),
        mouse: Some(false),
        cursor_follow: Some(false),
        ..Default::default()
    }
}
fn read_document(bytes: &[u8]) -> Result<Document, String> {
    let document: Document =
        buddy_project::parse_json(bytes, "preferences.json").map_err(|e| e.to_string())?;
    if document.schema_version != 1
        || document.overrides.timing.is_some_and(|t| !t.is_valid())
        || document
            .overrides
            .scale
            .is_some_and(|s| !(0.25..=3.0).contains(&s))
        || document
            .overrides
            .base_placement
            .as_ref()
            .is_some_and(|p| !p.position.x.is_finite() || !p.position.y.is_finite())
    {
        return Err("Invalid preferences".into());
    }
    Ok(document)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_preferences_preserve_main_and_backup_and_start_with_input_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = PreferencesStore::open(directory.path()).unwrap();
        store.save(&PreferenceOverrides::default()).unwrap();
        store
            .save(&PreferenceOverrides {
                scale: Some(1.5),
                ..Default::default()
            })
            .unwrap();
        let backup_path = directory.path().join("preferences.backup.json");
        let backup = fs::read(&backup_path).unwrap();
        let oversized = vec![b' '; PREFERENCES_LIMIT as usize + 500];
        fs::write(&store.path, &oversized).unwrap();
        assert_eq!(
            read_preferences(&store.path).unwrap().len(),
            PREFERENCES_LIMIT as usize + 1
        );
        let (mut reopened, values) = PreferencesStore::open(directory.path()).unwrap();
        assert!(reopened.read_only);
        assert_eq!((values.keyboard, values.mouse), (Some(false), Some(false)));
        assert!(reopened.notice.as_ref().unwrap().contains("1 MiB"));
        assert!(reopened.save(&PreferenceOverrides::default()).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), oversized);
        assert_eq!(fs::read(backup_path).unwrap(), backup);
        let reset = reopened.reset().unwrap();
        assert!(!reopened.read_only);
        assert_eq!((reset.keyboard, reset.mouse), (Some(false), Some(false)));
        assert!(fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("preferences.reset-")));
    }

    #[test]
    fn backup_write_failure_preserves_current_preferences_and_unrelated_files() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = PreferencesStore::open(directory.path()).unwrap();
        store
            .save(&PreferenceOverrides {
                scale: Some(1.25),
                ..Default::default()
            })
            .unwrap();
        let before = fs::read(&store.path).unwrap();
        let blocked_backup = store.path.with_extension("backup.json");
        fs::create_dir(&blocked_backup).unwrap();
        fs::write(blocked_backup.join("keep"), "unrelated file").unwrap();
        assert!(store
            .save(&PreferenceOverrides {
                scale: Some(2.0),
                ..Default::default()
            })
            .is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert_eq!(
            fs::read_to_string(blocked_backup.join("keep")).unwrap(),
            "unrelated file"
        );
        assert_eq!(read_document(&before).unwrap().overrides.scale, Some(1.25));
        assert!(!fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".tmp-")));
    }
    #[test]
    fn future_settings_are_preserved_until_explicit_reset() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"{\"schemaVersion\":99,\"future\":true}";
        fs::write(dir.path().join("preferences.json"), bytes).unwrap();
        let (mut store, values) = PreferencesStore::open(dir.path()).unwrap();
        assert_eq!(values.keyboard, Some(false));
        assert!(store.save(&values).is_err());
        assert_eq!(
            fs::read(dir.path().join("preferences.json")).unwrap(),
            bytes
        );
        store.reset().unwrap();
        assert!(!store.read_only);
    }
    #[test]
    fn explicit_values_survive_and_corruption_recovers_from_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = PreferencesStore::open(dir.path()).unwrap();
        let first = PreferenceOverrides {
            scale: Some(1.0),
            keyboard: Some(false),
            sound_enabled: Some(false),
            ..Default::default()
        };
        store.save(&first).unwrap();
        let second = PreferenceOverrides {
            scale: Some(2.0),
            ..first.clone()
        };
        store.save(&second).unwrap();
        fs::write(dir.path().join("preferences.json"), "broken").unwrap();
        let (store, recovered) = PreferencesStore::open(dir.path()).unwrap();
        assert_eq!(recovered.scale, Some(1.0));
        assert_eq!(recovered.keyboard, Some(false));
        assert_eq!(recovered.sound_enabled, Some(false));
        assert!(store.notice.is_some());
    }
}
