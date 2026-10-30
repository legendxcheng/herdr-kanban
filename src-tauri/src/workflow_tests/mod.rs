use super::*;

#[derive(Default)]
struct FakeHerdr {
    machine_name: Option<String>,
    sent: Mutex<Vec<String>>,
    uncertain: Mutex<bool>,
    send_error_message: Mutex<Option<String>>,
    unavailable: Mutex<bool>,
    replaced: Mutex<bool>,
    after_send: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
impl Herdr for FakeHerdr {
    fn machine_id(&self) -> &str {
        self.machine_name.as_deref().unwrap_or("local")
    }
    fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
        Ok(vec![
            Target {
                machine_id: self.machine_id().into(),
                session: "default".into(),
                workspace_id: "w1".into(),
                pane_id: "w1:p1".into(),
                terminal_id: "term-a".into(),
                agent: "codex".into(),
                process_id: "100".into(),
                codex_resume_locks: Vec::new(),
                title: "Agent A".into(),
                status: "idle".into(),
            },
            Target {
                machine_id: self.machine_id().into(),
                session: "default".into(),
                workspace_id: "w1".into(),
                pane_id: "w1:p2".into(),
                terminal_id: "term-b".into(),
                agent: "codex".into(),
                process_id: "200".into(),
                codex_resume_locks: Vec::new(),
                title: "Agent B".into(),
                status: "idle".into(),
            },
        ])
    }
    fn validate(&self, workspace: &str, expected: &Target) -> Result<Target, String> {
        if *self.replaced.lock().unwrap() {
            return Err("binding.identity_changed".into());
        }
        self.targets(workspace)?
            .into_iter()
            .find(|t| t.same_identity(expected))
            .ok_or("binding.missing".into())
    }
    fn send(&self, _: &Target, prompt: &str) -> Result<(), SendError> {
        if *self.unavailable.lock().unwrap() {
            return Err(SendError {
                message: "herdr.unavailable".into(),
                uncertain: false,
            });
        }
        self.sent.lock().unwrap().push(prompt.into());
        if let Some(callback) = self.after_send.lock().unwrap().as_ref() {
            callback();
        }
        if *self.uncertain.lock().unwrap() {
            Err(SendError {
                message: self
                    .send_error_message
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| "timeout".into()),
                uncertain: true,
            })
        } else {
            Ok(())
        }
    }
}

#[test]
fn same_pane_on_two_computers_can_be_bound_without_cross_machine_ownership() {
    let (local, _) = fixture();
    local.db.lock().unwrap().execute_batch("DELETE FROM tasks WHERE id='b'; UPDATE projects SET machine_id='devpc' WHERE id='p2'; INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('b','p2','B','todo','now','now');").unwrap();
    let remote = Workflow {
        db: local.db.clone(),
        herdr: Arc::new(FakeHerdr {
            machine_name: Some("devpc".into()),
            ..Default::default()
        }),
    };
    local.repair("a", "w1:p1").unwrap();
    remote.repair("b", "w1:p1").unwrap();
    let first = local.binding("a").unwrap().target.unwrap();
    let second = remote.binding("b").unwrap().target.unwrap();
    assert_eq!(first.machine_id, "local");
    assert_eq!(second.machine_id, "devpc");
    assert!(!first.same_identity(&second));
    assert_eq!(
        pane_owner(&local.db.lock().unwrap(), &first)
            .unwrap()
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        pane_owner(&remote.db.lock().unwrap(), &second)
            .unwrap()
            .as_deref(),
        Some("b")
    );
}
fn fixture() -> (Workflow, Arc<FakeHerdr>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p1','One','.','one','now'),('p2','Two','.','one','now');
        INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('a','p1','A','todo','now','now'),('b','p2','B','todo','now','now');").unwrap();
    // Historical duplicate association fixture keeps testing cross-project pane ownership.
    db.execute_batch("DROP INDEX project_workspace_identity; UPDATE projects SET herdr_session='default',workspace_id='w1',sync_status='ready';").unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    (
        Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: herdr.clone(),
        },
        herdr,
    )
}

mod dispatch_recovery;
mod live;
mod ssh_submit;
mod ownership;
mod task_mutations;
