//! Console protocol for the same signed/build artifact, without starting Tauri.
use serde_json::{Value, json};
use std::io::{Read, Write};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CliOutput {
    pub stdout: Vec<u8>,
    pub error: Option<String>,
    pub uncertain: bool,
}

pub fn run() -> i32 {
    let result = serve();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Preserve the official binary stream; verify identity in the very process
/// that opens it, so changing an SSH alias cannot attach to a different host.
pub fn bridge() -> i32 {
    let arguments: Vec<String> = std::env::args().collect();
    let result = (|| -> Result<i32, String> {
        if arguments.len() != 4 {
            return Err("machine.invalid_request".into());
        }
        let file = std::path::PathBuf::from(
            std::env::var_os("USERPROFILE").ok_or("machine.platform_unsupported")?,
        )
        .join(".herdr-kanban/identity");
        if std::fs::read_to_string(file)
            .map_err(|_| "machine.setup_required")?
            .trim()
            != arguments[2]
        {
            return Err("machine.identity_changed".into());
        }
        let status = std::process::Command::new(crate::herdr::Cli::from_env().executable)
            .args(["--session", &arguments[3], "remote-client-bridge"])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(status.code().unwrap_or(1))
    })();
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn serve() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > 20 * 1024 * 1024 {
        return Err("machine.input_limit".into());
    }
    let envelope: Value = serde_json::from_slice(&input).map_err(|_| "machine.invalid_request")?;
    let identity_path = std::path::PathBuf::from(
        std::env::var_os("USERPROFILE").ok_or("machine.platform_unsupported")?,
    )
    .join(".herdr-kanban/identity");
    let identity = std::fs::read_to_string(identity_path).map_err(|_| "machine.setup_required")?;
    let identity = identity.trim();
    if envelope["version"] != 1 || envelope["host_identity"].as_str() != Some(identity) {
        return Err("machine.identity_changed".into());
    }
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = envelope["session"]
        .as_str()
        .ok_or("machine.invalid_request")?
        .into();
    let result = execute(&cli, &envelope["request"]);
    let response = match result {
        Ok(value) => json!({"version":1,"host_identity":identity,"result":value,"error":null}),
        Err(error) => json!({"version":1,"host_identity":identity,"result":null,"error":error}),
    };
    serde_json::to_writer(std::io::stdout().lock(), &response).map_err(|e| e.to_string())?;
    std::io::stdout().flush().map_err(|e| e.to_string())
}

fn execute(cli: &crate::herdr::Cli, request: &Value) -> Result<Value, String> {
    use crate::execution::{LOCAL, Resources};
    use crate::workflow::Herdr;
    let text = |key| {
        request[key]
            .as_str()
            .ok_or_else(|| "machine.invalid_request".to_string())
    };
    let encode = |result| serde_json::to_value(result).map_err(|e| e.to_string());
    match request["method"]
        .as_str()
        .ok_or("machine.invalid_request")?
    {
        "cli_output" => {
            let args: Vec<String> = serde_json::from_value(request["args"].clone())
                .map_err(|_| "machine.invalid_request")?;
            // Global options cannot override the identity-checked session.
            if args.first().is_none_or(|arg| {
                !matches!(
                    arg.as_str(),
                    "workspace" | "pane" | "agent" | "status" | "tab"
                )
            }) {
                return Err("machine.invalid_request".into());
            }
            let output = match cli.output(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
                Ok(stdout) => CliOutput {
                    stdout,
                    error: None,
                    uncertain: false,
                },
                Err(error) => CliOutput {
                    stdout: vec![],
                    error: Some(error.message),
                    uncertain: error.uncertain,
                },
            };
            serde_json::to_value(output).map_err(|e| e.to_string())
        }
        "hello" => Ok(
            json!({"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"host":std::env::var("COMPUTERNAME").unwrap_or_default(),"version":1}),
        ),
        "preflight" => {
            cli.workspaces()?;
            let mut codex = std::process::Command::new("cmd");
            codex.args(["/d", "/c", "codex", "--version"]);
            let version = crate::remote_transport::run(codex, Vec::new(), 15)
                .map_err(|e| format!("machine.codex_unavailable: {e}"))?;
            Ok(
                json!({"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"user":std::env::var("USERNAME").unwrap_or_default(),"host":std::env::var("COMPUTERNAME").unwrap_or_default(),"codex":String::from_utf8_lossy(&version).trim(),"session":cli.session,"version":1}),
            )
        }
        "same_directory" => Ok(json!(match (
            std::fs::canonicalize(text("a")?),
            std::fs::canonicalize(text("b")?)
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        })),
        "workspaces" => serde_json::to_value(cli.workspaces()?).map_err(|e| e.to_string()),
        "image" => {
            use base64::Engine;
            let bytes = request["data"]
                .as_str()
                .map(|data| {
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|_| "image.format")
                })
                .transpose()?;
            let root = std::path::PathBuf::from(
                std::env::var_os("USERPROFILE").ok_or("machine.platform_unsupported")?,
            )
            .join(".herdr-kanban/images");
            Ok(json!(crate::remote_images::store(
                &root,
                text("digest")?,
                text("extension")?,
                request["size"].as_u64().ok_or("machine.invalid_request")?,
                bytes.as_deref()
            )?))
        }
        "move_workspace" => {
            serde_json::to_value(cli.move_workspace(text("id")?, request["before"].as_str())?)
                .map_err(|e| e.to_string())
        }
        "session_id" | "session_locks" | "prompt_receipt" => {
            let mut target: crate::workflow::Target =
                serde_json::from_value(request["target"].clone())
                    .map_err(|_| "machine.invalid_request")?;
            target.machine_id = crate::machines::local_id();
            match text("method")? {
                "session_id" => Ok(json!(cli.session_id(&target)?)),
                "session_locks" => Ok(json!(cli.startup_session_locks(&target)?)),
                _ => Ok(json!(cli.prompt_receipt(
                    &target,
                    text("prompt")?,
                    text("after")?,
                    request["expected"].as_str()
                )?)),
            }
        }
        "read_text" => encode(LOCAL.read_text(text("path")?)?),
        "directory_exists" => Ok(json!(LOCAL.directory_exists(text("path")?)?)),
        "search_files" => serde_json::to_value(LOCAL.search_files(text("root")?, text("query")?)?)
            .map_err(|e| e.to_string()),
        "search_skills" => {
            serde_json::to_value(LOCAL.search_skills(request["root"].as_str(), text("query")?)?)
                .map_err(|e| e.to_string())
        }
        "git_statuses" => serde_json::to_value(
            LOCAL.git_statuses(
                serde_json::from_value(request["projects"].clone())
                    .map_err(|_| "machine.invalid_request")?,
            )?,
        )
        .map_err(|e| e.to_string()),
        _ => Err("machine.unsupported_method".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_resource_requests_keep_paths_as_data_and_return_real_file_contents() {
        let root = std::env::temp_dir().join(format!("kanban-remote-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("中文 $variable.txt");
        std::fs::write(&path, "remote reference\n第二行").unwrap();
        let cli = crate::herdr::Cli::from_env();
        assert_eq!(
            execute(&cli, &json!({"method":"read_text","path":path})).unwrap(),
            json!("remote reference\n第二行")
        );
        assert_eq!(
            execute(&cli, &json!({"method":"directory_exists","path":root})).unwrap(),
            json!(true)
        );
        let files = execute(
            &cli,
            &json!({"method":"search_files","root":root,"query":"中文"}),
        )
        .unwrap();
        assert_eq!(files.as_array().unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
