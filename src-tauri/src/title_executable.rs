//! Resolve on the executing machine, including npm installations absent from SSH PATH.
//! npm's official JS entry is run with node directly; no prompt or path enters cmd.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn resolve() -> PathBuf {
    resolve_with_saved_paths(saved_paths)
}

fn resolve_with_saved_paths(saved: impl FnOnce() -> Vec<PathBuf>) -> PathBuf {
    if let Some(explicit) = std::env::var_os("KANBAN_CODEX_EXECUTABLE") {
        return explicit.into();
    }
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(entry) = find_entry(std::env::split_paths(&path)) {
            return entry;
        }
    }
    // sshd may inherit an old/minimal PATH. Read persisted paths on the executing
    // machine, without changing the user's environment or caching a version.
    if let Some(entry) = find_entry(saved()) {
        return entry;
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs/OpenAI/Codex/bin/codex.exe"));
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(roaming).join("npm/codex.cmd"));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| "codex".into())
}

fn find_entry(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for directory in paths {
        if directory.as_os_str().is_empty() {
            continue;
        }
        #[cfg(windows)]
        let names = ["codex.exe", "codex.cmd"];
        #[cfg(not(windows))]
        let names = ["codex"];
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn powershell(script: &str) -> Command {
    use base64::Engine;
    let executable = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .unwrap_or_else(|| "powershell.exe".into());
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); {script}"
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = Command::new(executable);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ]);
    command
}

#[cfg(windows)]
fn discovery_output(command: Command) -> Result<String, String> {
    let output = crate::title_process::run(
        command,
        vec![],
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(10),
    )?;
    String::from_utf8(output).map_err(|_| "title.unavailable".into())
}

fn saved_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let command = powershell(
            r#"
if (-not $env:LOCALAPPDATA) { $env:LOCALAPPDATA=[Environment]::GetFolderPath('LocalApplicationData') }
if (-not $env:APPDATA) { $env:APPDATA=[Environment]::GetFolderPath('ApplicationData') }
$paths = foreach ($scope in @('User','Machine')) {
    foreach ($entry in ([Environment]::GetEnvironmentVariable('Path',$scope) -split ';')) {
        if (-not [string]::IsNullOrWhiteSpace($entry)) {
            [Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"'))
        }
    }
}
ConvertTo-Json -Compress -InputObject @($paths)
"#,
        );
        discovery_output(command)
            .ok()
            .and_then(|output| serde_json::from_str::<Vec<PathBuf>>(&output).ok())
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// Follow the discovered launcher's own resolver, never guess a release directory.
pub(crate) fn standalone_native(entry: &Path) -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    if entry.is_file()
        && entry
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("codex.cmd"))
    {
        let resolver = entry
            .parent()
            .ok_or("title.unavailable")?
            .join("resolve-codex.ps1");
        if resolver.is_file() {
            let mut command = powershell("& $env:KANBAN_CODEX_RESOLVER; if (-not $?) { exit 1 }");
            command.env("KANBAN_CODEX_RESOLVER", resolver);
            let output = discovery_output(command)?;
            let path = PathBuf::from(output.trim());
            if !path.is_absolute()
                || !path.is_file()
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            {
                return Err("title.unavailable".into());
            }
            return Ok(Some(path));
        }
    }
    Ok(None)
}

pub fn command(executable: &Path) -> Result<Command, String> {
    if let Some(native) = standalone_native(executable)? {
        return Ok(Command::new(native));
    }
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
            assert_eq!(resolve_with_saved_paths(Vec::new), expected);
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

    #[test]
    #[cfg(windows)]
    fn saved_path_fallback_preserves_explicit_and_process_path_precedence() {
        const CHILD: &str = "KANBAN_SAVED_PATH_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let mode = std::env::var("KANBAN_SAVED_PATH_MODE").unwrap();
            let entry = resolve_with_saved_paths(|| {
                assert_eq!(mode, "saved", "saved PATH is only read as a fallback");
                vec![root.join("missing"), root.join("saved path 中文 & space")]
            });
            assert_eq!(
                entry,
                match mode.as_str() {
                    "explicit" => root.join("explicit-missing.exe"),
                    "process" => root.join("process/codex.cmd"),
                    _ => root.join("saved path 中文 & space/codex.cmd"),
                }
            );
            return;
        }
        let root = std::env::temp_dir().join(format!("codex-discovery-{}", uuid::Uuid::new_v4()));
        for dir in [
            "process",
            "saved path 中文 & space",
            "Programs/OpenAI/Codex/bin",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "process/codex.cmd",
            "saved path 中文 & space/codex.cmd",
            "Programs/OpenAI/Codex/bin/codex.exe",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        for mode in ["explicit", "process", "saved"] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "title_executable::tests::saved_path_fallback_preserves_explicit_and_process_path_precedence"])
                .env(CHILD, &root).env("KANBAN_SAVED_PATH_MODE", mode)
                .env("LOCALAPPDATA", &root).env_remove("APPDATA")
                .env("PATH", if mode == "process" { root.join("process") } else { root.join("missing") })
                .env_remove("KANBAN_CODEX_EXECUTABLE");
            if mode == "explicit" {
                child.env("KANBAN_CODEX_EXECUTABLE", root.join("explicit-missing.exe"));
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
