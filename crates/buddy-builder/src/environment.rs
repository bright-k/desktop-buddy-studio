use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostEnvironment {
    pub os: &'static str,
    pub architecture: &'static str,
    pub version: Option<String>,
    pub meets_support_baseline: bool,
}
pub fn host_environment() -> HostEnvironment {
    let mut result = HostEnvironment {
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        version: None,
        meets_support_baseline: false,
    };
    #[cfg(target_os = "macos")]
    {
        result.version = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
        result.meets_support_baseline = result.architecture == "aarch64"
            && result
                .version
                .as_ref()
                .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
                .is_some_and(|v| v >= 15);
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::{
            Wdk::System::SystemServices::RtlGetVersion,
            Win32::System::SystemInformation::{OSVERSIONINFOEXW, OSVERSIONINFOW},
        };
        let mut info: OSVERSIONINFOEXW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOEXW>() as u32;
        if RtlGetVersion((&mut info as *mut OSVERSIONINFOEXW).cast::<OSVERSIONINFOW>()) >= 0 {
            result.version = Some(format!(
                "{}.{}.{}",
                info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
            ));
            result.meets_support_baseline = result.architecture == "x86_64"
                && info.wProductType == 1
                && info.dwMajorVersion == 10
                && info.dwBuildNumber >= 26200;
        }
    }
    result
}
