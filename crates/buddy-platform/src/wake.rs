use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

/// Shared wake flag prevents lost wakeups between draining events and waiting.
#[derive(Clone, Default)]
pub struct WakeSignal(Arc<(Mutex<bool>, Condvar)>);
impl WakeSignal {
    pub fn wake(&self) {
        let (flag, condition) = &*self.0;
        *flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
        condition.notify_one();
    }
    pub fn wait(&self, timeout: Option<Duration>) {
        let (flag, condition) = &*self.0;
        let mut flag = flag.lock().unwrap_or_else(|p| p.into_inner());
        if !*flag {
            flag = match timeout {
                Some(duration) => {
                    condition
                        .wait_timeout(flag, duration)
                        .unwrap_or_else(|p| p.into_inner())
                        .0
                }
                None => condition.wait(flag).unwrap_or_else(|p| p.into_inner()),
            };
        }
        *flag = false;
    }
}
