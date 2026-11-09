//! A short pane command invokes this mode. Prompt bytes never enter a shell command.
use crate::startup_prompt::Request;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub(crate) fn fingerprint(request: &Request) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request).map_err(err)?)
    ))
}
pub(crate) fn path(operation: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(operation).map_err(|_| "startup.invalid_operation")?;
    let root = PathBuf::from(
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .ok_or("startup.home_missing")?,
    )
    .join(".herdr-kanban/startups");
    std::fs::create_dir_all(&root).map_err(err)?;
    Ok(root.join(format!("{operation}.json")))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}
pub(crate) fn stage(request: &Request) -> Result<String, String> {
    request.settings.validate()?;
    // Validate the actual native command before making any pane request.
    let executable = resolve()?;
    let args = arguments(request);
    #[cfg(windows)]
    if args
        .iter()
        .map(|a| windows_argument_len(a) + 1)
        .sum::<usize>()
        + executable.as_os_str().len()
        + 3
        > 32760
    {
        return Err("startup.prompt_too_long".into());
    }
    let path = path(&request.operation)?;
    write_new(&path, &serde_json::to_vec(request).map_err(err)?)?;
    // A test/native acceptance may supply the application binary; never run the test harness.
    let helper = std::env::var_os("KANBAN_STARTUP_HELPER")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(err)?;
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "''"));
    // PowerShell does not wait for a GUI-subsystem executable invoked with `&`.
    // Keep its console alive explicitly, including in the production GUI build.
    let launcher_args = format!("--codex-start \"{}\"", path.to_string_lossy()).replace('\'', "''");
    let script = format!(
        "$p=Start-Process -FilePath {} -ArgumentList '{launcher_args}' -NoNewWindow -Wait -PassThru; exit $p.ExitCode",
        quote(&helper)
    );
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    Ok(format!(
        "powershell.exe -NoLogo -NoProfile -EncodedCommand {encoded}"
    ))
}
fn arguments(request: &Request) -> Vec<String> {
    let mut args = vec![
        "--no-daemon".into(),
        "--dangerously-bypass-approvals-and-sandbox".into(),
        "-C".into(),
        request.cwd.clone(),
    ];
    args.extend(request.settings.args());
    args.extend(["--".into(), request.prompt.clone()]);
    args
}
fn command(request: &Request, executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments(request));
    command
}
// Rust's native Windows argv quoting: only backslashes before quotes or the
// closing quote are doubled. Ordinary long text must not be rejected at 16 KB.
fn windows_argument_len(arg: &str) -> usize {
    let mut length = 2;
    let mut slashes = 0;
    for c in arg.encode_utf16() {
        if c == b'\\' as u16 {
            slashes += 1;
        } else {
            length += slashes + if c == b'"' as u16 { slashes + 2 } else { 1 };
            slashes = 0;
        }
    }
    length + slashes * 2
}
fn resolve() -> Result<PathBuf, String> {
    let selected = crate::title_executable::resolve();
    resolve_native(&selected)
}

fn resolve_native(selected: &Path) -> Result<PathBuf, String> {
    if selected
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        && selected.is_file()
    {
        return Ok(selected.into());
    }
    if selected
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
    {
        let root = selected.parent().ok_or("startup.native_codex_missing")?;
        if let Some(native) = crate::title_executable::standalone_native(selected)
            .map_err(|_| "startup.native_codex_missing")?
        {
            return Ok(native);
        }
        for package in [
            "node_modules/@openai/codex",
            "node_modules/@openai/codex-win32-x64",
            "node_modules/@openai/codex/node_modules/@openai/codex-win32-x64",
        ] {
            for subdir in ["bin", "codex"] {
                let p = root
                    .join(package)
                    .join(format!("vendor/x86_64-pc-windows-msvc/{subdir}/codex.exe"));
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    Err("startup.native_codex_missing".into())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Started {
    pub fingerprint: String,
    pub pid: u32,
    pub birth: u64,
}
#[cfg(windows)]
pub(crate) fn birth(pid: u32) -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut times: [FILETIME; 4] = std::mem::zeroed();
        let p = times.as_mut_ptr();
        let ok = GetProcessTimes(handle, p, p.add(1), p.add(2), p.add(3));
        CloseHandle(handle);
        (ok != 0).then_some(
            (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime),
        )
    }
}
#[cfg(not(windows))]
pub(crate) fn birth(_: u32) -> Option<u64> {
    None
}
pub(crate) fn started(request: &Request) -> Result<Option<Started>, String> {
    let file = path(&request.operation)?.with_extension("started");
    if let Ok(data) = std::fs::read(file.with_extension("failed")) {
        let failure: serde_json::Value = serde_json::from_slice(&data).map_err(err)?;
        if failure["fingerprint"].as_str() != Some(fingerprint(request)?.as_str()) {
            return Err("binding.identity_changed".into());
        }
        return Err(failure["error"]
            .as_str()
            .unwrap_or("startup.launch_failed")
            .into());
    }
    let data = match std::fs::read(file) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(e)),
    };
    let record: Started = serde_json::from_slice(&data).map_err(err)?;
    if record.fingerprint != fingerprint(request)? {
        return Err("binding.identity_changed".into());
    }
    if birth(record.pid) != Some(record.birth) {
        return Err("startup.process_exited".into());
    }
    Ok(Some(record))
}
pub fn run() -> i32 {
    #[cfg(windows)]
    unsafe {
        // The GUI release binary must join the pane console before starting the TUI.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(pid: u32) -> i32;
        }
        AttachConsole(u32::MAX);
    }
    match run_inner() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
fn run_inner() -> Result<i32, String> {
    let file = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .ok_or("startup.invalid_request")?,
    );
    let request: Request =
        serde_json::from_slice(&std::fs::read(&file).map_err(err)?).map_err(err)?;
    if file != path(&request.operation)? {
        return Err("startup.invalid_request".into());
    }
    // Defense against accidental pane-command replay, including response loss.
    write_new(
        &file.with_extension("claimed"),
        fingerprint(&request)?.as_bytes(),
    )?;
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = request.target.session.clone();
    let launched = cli
        .validate_startup_context(&request)
        .and_then(|_| launch_suspended(&request, &file));
    let mut child = match launched {
        Ok(child) => child,
        Err(error) => {
            let error = format!("startup.not_launched:{error}");
            let failure = serde_json::json!({"fingerprint":fingerprint(&request)?,"error":error});
            let _ = write_new(
                &file.with_extension("failed"),
                &serde_json::to_vec(&failure).map_err(err)?,
            );
            return Err(error);
        }
    };
    // Only the database retains the frozen prompt once the launcher has consumed it.
    let _ = std::fs::remove_file(&file);
    child.wait().map(|s| s.code().unwrap_or(1)).map_err(err)
}

fn launch_suspended(request: &Request, file: &Path) -> Result<std::process::Child, String> {
    spawn_journaled(command(request, &resolve()?), request, file)
}
fn spawn_journaled(
    mut command: Command,
    request: &Request,
    file: &Path,
) -> Result<std::process::Child, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let mut child = command.spawn().map_err(err)?;
    let result = (|| {
        let record = Started {
            fingerprint: fingerprint(request)?,
            pid: child.id(),
            birth: birth(child.id()).ok_or("startup.identity_unknown")?,
        };
        let temp = file.with_extension("starting");
        write_new(&temp, &serde_json::to_vec(&record).map_err(err)?)?;
        std::fs::rename(&temp, file.with_extension("started")).map_err(err)?;
        // No Codex instruction runs until its recoverable identity is durable.
        resume(child.id())
    })();
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}
#[cfg(windows)]
fn resume(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("startup.thread_unknown".into());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = vec![];
        let mut more = Thread32First(snapshot, &mut entry);
        while more != 0 {
            if entry.th32OwnerProcessID == pid {
                found.push(entry.th32ThreadID);
            }
            more = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        if found.len() != 1 {
            return Err("startup.thread_unknown".into());
        }
        let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, found[0]);
        if thread.is_null() {
            return Err("startup.thread_unknown".into());
        }
        let count = ResumeThread(thread);
        CloseHandle(thread);
        if count == u32::MAX {
            Err("startup.resume_failed".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(not(windows))]
fn resume(_: u32) -> Result<(), String> {
    Err("startup.unsupported".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn standalone_resolver_tracks_updates_and_rejects_invalid_results() {
        let root = std::env::temp_dir()
            .join(format!("codex-resolver-{}", uuid::Uuid::new_v4()))
            .join("中文 path & ' quoted");
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("codex.cmd");
        let resolver = root.join("resolve-codex.ps1");
        std::fs::write(&entry, "").unwrap();
        std::fs::write(&resolver, "[Console]::WriteLine([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'selected.txt')))").unwrap();
        // The same launcher follows an update, including arbitrary release layouts.
        for release in ["old release", "new release"] {
            let native = root.join(release).join("codex.exe");
            std::fs::create_dir_all(native.parent().unwrap()).unwrap();
            std::fs::write(&native, "").unwrap();
            std::fs::write(root.join("selected.txt"), native.to_str().unwrap()).unwrap();
            assert_eq!(resolve_native(&entry).unwrap(), native);
            assert_eq!(
                crate::title_executable::command(&entry)
                    .unwrap()
                    .get_program(),
                native
            );
        }
        for invalid in [
            root.join("missing.exe").to_string_lossy().into_owned(),
            "relative.exe".into(),
            "one.exe\ntwo.exe".into(),
        ] {
            std::fs::write(root.join("selected.txt"), invalid).unwrap();
            assert_eq!(
                resolve_native(&entry).unwrap_err(),
                "startup.native_codex_missing"
            );
        }
        std::fs::write(&resolver, "exit 1").unwrap();
        assert_eq!(
            resolve_native(&entry).unwrap_err(),
            "startup.native_codex_missing"
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn npm_native_layouts_remain_supported() {
        for package in [
            "node_modules/@openai/codex",
            "node_modules/@openai/codex-win32-x64",
            "node_modules/@openai/codex/node_modules/@openai/codex-win32-x64",
        ] {
            for subdir in ["bin", "codex"] {
                let root = std::env::temp_dir().join(format!("codex-npm-{}", uuid::Uuid::new_v4()));
                let native = root
                    .join(package)
                    .join(format!("vendor/x86_64-pc-windows-msvc/{subdir}/codex.exe"));
                std::fs::create_dir_all(native.parent().unwrap()).unwrap();
                std::fs::write(root.join("codex.cmd"), "").unwrap();
                std::fs::write(&native, "").unwrap();
                assert_eq!(resolve_native(&root.join("codex.cmd")).unwrap(), native);
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "Read-only installed Codex discovery with minimal SSH-like environment"]
    fn installed_codex_is_discovered_without_process_path_or_appdata() {
        const CHILD: &str = "KANBAN_INSTALLED_CODEX_DISCOVERY_TEST";
        if std::env::var_os(CHILD).is_some() {
            let native = resolve().unwrap();
            let output = Command::new(&native).arg("--version").output().unwrap();
            assert!(output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("codex-cli"));
            println!(
                "native={} version={}",
                native.display(),
                String::from_utf8_lossy(&output.stdout).trim()
            );
            return;
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "codex_launcher::tests::installed_codex_is_discovered_without_process_path_or_appdata", "--ignored", "--nocapture"])
            .env(CHILD, "1").env("PATH", "").env_remove("LOCALAPPDATA").env_remove("APPDATA")
            .env_remove("KANBAN_CODEX_EXECUTABLE").output().unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn native_process_receives_exact_multiline_long_prompt_and_settings() {
        let root = std::env::temp_dir().join(format!("startup-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("capture.rs");
        let executable = root.join("capture.exe");
        std::fs::write(&source,r#"fn main() { use std::io::Write; assert!(std::path::Path::new(&std::env::var_os("CAPTURE_JOURNAL").unwrap()).is_file()); let mut f=std::fs::File::create(std::env::var_os("CAPTURE_PATH").unwrap()).unwrap(); for arg in std::env::args().skip(1) { f.write_all(arg.as_bytes()).unwrap(); f.write_all(&[0]).unwrap(); } }"#).unwrap();
        assert!(
            Command::new("rustc")
                .args([
                    source.as_os_str(),
                    std::ffi::OsStr::new("-o"),
                    executable.as_os_str()
                ])
                .status()
                .unwrap()
                .success()
        );
        let target = crate::workflow::Target {
            machine_id: "local".into(),
            session: "default".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "term".into(),
            agent: String::new(),
            process_id: "1:1".into(),
            codex_resume_locks: vec![],
            title: String::new(),
            status: "unknown".into(),
        };
        for count in [1, 600, 1400] {
            let prompt = format!(
                "--resume 中文\n\n\t'quote' \"double\" C:\\测试\\ $(not-a-command)\n{}\n$skill",
                "transport sample. ".repeat(count)
            );
            let request = Request {
                operation: uuid::Uuid::new_v4().to_string(),
                target: target.clone(),
                cwd: "C:\\folder with space\\中文".into(),
                prompt: prompt.clone(),
                settings: crate::codex_settings::Settings {
                    model: Some("test-model".into()),
                    effort: Some("high".into()),
                },
                epoch: 0,
            };
            let output = root.join("args.bin");
            let file = root.join(format!("request-{count}.json"));
            let mut cmd = command(&request, &executable);
            cmd.env("CAPTURE_PATH", &output)
                .env("CAPTURE_JOURNAL", file.with_extension("started"));
            assert!(
                spawn_journaled(cmd, &request, &file)
                    .unwrap()
                    .wait()
                    .unwrap()
                    .success()
            );
            let data = std::fs::read(&output).unwrap();
            let received: Vec<&str> = std::str::from_utf8(&data)
                .unwrap()
                .split_terminator('\0')
                .collect();
            assert_eq!(
                received,
                [
                    "--no-daemon",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "-C",
                    &request.cwd,
                    "--model",
                    "test-model",
                    "-c",
                    "model_reasoning_effort=\"high\"",
                    "--",
                    &prompt
                ]
            );
            assert!(
                received
                    .iter()
                    .map(|arg| windows_argument_len(arg) + 1)
                    .sum::<usize>()
                    < 32760
            );
            // Simulate a failed identity write after CreateProcess: the suspended
            // child must be killed before it can produce even its first effect.
            std::fs::remove_file(&output).unwrap();
            write_new(&file.with_extension("starting"), b"occupied").unwrap();
            let mut cmd = command(&request, &executable);
            cmd.env("CAPTURE_PATH", &output)
                .env("CAPTURE_JOURNAL", file.with_extension("started"));
            assert!(spawn_journaled(cmd, &request, &file).is_err());
            assert!(!output.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
