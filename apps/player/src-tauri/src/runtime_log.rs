use fs2::FileExt;
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 256 * 1024;
static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn install(directory: Option<PathBuf>) {
    if let Some(directory) = directory {
        if fs::create_dir_all(&directory).is_ok() {
            let _ = PATH.set(directory.join("player.log"));
        }
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let thread = std::thread::current();
        if let Some(location) = panic.location() {
            write(format_args!(
                "Panic in {} at {}:{}",
                thread.name().unwrap_or("unnamed thread"),
                location.file(),
                location.line()
            ));
        }
        default_hook(panic);
    }));
}

pub fn write(message: fmt::Arguments<'_>) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format_args!("[{millis} pid={}] {message}", std::process::id()).to_string();
    let _ = writeln!(io::stderr().lock(), "{line}");
    let Some(path) = PATH.get() else { return };
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
    {
        if FileExt::try_lock_exclusive(&file).is_ok() {
            if file
                .metadata()
                .is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES)
            {
                let _ = file.set_len(0);
            }
            let _ = writeln!(file, "{line}");
        }
    }
}
