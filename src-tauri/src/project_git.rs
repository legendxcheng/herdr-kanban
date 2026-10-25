//! Read-only Git labels for workspace navigation. Never hold the database lock here.
use serde::Serialize;
use std::{
    collections::HashMap,
    ffi::OsStr,
    io::Read,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitState {
    Branch,
    Detached,
    NotRepository,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GitStatus {
    pub state: GitState,
    pub head: Option<String>,
}

impl GitStatus {
    fn unavailable() -> Self {
        Self {
            state: GitState::Unavailable,
            head: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProjectGitStatus {
    pub project_id: String,
    pub root_path: String,
    #[serde(flatten)]
    pub status: GitStatus,
}

/// Deduplicate shared directories and limit concurrent Git processes to four.
pub fn read_projects(projects: Vec<(String, String)>) -> Vec<ProjectGitStatus> {
    let mut roots: Vec<_> = projects.iter().map(|(_, root)| root.as_str()).collect();
    roots.sort_unstable();
    roots.dedup();
    let mut statuses = HashMap::new();
    for chunk in roots.chunks(4) {
        thread::scope(|scope| {
            let jobs: Vec<_> = chunk
                .iter()
                .map(|root| scope.spawn(move || read(root)))
                .collect();
            for (root, job) in chunk.iter().zip(jobs) {
                statuses.insert(
                    *root,
                    job.join().unwrap_or_else(|_| GitStatus::unavailable()),
                );
            }
        });
    }
    projects
        .iter()
        .map(|(id, root)| ProjectGitStatus {
            project_id: id.clone(),
            root_path: root.clone(),
            status: statuses
                .get(root.as_str())
                .cloned()
                .unwrap_or_else(GitStatus::unavailable),
        })
        .collect()
}

fn read(root: &str) -> GitStatus {
    read_with_git(root, OsStr::new("git"))
}

fn read_with_git(root: &str, executable: &OsStr) -> GitStatus {
    if root.is_empty() || !Path::new(root).is_dir() {
        return GitStatus::unavailable();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let query = |args: &[&str]| {
        let mut command = Command::new(executable);
        command
            .args(["-C", root])
            .args(args)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("GIT_TERMINAL_PROMPT", "0");
        // The host's repository environment must not override this workspace's path.
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        ] {
            command.env_remove(key);
        }
        output(command, deadline)
    };
    let Ok((status, stdout, stderr)) = query(&["symbolic-ref", "--quiet", "--short", "HEAD"])
    else {
        return GitStatus::unavailable();
    };
    if status.success() {
        return label(GitState::Branch, &stdout);
    }
    if String::from_utf8_lossy(&stderr).contains("not a git repository") {
        return GitStatus {
            state: GitState::NotRepository,
            head: None,
        };
    }
    if status.code() == Some(1) {
        if let Ok((status, stdout, _)) = query(&["rev-parse", "--short", "HEAD"]) {
            if status.success() {
                return label(GitState::Detached, &stdout);
            }
        }
    }
    GitStatus::unavailable()
}

fn label(state: GitState, bytes: &[u8]) -> GitStatus {
    match std::str::from_utf8(bytes)
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(head) => GitStatus {
            state,
            head: Some(head.into()),
        },
        None => GitStatus::unavailable(),
    }
}

/// Metadata-only Git commands produce small output. Excess output times out or is rejected.
fn output(mut command: Command, deadline: Instant) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), ()> {
    if Instant::now() >= deadline {
        return Err(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ())?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
        }
    };
    fn read(stream: impl Read) -> Result<Vec<u8>, ()> {
        let mut bytes = Vec::new();
        stream
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() > 16 * 1024 {
            return Err(());
        }
        Ok(bytes)
    }
    Ok((
        status,
        read(child.stdout.take().ok_or(())?)?,
        read(child.stderr.take().ok_or(())?)?,
    ))
}

#[cfg(test)]
#[path = "project_git_tests.rs"]
mod tests;
