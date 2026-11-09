//! A setup result may publish only while its original machine connection attempt is current.
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Attempts {
    active: HashMap<String, Arc<AtomicBool>>,
    stopped: bool,
}
fn attempts() -> &'static Mutex<Attempts> {
    static ATTEMPTS: OnceLock<Mutex<Attempts>> = OnceLock::new();
    ATTEMPTS.get_or_init(Default::default)
}
pub(super) struct Attempt {
    profile: String,
    pub cancelled: Arc<AtomicBool>,
}
impl Attempt {
    // Call under the database lock, as for disable and final publication.
    pub fn begin(profile: &str) -> Result<Self, String> {
        let mut attempts = attempts().lock().map_err(|e| e.to_string())?;
        if attempts.stopped {
            return Err("machine.cancelled".into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Some(old) = attempts.active.insert(profile.into(), cancelled.clone()) {
            old.store(true, Ordering::Release);
        }
        Ok(Self {
            profile: profile.into(),
            cancelled,
        })
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("machine.cancelled".into())
        } else {
            Ok(())
        }
    }
    pub fn publish<T>(&self, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let attempts = attempts().lock().map_err(|e| e.to_string())?;
        self.check()?;
        if attempts.stopped
            || !attempts
                .active
                .get(&self.profile)
                .is_some_and(|p| Arc::ptr_eq(p, &self.cancelled))
        {
            return Err("machine.cancelled".into());
        }
        action()
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if let Ok(mut attempts) = attempts().lock() {
            if attempts
                .active
                .get(&self.profile)
                .is_some_and(|p| Arc::ptr_eq(p, &self.cancelled))
            {
                attempts.active.remove(&self.profile);
            }
        }
    }
}
pub(super) fn cancel(profile: &str) {
    if let Some(attempt) = attempts().lock().unwrap().active.remove(profile) {
        attempt.store(true, Ordering::Release);
    }
}
pub(super) fn shutdown() {
    let mut attempts = attempts().lock().unwrap();
    attempts.stopped = true;
    for (_, attempt) in attempts.active.drain() {
        attempt.store(true, Ordering::Release);
    }
}
