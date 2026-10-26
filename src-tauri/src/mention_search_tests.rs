use super::*;
use std::sync::atomic::AtomicUsize;

#[test]
fn cached_queries_still_reject_disabled_machines_or_changed_project_roots() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute("INSERT INTO machines(id,name,kind,session,host_identity) VALUES('remote','Remote','ssh','default','host-a')", []).unwrap();
    db.execute("INSERT INTO projects(id,name,root_path,machine_id,created_at,herdr_workspace) VALUES('p','Project','C:/root','remote','now','workspace')", []).unwrap();
    let db = Arc::new(Mutex::new(db));
    let first = target(&db, Some("p"), "C:/root").unwrap().unwrap();
    let mut other = first.clone();
    other.session = "other".into();
    assert_ne!(identity(&first), identity(&other));
    other = first.clone();
    other.host_identity = Some("host-b".into());
    assert_ne!(identity(&first), identity(&other));
    assert_eq!(
        target(&db, Some("p"), "C:/stale").unwrap_err(),
        "mention.project_changed"
    );
    db.lock()
        .unwrap()
        .execute("UPDATE machines SET disabled=1 WHERE id='remote'", [])
        .unwrap();
    assert_eq!(
        target(&db, Some("p"), "C:/root").unwrap_err(),
        "machine.disabled"
    );
}

#[test]
fn concurrent_keywords_share_one_snapshot_and_other_scopes_remain_independent() {
    let cache = Arc::new(Cache::<Vec<String>>::default());
    let loads = Arc::new(AtomicUsize::new(0));
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let first_cache = cache.clone();
    let first_loads = loads.clone();
    let first = std::thread::spawn(move || {
        first_cache
            .get("machine-a/project-a".into(), false, || {
                first_loads.fetch_add(1, Ordering::Relaxed);
                started.send(()).unwrap();
                wait_release.recv().unwrap();
                Ok(vec!["all-candidates".into()])
            })
            .unwrap()
    });
    wait_started.recv().unwrap();
    let second_cache = cache.clone();
    let second_loads = loads.clone();
    let second = std::thread::spawn(move || {
        second_cache
            .get("machine-a/project-a".into(), false, || {
                second_loads.fetch_add(1, Ordering::Relaxed);
                Ok(vec![])
            })
            .unwrap()
    });
    assert_eq!(
        *cache
            .get("machine-b/project-a".into(), false, || Ok(vec![
                "other-machine".into()
            ]))
            .unwrap(),
        ["other-machine"]
    );
    release.send(()).unwrap();
    assert_eq!(*first.join().unwrap(), *second.join().unwrap());
    assert_eq!(loads.load(Ordering::Relaxed), 1);
}

#[test]
fn refresh_and_expiry_reload_and_late_old_scan_cannot_overwrite_refresh() {
    let cache = Arc::new(Cache::<usize>::default());
    assert_eq!(*cache.get("scope".into(), false, || Ok(1)).unwrap(), 1);
    assert_eq!(*cache.get("scope".into(), false, || Ok(2)).unwrap(), 1);
    assert_eq!(*cache.get("scope".into(), true, || Ok(3)).unwrap(), 3);
    cache.entries.lock().unwrap()["scope"]
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .0 -= TTL;
    assert_eq!(*cache.get("scope".into(), false, || Ok(4)).unwrap(), 4);
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let old_cache = cache.clone();
    let old = std::thread::spawn(move || {
        old_cache.get("scope".into(), true, || {
            started.send(()).unwrap();
            wait_release.recv().unwrap();
            Ok(5)
        })
    });
    wait_started.recv().unwrap();
    assert_eq!(*cache.get("scope".into(), true, || Ok(6)).unwrap(), 6);
    release.send(()).unwrap();
    old.join().unwrap().unwrap();
    assert_eq!(*cache.get("scope".into(), false, || Ok(7)).unwrap(), 6);
}

#[test]
fn errors_are_not_cached_as_empty_results_and_retry_can_recover() {
    let cache = Cache::<usize>::default();
    assert_eq!(
        cache
            .get("scope".into(), false, || Err("offline".into()))
            .unwrap_err(),
        "offline"
    );
    assert_eq!(
        cache.get("scope".into(), false, || Ok(1)).unwrap_err(),
        "offline"
    );
    assert_eq!(*cache.get("scope".into(), true, || Ok(2)).unwrap(), 2);
    for n in 0..20 {
        cache.get(n.to_string(), false, || Ok(n)).unwrap();
    }
    assert!(cache.entries.lock().unwrap().len() <= CAPACITY);
}
