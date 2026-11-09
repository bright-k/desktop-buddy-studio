use buddy_runtime::LaunchAtLoginStatus;
use std::{
    ffi::{c_char, c_void, CStr},
    path::Path,
    ptr,
};

type Object = *mut c_void;
type Selector = *const c_void;

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {
    #[link_name = "OBJC_CLASS_$_SMAppService"]
    static SERVICE_CLASS: u8;
}

#[link(name = "objc")]
extern "C" {
    fn sel_registerName(name: *const c_char) -> Selector;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> Object;
    fn objc_autoreleasePoolPop(pool: Object);
}

struct AutoreleasePool(Object);

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

unsafe fn object_message(receiver: Object, selector: &CStr) -> Object {
    let send: unsafe extern "C" fn(Object, Selector) -> Object =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, sel_registerName(selector.as_ptr()))
}

unsafe fn service() -> Object {
    object_message(
        ptr::addr_of!(SERVICE_CLASS).cast_mut().cast(),
        c"mainAppService",
    )
}

unsafe fn service_status(service: Object) -> LaunchAtLoginStatus {
    let send: unsafe extern "C" fn(Object, Selector) -> isize =
        std::mem::transmute(objc_msgSend as *const ());
    // SMAppServiceStatus uses NSInteger and is declared in this order in SMAppService.h.
    match send(service, sel_registerName(c"status".as_ptr())) {
        // A freshly installed main bundle can report NotFound until its
        // first registration. Keep the switch available in that case.
        0 | 3 => LaunchAtLoginStatus::Off,
        1 => LaunchAtLoginStatus::On,
        2 => LaunchAtLoginStatus::NeedsApproval,
        _ => LaunchAtLoginStatus::Unavailable,
    }
}

pub struct Registration;

impl Registration {
    pub fn new(executable: &Path, _: &str) -> Option<Self> {
        let binary_folder = executable.parent()?;
        let contents = binary_folder.parent()?;
        let app = contents.parent()?;
        (binary_folder.ends_with("MacOS")
            && contents.ends_with("Contents")
            && app.extension().is_some_and(|extension| extension == "app")
            && !app.starts_with("/Volumes")
            && !app
                .components()
                .any(|part| part.as_os_str() == "AppTranslocation"))
        .then_some(Self)
    }

    pub fn status(&self) -> Result<LaunchAtLoginStatus, String> {
        unsafe {
            let _pool = AutoreleasePool(objc_autoreleasePoolPush());
            let service = service();
            if service.is_null() {
                return Err("macOS 로그인 항목에 접근할 수 없습니다.".into());
            }
            Ok(service_status(service))
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        unsafe {
            let _pool = AutoreleasePool(objc_autoreleasePoolPush());
            let service = service();
            if service.is_null() {
                return Err("macOS 로그인 항목에 접근할 수 없습니다.".into());
            }
            let matches_request = |status| match status {
                LaunchAtLoginStatus::On | LaunchAtLoginStatus::NeedsApproval => enabled,
                LaunchAtLoginStatus::Off => !enabled,
                _ => false,
            };
            if matches_request(service_status(service)) {
                return Ok(());
            }
            let selector = if enabled {
                c"registerAndReturnError:"
            } else {
                c"unregisterAndReturnError:"
            };
            let send: unsafe extern "C" fn(Object, Selector, *mut Object) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            let mut error = ptr::null_mut();
            if send(service, sel_registerName(selector.as_ptr()), &mut error) != 0
                || matches_request(service_status(service))
            {
                return Ok(());
            }
            if !error.is_null() {
                let description = object_message(error, c"localizedDescription");
                let text = object_message(description, c"UTF8String").cast::<c_char>();
                if !text.is_null() {
                    return Err(CStr::from_ptr(text).to_string_lossy().into_owned());
                }
            }
            Err("macOS 로그인 항목을 변경하지 못했습니다.".into())
        }
    }
}
