use buddy_runtime::LaunchAtLoginStatus;
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        ERROR_UNSUPPORTED_TYPE,
    },
    System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    },
};

const RUN_KEY: *const u16 = windows_sys::w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
// Run values accept at most 260 UTF-16 units for the command line, plus the NUL.
const COMMAND_CAPACITY: usize = 261;

pub struct Registration {
    value: Vec<u16>,
    command: Vec<u16>,
}

impl Registration {
    pub fn new(executable: &Path, identifier: &str) -> Option<Self> {
        if !executable.with_file_name("uninstall.exe").is_file() {
            return None;
        }
        // Matches the per-product value removed by the NSIS uninstaller.
        let value = format!(
            "DesktopBuddy-{}",
            buddy_project::sha256(identifier.as_bytes())
        )
        .encode_utf16()
        .chain([0])
        .collect();
        let quote = u16::from(b'"');
        let mut command = vec![quote];
        command.extend(executable.as_os_str().encode_wide());
        command.extend([quote, 0]);
        Some(Self { value, command })
    }

    pub fn status(&self) -> Result<LaunchAtLoginStatus, String> {
        let mut data = [0u16; COMMAND_CAPACITY];
        let mut size = std::mem::size_of_val(&data) as u32;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                self.value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match result {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(LaunchAtLoginStatus::Off),
            ERROR_SUCCESS if data.get(..size as usize / 2) == Some(self.command.as_slice()) => {
                Ok(LaunchAtLoginStatus::On)
            }
            ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_UNSUPPORTED_TYPE => {
                Ok(LaunchAtLoginStatus::PathChanged)
            }
            code => Err(registry_error(code)),
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        if !enabled {
            let result =
                unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, self.value.as_ptr()) };
            return match result {
                ERROR_SUCCESS | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(()),
                code => Err(registry_error(code)),
            };
        }
        if self.command.len() > COMMAND_CAPACITY {
            return Err(
                "설치 경로가 너무 깁니다. 더 짧은 경로에 설치한 뒤 자동 실행을 켜세요.".into(),
            );
        }
        unsafe {
            let mut key = ptr::null_mut();
            let opened = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            );
            if opened != ERROR_SUCCESS {
                return Err(registry_error(opened));
            }
            let written = RegSetValueExW(
                key,
                self.value.as_ptr(),
                0,
                REG_SZ,
                self.command.as_ptr().cast(),
                std::mem::size_of_val(self.command.as_slice()) as u32,
            );
            RegCloseKey(key);
            if written != ERROR_SUCCESS {
                return Err(registry_error(written));
            }
        }
        Ok(())
    }
}

fn registry_error(code: u32) -> String {
    format!(
        "Windows 시작 앱 등록에 접근하지 못했습니다. {}",
        io::Error::from_raw_os_error(code as i32)
    )
}
