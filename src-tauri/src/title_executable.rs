//! Resolve on the executing machine, including npm installations absent from SSH PATH.
//! npm's official JS entry is run with node directly; no prompt or path enters cmd.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn resolve() -> PathBuf {
    if let Some(explicit) = std::env::var_os("KANBAN_CODEX_EXECUTABLE") {
        return explicit.into();
    }
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs/OpenAI/Codex/bin/codex.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            #[cfg(windows)]
            {
                candidates.push(directory.join("codex.exe"));
                candidates.push(directory.join("codex.cmd"));
            }
            #[cfg(not(windows))]
            candidates.push(directory.join("codex"));
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(roaming).join("npm/codex.cmd"));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| "codex".into())
}

pub fn command(executable: &Path) -> Result<Command, String> {
    if executable
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
    {
        if !executable.is_file()
            || !executable
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.cmd"))
        {
            return Err("title.unavailable".into());
        }
        let root = executable.parent().ok_or("title.unavailable")?;
        let script = root.join("node_modules/@openai/codex/bin/codex.js");
        if !script.is_file() {
            return Err("title.unavailable".into());
        }
        let mut nodes = vec![root.join("node.exe")];
        if let Some(path) = std::env::var_os("PATH") {
            nodes.extend(std::env::split_paths(&path).map(|p| p.join("node.exe")));
        }
        for key in ["ProgramFiles", "LOCALAPPDATA"] {
            if let Some(directory) = std::env::var_os(key) {
                nodes.push(PathBuf::from(directory).join("nodejs/node.exe"));
            }
        }
        let node = nodes
            .into_iter()
            .find(|p| p.is_file())
            .ok_or("title.unavailable")?;
        let mut command = Command::new(node);
        command.arg(script);
        Ok(command)
    } else {
        Ok(Command::new(executable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn ssh_without_npm_on_path_resolves_user_install_and_honors_explicit_override() {
        const CHILD: &str = "KANBAN_TITLE_RESOLVER_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let expected = std::env::var_os("KANBAN_CODEX_EXECUTABLE")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("npm/codex.cmd"));
            assert_eq!(resolve(), expected);
            return;
        }
        let root = std::env::temp_dir().join(format!("kanban-resolve-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("npm")).unwrap();
        std::fs::write(root.join("npm/codex.cmd"), "").unwrap();
        for explicit in [None, Some(root.join("missing.exe"))] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "title_executable::tests::ssh_without_npm_on_path_resolves_user_install_and_honors_explicit_override"])
                .env(CHILD, &root).env("APPDATA", &root).env("LOCALAPPDATA", &root).env("PATH", "")
                .env_remove("KANBAN_CODEX_EXECUTABLE");
            if let Some(path) = explicit {
                child.env("KANBAN_CODEX_EXECUTABLE", path);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn npm_entry_uses_structured_arguments_and_unknown_wrappers_fail_closed() {
        let root = std::env::temp_dir()
            .join(format!("kanban-npm-{}", uuid::Uuid::new_v4()))
            .join("path with spaces & chars");
        std::fs::create_dir_all(root.join("node_modules/@openai/codex/bin")).unwrap();
        assert!(command(&root.join("codex.cmd")).is_err());
        std::fs::write(root.join("node.exe"), "").unwrap();
        std::fs::write(root.join("codex.cmd"), "").unwrap();
        std::fs::write(root.join("node_modules/@openai/codex/bin/codex.js"), "").unwrap();
        let command = command(&root.join("codex.cmd")).unwrap();
        assert_eq!(command.get_program(), root.join("node.exe"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [root
                .join("node_modules/@openai/codex/bin/codex.js")
                .as_os_str()]
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}
