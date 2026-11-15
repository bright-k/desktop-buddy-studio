#[cfg(windows)]
use std::ffi::OsString;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) fn run(
    command: &mut Command,
    cancellation: &CancellationToken,
    log: &mut impl FnMut(&str),
) -> Result<(), String> {
    if cancellation.is_cancelled() {
        return Err("BUILD_CANCELLED".into());
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CI", "true");
    crate::configure_toolchain_environment(command)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000200);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("TOOL_START_FAILED: {e}"))?;
    let (sender, receiver) = mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let second = sender.clone();
    let stdout = std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            let _ = sender.send(line);
        }
    });
    let stderr = std::thread::spawn(move || {
        for line in BufReader::new(err).lines() {
            let Ok(line) = line else { break };
            let _ = second.send(line);
        }
    });
    let status = loop {
        while let Ok(line) = receiver.try_recv() {
            log(&line);
        }
        if cancellation.is_cancelled() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill.exe")
                    .args([
                        OsString::from("/PID"),
                        child.id().to_string().into(),
                        "/T".into(),
                        "/F".into(),
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            for _ in 0..20 {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
            return Err("BUILD_CANCELLED".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = stdout.join();
    let _ = stderr.join();
    while let Ok(line) = receiver.try_recv() {
        log(&line);
    }
    if !status.success() {
        return Err(format!("TOOL_FAILED: process exited with {status}"));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_terminates_child_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let ready = pid_file.clone();
        let token = CancellationToken::default();
        let cancel = token.clone();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !ready.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            cancel.cancel();
        });
        let start = std::time::Instant::now();
        let result = run(
            Command::new("/bin/sh")
                .args([
                    "-c",
                    "sleep 20 & child=$!; echo \"$child\" > \"$1\"; wait",
                    "buddy-cancel-test",
                ])
                .arg(&pid_file),
            &token,
            &mut |_| {},
        );
        assert_eq!(result.unwrap_err(), "BUILD_CANCELLED");
        assert!(start.elapsed() < Duration::from_secs(3));
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let status = Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&status.stdout);
            if state.trim().is_empty() || state.trim_start().starts_with('Z') {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "descendant remains alive after cancellation"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
