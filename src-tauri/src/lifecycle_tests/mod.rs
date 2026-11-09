use super::*;
mod images;
use crate::workflow::{Herdr, SendError};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    remote: bool,
    panes: BTreeMap<String, (Target, Option<String>)>,
    created: usize,
    prepared: Vec<Option<String>>,
    settings: Vec<crate::codex_settings::Settings>,
    catalog_error: bool,
    catalog_directories: Vec<String>,
    directories: Vec<String>,
    sent: Vec<String>,
    launch_prompts: Vec<String>,
    launches: BTreeMap<String, Target>,
    startup_failure: Option<String>,
    closed: Vec<String>,
    missing_id: bool,
    close_error: bool,
    close_then_error: bool,
    prepare_error: bool,
    prepare_busy: bool,
    mismatch: bool,
    send_error: bool,
    before_input_error: Option<String>,
    first_prompt_states: Option<Vec<serde_json::Value>>,
    input_requests: Vec<Vec<String>>,
    receipt_available: bool,
    early_receipt: bool,
    receipt_checks: usize,
    lookup_error: Option<String>,
    create_error: bool,
    lost_create_response: bool,
    prepare_gate: Option<Arc<(Mutex<bool>, std::sync::Condvar)>>,
    observe: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}
#[derive(Default)]
struct Fake(Mutex<State>, Option<Arc<dyn crate::execution::Resources>>);
impl Fake {
    fn observe(&self, phase: &str) {
        let callback = self.0.lock().unwrap().observe.clone();
        if let Some(callback) = callback {
            callback(phase);
        }
    }
}
fn target(id: &str) -> Target {
    Target {
        machine_id: crate::machines::local_id(),
        session: "default".into(),
        workspace_id: "w1".into(),
        pane_id: id.into(),
        terminal_id: format!("term-{id}"),
        agent: "codex".into(),
        process_id: format!("pid-{id}"),
        codex_resume_locks: Vec::new(),
        title: id.into(),
        status: "idle".into(),
    }
}
impl Herdr for Fake {
    fn launch_prompt(&self, request: &crate::startup_prompt::Request, check: &dyn Fn()->Result<(),String>) -> Result<(),String> {
        self.observe("launching");
        check()?;
        let live = self.prepare_codex_configured(&request.target,&request.cwd,None,&request.operation,&request.settings,check)?;
        check()?;
        let mut state = self.0.lock().unwrap();
        state.launch_prompts.push(request.prompt.clone());
        state.sent.push(request.prompt.clone());
        state.launches.insert(request.operation.clone(),live);
        let failed = state.send_error;
        if !failed { state.receipt_available=true; }
        drop(state);
        self.observe("sent");
        if failed { Err("timeout".into()) } else { Ok(()) }
    }
    fn find_prompt_launch(&self, request: &crate::startup_prompt::Request) -> Result<Option<Target>,String> {
        let s = self.0.lock().unwrap();
        if let Some(error)=&s.startup_failure { return Err(error.clone()); }
        if let Some(error) = &s.lookup_error { return Err(error.clone()); }
        let Some(expected) = s.launches.get(&request.operation) else { return Ok(None); };
        let live = s.panes.get(&expected.pane_id).ok_or("binding.missing")?.0.clone();
        if !live.same_identity(expected) { return Err("binding.identity_changed".into()); }
        Ok(Some(live))
    }
    fn workspaces(&self) -> Result<Vec<crate::projects::Workspace>, String> {
        if let Some(error) = self.0.lock().unwrap().lookup_error.clone() { return Err(error); }
        Ok(vec![crate::projects::Workspace { id: "w1".into(), name: "test".into(), directory: Some(".".into()) }])
    }
    fn machine_id(&self) -> &str {
        if self.0.lock().unwrap().remote {
            "ssh-test"
        } else {
            crate::machines::LOCAL_ID
        }
    }
    fn codex_catalog(&self, cwd: &str) -> Result<crate::codex_catalog::Catalog, String> {
        let mut state = self.0.lock().unwrap();
        state.catalog_directories.push(cwd.into());
        if state.catalog_error {
            return Err("model.catalog_unavailable".into());
        }
        Ok(crate::codex_catalog::Catalog {
            models: vec![crate::codex_catalog::Model {
                id: "test-model".into(),
                name: "Test".into(),
                efforts: vec!["high".into(), "low".into()],
            }],
            source: "cli".into(),
            cached: false,
            error: None,
        })
    }
    fn prepare_codex_configured(
        &self,
        t: &Target,
        cwd: &str,
        resume: Option<&str>,
        name: &str,
        settings: &crate::codex_settings::Settings,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        self.0.lock().unwrap().settings.push(settings.clone());
        self.prepare_codex(t, cwd, resume, name, check_current)
    }
    fn resources(&self) -> &dyn crate::execution::Resources {
        self.1.as_deref().unwrap_or(&crate::execution::LOCAL)
    }
    fn prompt_receipt(
        &self,
        t: &Target,
        prompt: &str,
        _: &str,
        expected: Option<&str>,
    ) -> Result<Option<crate::workflow::PromptReceipt>, String> {
        self.observe("receipt");
        let id = self.session_id(t)?.ok_or("flow.session_missing")?;
        if expected.is_some_and(|old| old != id) {
            return Err("flow.session_mismatch".into());
        }
        let s = self.0.lock().unwrap();
        Ok(
            (s.receipt_available && s.sent.iter().any(|p| p == prompt)).then(|| {
                crate::workflow::PromptReceipt {
                    session_id: id,
                    evidence: "Exact user message after dispatch".into(),
                }
            }),
        )
    }
    fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
        if let Some(error) = self.0.lock().unwrap().lookup_error.clone() { return Err(error); }
        Ok(self
            .0
            .lock()
            .unwrap()
            .panes
            .values()
            .map(|(t, _)| t.clone())
            .collect())
    }
    fn available(&self, w: &str, _: &str) -> Result<Vec<Target>, String> {
        let candidates = self
            .targets(w)?
            .into_iter()
            .filter(|t| matches!(t.status.as_str(), "idle" | "done"))
            .collect();
        self.observe("available");
        Ok(candidates)
    }
    fn pane_inventory(
        &self,
        w: &str,
    ) -> Result<Option<Vec<crate::workflow::PaneIdentity>>, String> {
        let inventory = Some(
            self.targets(w)?
                .into_iter()
                .map(|t| crate::workflow::PaneIdentity {
                    pane_id: t.pane_id,
                    terminal_id: t.terminal_id,
                })
                .collect(),
        );
        self.observe("inventory");
        Ok(inventory)
    }
    fn create_pane(
        &self,
        _: &str,
        _: &str,
        _: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        check_current()?;
        let mut s = self.0.lock().unwrap();
        if s.create_error {
            return Err("timeout".into());
        }
        s.created += 1;
        let mut t = target(&format!("created-{}", s.created));
        if s.remote {
            t.machine_id = "ssh-test".into();
        }
        s.panes.insert(t.pane_id.clone(), (t.clone(), None));
        let lost = s.lost_create_response;
        drop(s);
        self.observe("created");
        if lost {
            return Err("herdr.timeout".into());
        }
        Ok(t)
    }
    fn prepare_codex(
        &self,
        t: &Target,
        cwd: &str,
        resume: Option<&str>,
        _: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        self.observe("preparing");
        check_current()?;
        let mut s = self.0.lock().unwrap();
        s.prepared.push(resume.map(str::to_string));
        s.directories.push(cwd.into());
        let gate = s.prepare_gate.clone();
        drop(s);
        if let Some(gate) = gate {
            let (lock, wake) = &*gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
        }
        let mut s = self.0.lock().unwrap();
        if s.prepare_error {
            return Err("timeout".into());
        }
        if s.prepare_busy {
            return Err(r#"herdr.failed: {"error":{"code":"agent_pane_busy","message":"not an available shell"},"id":"cli:agent:start"}"#.into());
        }
        let id = if s.mismatch {
            uuid::Uuid::new_v4().to_string()
        } else {
            resume
                .map(str::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        };
        let mut live = t.clone();
        live.process_id = uuid::Uuid::new_v4().to_string();
        s.panes.insert(t.pane_id.clone(), (live.clone(), Some(id)));
        drop(s);
        self.observe("prepared");
        Ok(live)
    }
    fn session_id(&self, t: &Target) -> Result<Option<String>, String> {
        let s = self.0.lock().unwrap();
        let (live, id) = s.panes.get(&t.pane_id).ok_or("binding.missing")?;
        if !live.same_identity(t) {
            return Err("binding.identity_changed".into());
        }
        let result = if s.missing_id { None } else { id.clone() };
        drop(s);
        self.observe("session");
        Ok(result)
    }
    fn pane_exists(&self, t: &Target) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().panes.contains_key(&t.pane_id))
    }
    fn close_pane(
        &self,
        t: &Target,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check_current()?;
        let mut s = self.0.lock().unwrap();
        s.closed.push(t.pane_id.clone());
        if s.close_error {
            return Err("timeout".into());
        }
        s.panes.remove(&t.pane_id);
        if s.close_then_error {
            return Err("timeout".into());
        }
        Ok(())
    }
    fn send(&self, _: &Target, prompt: &str) -> Result<(), SendError> {
        let mut s = self.0.lock().unwrap();
        if let Some(message) = &s.before_input_error { return Err(SendError { message: message.clone(), uncertain: false }); }
        s.sent.push(prompt.into());
        let send_error = s.send_error;
        drop(s);
        self.observe("sent");
        if send_error {
            return Err(SendError {
                message: "timeout".into(),
                uncertain: true,
            });
        }
        Ok(())
    }
    fn send_first_prompt(
        &self, target: &Target, prompt: &str, expected: Option<&str>,
        check_current: &dyn Fn() -> Result<(), String>,
        received: &dyn Fn() -> Result<bool, String>,
    ) -> Result<(), SendError> {
        if expected.is_some_and(|id| self.session_id(target).ok().flatten().as_deref() != Some(id)) {
            return Err(SendError { message:"flow.session_mismatch".into(), uncertain:false });
        }
        let states = self.0.lock().unwrap().first_prompt_states.take();
        let Some(states) = states else { return self.send_checking_receipt(target, prompt, check_current, received) };
        let last = states.last().unwrap().clone();
        let mut states = states.into_iter();
        let check = || check_current().map_err(|message| SendError { message, uncertain: false });
        crate::herdr::scripted_first_prompt(&target.pane_id, prompt, |args| {
            check()?;
            let mut state = self.0.lock().unwrap();
            state.input_requests.push(args.iter().map(|s| s.to_string()).collect());
            if args[1] == "send-text" { state.sent.push(prompt.into()); }
            Ok(())
        }, || { check()?; Ok(states.next().unwrap_or_else(|| last.clone())) },
        || received().map_err(|message| SendError { message, uncertain: true }))
    }
    fn send_checking_receipt(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
        received: &dyn Fn() -> Result<bool, String>,
    ) -> Result<(), SendError> {
        self.send_checked(target, prompt, check_current)?;
        if self.0.lock().unwrap().early_receipt {
            self.0.lock().unwrap().receipt_checks += 1;
            let found = received().map_err(|message| SendError {
                message,
                uncertain: true,
            })?;
            self.observe("early_receipt");
            assert!(found, "test fixture expected an exact receipt");
            return Err(SendError {
                message: "dispatch.submit_verify_required".into(),
                uncertain: true,
            });
        }
        Ok(())
    }
    fn find_created(&self, _: &str, _: &str) -> Result<Option<Target>, String> {
        Ok(None)
    }
}
fn fixture() -> (Workflow, Arc<Fake>) {
    fixture_on(false)
}

// Reproduce the persisted pre-startup-input protocol for upgrade compatibility.
// Production new starts never select this path.
impl Workflow {
    fn legacy_start_task(&self, task: &str, prompt: &str, id: &str) -> Result<Outcome,String> {
        self.begin_flow(task,id,"start")?;
        let result: Result<Outcome,String> = (|| {
            let (workspace,cwd,_) = self.project_context(task)?;
            let shell = self.herdr.create_pane(&workspace,&cwd,id,&||self.check_flow(task,id))?;
            self.reserve_target(task,id,&shell)?;
            let live = self.herdr.prepare_codex(&shell,&cwd,None,id,&||self.check_flow(task,id))?;
            self.reserve_target(task,id,&live)?;
            self.save_session(task,id,&live,&cwd,None)?;
            self.db.lock().unwrap().execute("UPDATE task_bindings SET state='valid' WHERE task_id=?1",[task]).unwrap();
            self.dispatch_inner(task,&live.pane_id,prompt,&format!("{id}:dispatch"),Some(id))?;
            self.activate(task,id,&live)?;
            Ok(Outcome::default())
        })();
        if let Err(ref error) = result {
            if crate::herdr::agent_start_rejected_busy(error) { self.release_rejected_start(task,id,error,&||Ok(()))?; }
            else { self.end_flow(id,"unknown",error)?; }
        }
        result
    }
}

fn fixture_on(remote: bool) -> (Workflow, Arc<Fake>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    if remote {
        db.execute("INSERT INTO machines(id,name,kind,session) VALUES ('ssh-test','SSH test','ssh','default')", []).unwrap();
    }
    db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id) VALUES ('p1','P1','.','workspace','now',?1),('p2','P2','.','workspace','now',?1)", [if remote { "ssh-test" } else { crate::machines::LOCAL_ID }]).unwrap();
    db.execute_batch("INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('a','p1','A','todo','now','now'),('b','p2','B','todo','now','now');").unwrap();
    // Historical duplicate association fixture keeps testing cross-project pane ownership.
    db.execute_batch("DROP INDEX project_workspace_identity; UPDATE projects SET herdr_session='default',workspace_id='w1',sync_status='ready';").unwrap();
    let fake = Arc::new(Fake::default());
    fake.0.lock().unwrap().remote = remote;
    (
        Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: fake.clone(),
        },
        fake,
    )
}

fn op() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn status(app: &Workflow, task: &str) -> String {
    app.project_context(task).unwrap().2
}
fn saved(app: &Workflow, task: &str) -> Option<String> {
    view(&app.db.lock().unwrap(), task).unwrap().session_id
}

fn await_start(app: &Workflow, task: &str) -> Flow {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let flow = view(&app.db.lock().unwrap(), task).unwrap();
        if flow.state.as_deref() != Some("pending") {
            return flow;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "background start did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn set_prompt(app: &Workflow, task: &str, text: &str) {
    let json = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]}).to_string();
    crate::tasks::update(
        &app.db.lock().unwrap(),
        task,
        "Task",
        &json,
        crate::models::TaskPriority::Normal,
    )
    .unwrap();
}

mod allocation;
mod background_start;
mod completion;
mod archive;
mod recovery;
mod first_prompt;
mod startup_prompt;
mod reset;
