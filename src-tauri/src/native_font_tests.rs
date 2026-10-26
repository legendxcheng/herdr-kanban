use super::*;
use std::io::Write;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("kanban-font-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn view(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn saved(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.0.join("font-preference.json")).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn append(directory: &Path, text: &str) {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(directory.join("font-events.jsonl"))
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

#[test]
fn resuming_a_retained_view_skips_stale_journal_but_records_new_adjustments() {
    let f = Fixture::new();
    let a = f.view("a");
    let b = f.view("b");
    let mut store = Preferences::default();
    store.activate("a", &f.0, &a).unwrap();
    append(&a, "15\n");
    store.finish("a");
    store.activate("b", &f.0, &b).unwrap();
    append(&b, "17\n");
    store.finish("b");
    append(&a, "9\n");
    store.resume("a", &a).unwrap();
    store.poll("a");
    assert_eq!(f.saved()["font_size"].as_f64(), Some(17.0));
    append(&a, "18\n");
    store.poll("a");
    assert_eq!(f.saved()["font_size"].as_f64(), Some(18.0));
}

#[test]
fn switch_flushes_last_adjustment_and_rejects_old_window_writes() {
    let f = Fixture::new();
    let a = f.view("a");
    let b = f.view("b");
    let mut store = Preferences::default();
    store.activate("a", &f.0, &a).unwrap();
    append(&a, "14.3\n15.73\n");
    // Switching before the next monitor tick must still persist the last key.
    store.finish("a");
    assert_eq!(f.saved()["font_size"], 15.73);
    store.activate("b", &f.0, &b).unwrap();
    append(&b, "17.303\n");
    store.poll("b");
    append(&a, "9\n");
    store.poll("a");
    store.finish("a");
    assert_eq!(f.saved()["font_size"], 17.303);
    append(&b, "null\n");
    store.finish("b");
    assert!(f.saved()["font_size"].is_null());
    // A fresh manager represents an application restart. No startup default write.
    drop(store);
    let mut restarted = Preferences::default();
    restarted.activate("c", &f.0, &f.view("c")).unwrap();
    restarted.poll("c");
    assert!(f.saved()["font_size"].is_null());
}

#[test]
fn incomplete_and_invalid_records_cannot_replace_the_last_saved_size() {
    let f = Fixture::new();
    let view = f.view("a");
    let mut store = Preferences::default();
    store.activate("a", &f.0, &view).unwrap();
    append(&view, "16\n17.");
    store.poll("a");
    assert_eq!(f.saved()["font_size"], 16.0);
    append(&view, "6\nNaN\n-2\n100000\n{}\n");
    store.poll("a");
    assert_eq!(f.saved()["font_size"], 17.6);
    store.poll("a");
    assert_eq!(f.saved()["font_size"], 17.6);
}

#[test]
fn failed_publish_retries_without_consuming_the_adjustment() {
    let f = Fixture::new();
    let view = f.view("a");
    let mut store = Preferences::default();
    store.activate("a", &f.0, &view).unwrap();
    fs::create_dir(f.0.join("font-preference.tmp")).unwrap();
    append(&view, "18\n");
    assert!(store.flush().is_err());
    fs::remove_dir(f.0.join("font-preference.tmp")).unwrap();
    store.flush().unwrap();
    assert_eq!(f.saved()["font_size"], 18.0);
}

#[test]
fn config_expression_preserves_unicode_paths_and_lua_delimiters() {
    let f = Fixture::new();
    let view = f.view("字体 ' ]=] test");
    let expression = Preferences::default().activate("a", &f.0, &view).unwrap();
    assert!(expression.contains("[==["));
    assert!(expression.contains("字体 ' ]=] test"));
    assert!(!expression.contains("--config-file"));
}
