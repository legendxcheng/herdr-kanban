use super::*;

#[test]
fn a_pane_cannot_receive_another_projects_task() {
    let (app, external) = fixture();
    assert_eq!(
        app.dispatch("a", "w1:p1", "Task A", "op-a")
            .unwrap()
            .target_agent,
        "w1:p1"
    );
    assert_eq!(
        app.dispatch("b", "w1:p1", "Task B", "op-b").unwrap_err(),
        "binding.occupied"
    );
    assert_eq!(*external.sent.lock().unwrap(), vec!["Task A"]);
}

#[test]
fn a_replaced_agent_cannot_be_used_until_explicit_repair() {
    let (app, external) = fixture();
    app.dispatch("a", "w1:p1", "A", "op").unwrap();
    *external.replaced.lock().unwrap() = true;
    assert_eq!(
        app.target("a", true).unwrap_err(),
        "binding.identity_changed"
    );
    assert_eq!(*external.sent.lock().unwrap(), vec!["A"]);
}

#[test]
fn concurrent_projects_cannot_both_dispatch_to_one_pane() {
    let (app, external) = fixture();
    let app = Arc::new(app);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|task| {
            let app = app.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                app.dispatch(task, "w1:p1", task, task).is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(external.sent.lock().unwrap().len(), 1);
}

#[test]
fn legacy_shared_targets_require_explicit_unique_repairs_without_sending() {
    let (app, external) = fixture();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='in_progress',dispatched_to='one/w1:p1'",
            [],
        )
        .unwrap();
    assert_eq!(app.binding("a").unwrap().state, "repair");
    assert_eq!(app.binding("b").unwrap().state, "repair");
    assert!(app.target("a", true).is_err());
    app.repair("a", "w1:p1").unwrap();
    assert_eq!(app.repair("b", "w1:p1").unwrap_err(), "binding.occupied");
    app.repair("b", "w1:p2").unwrap();
    assert_eq!(app.target("a", true).unwrap().pane_id, "w1:p1");
    assert_eq!(app.target("b", true).unwrap().pane_id, "w1:p2");
    assert!(external.sent.lock().unwrap().is_empty());
}
