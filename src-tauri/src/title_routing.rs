//! Naming follows project ownership; remote failures never spend local allowance.
use crate::{
    machines::{self, Machine},
    task_titles::TitleGenerator,
};
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::time::{Duration, Instant};

trait Backend: Send + Sync {
    fn local(&self, input: &str, stop: &AtomicBool) -> Result<String, String>;
    fn remote(&self, machine: &Machine, input: &str, stop: &AtomicBool) -> Result<String, String>;
}

struct CodexBackend {
    db: Arc<Mutex<rusqlite::Connection>>,
}
impl Backend for CodexBackend {
    fn local(&self, input: &str, stop: &AtomicBool) -> Result<String, String> {
        crate::title_cli::CodexTitleGenerator::from_env().generate(input, stop)
    }
    fn remote(&self, machine: &Machine, input: &str, stop: &AtomicBool) -> Result<String, String> {
        let budget = crate::remote_transport::Budget {
            stop,
            // A first use may deploy the helper. Model execution still has its
            // own 30s limit and each title request a 55s transport limit.
            deadline: Instant::now() + Duration::from_secs(240),
        };
        let current =
            crate::remote_install::ensure_current_with_budget(&self.db, machine, &budget)?;
        budget.verify_profile(&current)?;
        crate::remote_transport::request_with_budget(
            &current,
            serde_json::json!({"method":"task_title", "input":input}),
            &budget,
        )
    }
}

pub struct RoutedTitles {
    db: Arc<Mutex<rusqlite::Connection>>,
    backend: Box<dyn Backend>,
}
impl RoutedTitles {
    pub fn new(db: Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self {
            db: db.clone(),
            backend: Box::new(CodexBackend { db }),
        }
    }
    fn machine(&self, project: &str) -> Result<Machine, String> {
        let db = self.db.lock().map_err(|_| "title.storage")?;
        let id = machines::project_machine(&db, project)?;
        let machine = machines::get(&db, &id)?;
        if machine.disabled {
            return Err("machine.disabled".into());
        }
        Ok(machine)
    }
}
impl TitleGenerator for RoutedTitles {
    fn generate(&self, project: &str, input: &str, stop: &AtomicBool) -> Result<String, String> {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return Err("title.cancelled".into());
        }
        let machine = self.machine(project)?;
        let result = match machine.kind.as_str() {
            "local" => self.backend.local(input, stop),
            "ssh" => self.backend.remote(&machine, input, stop),
            _ => Err("machine.platform_unsupported".into()),
        };
        // Reject results if a machine was disabled/reconfigured during execution.
        let current = self.machine(project)?;
        if current.id != machine.id
            || current.kind != machine.kind
            || current.profile_id != machine.profile_id
            || current.ssh_target != machine.ssh_target
            || current.session != machine.session
            || current.host_identity != machine.host_identity
        {
            return Err("machine.identity_changed".into());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an explicit SSH machine fixture and helper build; uses remote Codex allowance"]
    fn live_ssh_title_uses_remote_codex_with_local_codex_unavailable() {
        let fixture =
            std::env::var("KANBAN_ACCEPTANCE_MACHINE_FILE").expect("explicit machine fixture");
        let helper = std::env::var("KANBAN_ACCEPTANCE_HELPER").expect("built helper");
        assert!(
            std::env::var("KANBAN_CODEX_EXECUTABLE")
                .unwrap()
                .contains("missing-title-acceptance")
        );
        let local = crate::title_cli::CodexTitleGenerator::from_env();
        assert_eq!(
            local
                .generate("Local probe", &AtomicBool::new(false))
                .unwrap_err(),
            "title.unavailable"
        );
        let machine: Machine = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        let machine =
            crate::remote_install::install(&machine, std::path::Path::new(&helper)).unwrap();
        let budget = crate::remote_transport::Budget {
            stop: &AtomicBool::new(false),
            deadline: Instant::now() + Duration::from_secs(55),
        };
        budget.verify_profile(&machine).unwrap();
        let title: String = crate::remote_transport::request_with_budget(&machine,
            serde_json::json!({"method":"task_title","input":"修复登录失败后缺少错误提示的问题，并保留用户已经填写的内容。"}), &budget).unwrap();
        assert!(!title.trim().is_empty());
        assert!(title.chars().count() <= 40);
        assert!(!title.chars().any(char::is_control));
        println!("remote_machine={} title={title}", machine.name);
    }
    struct Probe {
        db: Arc<Mutex<rusqlite::Connection>>,
        calls: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }
    impl Probe {
        fn assert_database_available(&self) {
            // Another title worker may briefly own the lock. A lock held across
            // this model call would remain unavailable until the deadline.
            let deadline = Instant::now() + Duration::from_secs(1);
            while self.db.try_lock().is_err() {
                assert!(
                    Instant::now() < deadline,
                    "no database lock during model call"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    impl Backend for Probe {
        fn local(&self, _: &str, _: &AtomicBool) -> Result<String, String> {
            self.assert_database_available();
            self.calls.lock().unwrap().push("local".into());
            Ok("Local title".into())
        }
        fn remote(&self, machine: &Machine, _: &str, _: &AtomicBool) -> Result<String, String> {
            self.assert_database_available();
            self.calls.lock().unwrap().push(machine.id.clone());
            if self.fail {
                Err("title.unavailable".into())
            } else {
                Ok("Remote title".into())
            }
        }
    }
    #[test]
    fn ownership_selects_codex_and_remote_failure_never_falls_back() {
        let db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        {
            let conn = db.lock().unwrap();
            crate::db::init(&conn).unwrap();
            conn.execute_batch("INSERT INTO machines(id,name,kind,session) VALUES('remote','Remote','ssh','default');
            INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id) VALUES
            ('local-project','Local','.','local-space','now','local'),
            ('remote-project','Remote','Z:/remote-only','remote-space','now','remote');").unwrap();
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        for fail in [false, true] {
            let router = RoutedTitles {
                db: db.clone(),
                backend: Box::new(Probe {
                    db: db.clone(),
                    calls: calls.clone(),
                    fail,
                }),
            };
            let stop = AtomicBool::new(false);
            assert_eq!(
                router.generate("local-project", "input", &stop).unwrap(),
                "Local title"
            );
            let result = router.generate("remote-project", "input", &stop);
            if fail {
                assert_eq!(result.unwrap_err(), "title.unavailable");
            } else {
                assert_eq!(result.unwrap(), "Remote title");
            }
        }
        assert_eq!(
            *calls.lock().unwrap(),
            ["local", "remote", "local", "remote"]
        );
        let router = RoutedTitles {
            db: db.clone(),
            backend: Box::new(Probe {
                db: db.clone(),
                calls: calls.clone(),
                fail: false,
            }),
        };
        db.lock()
            .unwrap()
            .execute("UPDATE machines SET disabled=1 WHERE id='remote'", [])
            .unwrap();
        assert_eq!(
            router
                .generate("remote-project", "input", &AtomicBool::new(false))
                .unwrap_err(),
            "machine.disabled"
        );
        assert_eq!(
            router
                .generate("local-project", "input", &AtomicBool::new(true))
                .unwrap_err(),
            "title.cancelled"
        );
        assert_eq!(calls.lock().unwrap().len(), 4);
    }

    #[test]
    fn background_worker_routes_saved_tasks_and_preserves_remote_errors() {
        let db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        {
            let conn = db.lock().unwrap();
            crate::db::init(&conn).unwrap();
            conn.execute_batch("INSERT INTO machines(id,name,kind,session) VALUES('remote','Remote','ssh','default');
            INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id) VALUES
            ('local-project','Local','.','local-space','now','local'),
            ('remote-project','Remote','Z:/remote-only','remote-space','now','remote');").unwrap();
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let router = RoutedTitles {
            db: db.clone(),
            backend: Box::new(Probe {
                db: db.clone(),
                calls: calls.clone(),
                fail: true,
            }),
        };
        let (send, receive) = std::sync::mpsc::channel();
        let service = crate::task_titles::TaskTitles::new(
            db.clone(),
            Arc::new(router),
            Arc::new(move |p| {
                send.send(p).unwrap();
            }),
        );
        let mut tasks = Vec::new();
        for project in ["local-project", "remote-project"] {
            tasks.push(service.create(crate::task_titles::NewTask {
                project_id: project.into(), name: None,
                prompt_json: serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Original summary"}]}]}).to_string(),
                priority: crate::models::TaskPriority::Normal, draft_id: None, locale: "en".into(),
            }).unwrap());
        }
        for _ in 0..2 {
            receive.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        service.shutdown();
        let conn = db.lock().unwrap();
        for (task, expected) in tasks.iter().zip([
            ("Local title", "succeeded", ""),
            ("Original summary", "failed", "title.unavailable"),
        ]) {
            let actual: (String, String, String) = conn.query_row("SELECT t.name,r.state,r.error FROM tasks t JOIN task_title_requests r ON r.task_id=t.id WHERE t.id=?1", [&task.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(
                actual,
                (expected.0.into(), expected.1.into(), expected.2.into())
            );
        }
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}
