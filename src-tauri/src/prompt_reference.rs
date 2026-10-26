//! Lexical paths only: never read a reference on the Kanban host.
fn normalize(value: &str) -> String {
    let path = value.replace('\\', "/");
    let path = if let Some(rest) = path.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else {
        path.strip_prefix("//?/").unwrap_or(&path).to_string()
    };
    let prefix = if path.starts_with("//") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in path[prefix.len()..].split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".."
            && parts
                .last()
                .is_some_and(|last| *last != ".." && !last.ends_with(':'))
        {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    format!("{prefix}{}", parts.join("/"))
}

pub fn workspace_relative_path(file: &str, root: &str) -> String {
    let path = normalize(file);
    let base = normalize(root);
    let base = base.trim_end_matches('/');
    let windows = path.starts_with("//")
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'/'));
    let prefix = format!("{base}/");
    let matches = if windows {
        path.get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(&prefix))
    } else {
        path.starts_with(&prefix)
    };
    // Legacy references outside this Workspace retain their real location.
    if !root.is_empty() && matches {
        path[prefix.len()..].into()
    } else {
        path
    }
}

/// Match Codex CLI's insert_selected_path, without adding separators to the document.
pub fn file_reference(file: &str, root: &str) -> String {
    let path = workspace_relative_path(file, root);
    if path.chars().any(char::is_whitespace) && !path.contains('"') {
        format!("\"{path}\"")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_workspace_path_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../src/promptPathCases.json")).unwrap();
        for case in cases.as_array().unwrap() {
            assert_eq!(
                super::workspace_relative_path(
                    case["file"].as_str().unwrap(),
                    case["root"].as_str().unwrap()
                ),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
            assert_eq!(
                super::file_reference(
                    case["file"].as_str().unwrap(),
                    case["root"].as_str().unwrap()
                ),
                case["formatted"].as_str().unwrap(),
                "{case}"
            );
        }
    }
}
