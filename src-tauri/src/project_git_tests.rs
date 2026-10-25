use super::*;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("kanban-git-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> &str {
        self.0.to_str().unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-C",
                self.root(),
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=.no-hooks",
                "-c",
                "user.name=Kanban",
                "-c",
                "user.email=kanban@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn init(&self) {
        self.git(&["init", "--initial-branch=feature/workspace-label"]);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unborn_branch_subdirectory_and_external_branch_rename_are_observed() {
    let fixture = Fixture::new();
    fixture.init();
    assert_eq!(
        read(fixture.root()),
        GitStatus {
            state: GitState::Branch,
            head: Some("feature/workspace-label".into())
        }
    );
    let sub = fixture.0.join("nested");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(read(sub.to_str().unwrap()), read(fixture.root()));
    fixture.git(&["branch", "-m", "feature/新的分支"]);
    assert_eq!(
        read(fixture.root()).head.as_deref(),
        Some("feature/新的分支")
    );
}

#[test]
fn detached_head_and_linked_worktree_use_their_own_head() {
    let fixture = Fixture::new();
    fixture.init();
    fixture.git(&["commit", "--allow-empty", "-m", "fixture"]);
    let short = fixture.git(&["rev-parse", "--short", "HEAD"]);
    let linked = fixture.0.join("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-b",
        "feature/linked",
        linked.to_str().unwrap(),
    ]);
    fixture.git(&["checkout", "--detach", "HEAD"]);
    assert_eq!(
        read(fixture.root()),
        GitStatus {
            state: GitState::Detached,
            head: Some(short)
        }
    );
    assert_eq!(
        read(linked.to_str().unwrap()),
        GitStatus {
            state: GitState::Branch,
            head: Some("feature/linked".into())
        }
    );
}

#[test]
fn non_repository_missing_directory_and_missing_git_are_distinct() {
    let fixture = Fixture::new();
    assert_eq!(read(fixture.root()).state, GitState::NotRepository);
    assert_eq!(
        read(fixture.0.join("missing").to_str().unwrap()).state,
        GitState::Unavailable
    );
    assert_eq!(read("").state, GitState::Unavailable);
    assert_eq!(
        read_with_git(
            fixture.root(),
            fixture.0.join("missing-git.exe").as_os_str()
        )
        .state,
        GitState::Unavailable
    );
}

#[test]
fn batch_results_keep_project_and_directory_identity() {
    let fixture = Fixture::new();
    fixture.init();
    let results = read_projects(vec![
        ("a".into(), fixture.root().into()),
        ("b".into(), fixture.root().into()),
        ("c".into(), "".into()),
    ]);
    assert_eq!(
        results
            .iter()
            .map(|r| r.project_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(results[0].root_path, fixture.root());
    assert_eq!(results[0].status, results[1].status);
    assert_eq!(results[2].status.state, GitState::Unavailable);
}

#[test]
fn slow_process_is_terminated_at_the_deadline() {
    #[cfg(windows)]
    let mut command = Command::new("powershell.exe");
    #[cfg(windows)]
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 5",
    ]);
    #[cfg(not(windows))]
    let mut command = Command::new("sleep");
    #[cfg(not(windows))]
    command.arg("5");
    let start = Instant::now();
    assert!(output(command, start + Duration::from_millis(150)).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
}
