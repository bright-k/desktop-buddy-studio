use buddy_runtime::LaunchAtLoginStatus;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as native;

#[cfg(not(any(target_os = "macos", windows)))]
mod native {
    use super::LaunchAtLoginStatus;
    use std::path::Path;

    pub struct Registration;

    impl Registration {
        pub fn new(_: &Path, _: &str) -> Option<Self> {
            None
        }

        pub fn status(&self) -> Result<LaunchAtLoginStatus, String> {
            Ok(LaunchAtLoginStatus::Unavailable)
        }

        pub fn set_enabled(&self, _: bool) -> Result<(), String> {
            Err("이 운영체제에서는 자동 실행을 설정할 수 없습니다.".into())
        }
    }
}

#[derive(Default)]
pub struct LaunchAtLogin(Option<native::Registration>);

impl LaunchAtLogin {
    pub fn new(bundled_project: bool, identifier: &str) -> Self {
        let registration = if bundled_project {
            std::env::current_exe()
                .ok()
                .and_then(|exe| native::Registration::new(&exe, identifier))
        } else {
            None
        };
        Self(registration)
    }

    pub fn status(&self) -> LaunchAtLoginStatus {
        self.0
            .as_ref()
            .and_then(|registration| registration.status().ok())
            .unwrap_or(LaunchAtLoginStatus::Unavailable)
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        match &self.0 {
            Some(registration) => registration.set_enabled(enabled),
            None if !enabled => Ok(()),
            None => Err("설치된 앱에서 자동 실행을 설정하세요.".into()),
        }
    }
}
