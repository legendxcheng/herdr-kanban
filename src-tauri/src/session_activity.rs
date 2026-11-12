//! In-flight writers are distinct from durable, historical unknown outcomes.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
type Key = (usize, String);
fn active() -> &'static Mutex<HashMap<Key, usize>> {
    static ACTIVE: OnceLock<Mutex<HashMap<Key, usize>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}
pub struct Guard(Key);
pub fn enter(app: &crate::workflow::Workflow, task: &str) -> Guard {
    let key = (Arc::as_ptr(&app.db) as usize, task.to_owned());
    *active().lock().unwrap().entry(key.clone()).or_default() += 1;
    Guard(key)
}
pub fn busy(app: &crate::workflow::Workflow, task: &str) -> bool {
    active()
        .lock()
        .unwrap()
        .contains_key(&(Arc::as_ptr(&app.db) as usize, task.to_owned()))
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut a = active().lock().unwrap();
        if let Some(n) = a.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                a.remove(&self.0);
            }
        }
    }
}
