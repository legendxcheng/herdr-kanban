//! Bounded, read-only Codex receipt evidence. Terminal text and status are not receipts.
use super::Cli;
use crate::workflow::{Herdr, PromptReceipt, Target};
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_TAIL: u64 = 16 * 1024 * 1024;
const MAX_HEADER: u64 = 1024 * 1024;

impl Cli {
    pub(super) fn read_prompt_receipt(
        &self,
        target: &Target,
        prompt: &str,
        after: &str,
        expected: Option<&str>,
    ) -> Result<Option<PromptReceipt>, String> {
        if target.agent != "codex" {
            return Ok(None);
        }
        let Some(id) = self.session_id(target)? else {
            return Ok(None);
        };
        uuid::Uuid::parse_str(&id).map_err(|_| "flow.session_invalid")?;
        if expected.is_some_and(|old| old != id) {
            return Err("flow.session_mismatch".into());
        }
        let Some(root) = codex_home() else {
            return Ok(None);
        };
        let Some(path) = find_session(&root.join("sessions"), &id) else {
            return Ok(None);
        };
        let after = DateTime::parse_from_rfc3339(after).map_err(|_| "dispatch.verify_required")?;
        let receipt = read_receipt(&path, &id, prompt, after, Utc::now().fixed_offset());
        // A /resume or process replacement during file inspection invalidates this evidence.
        if self.session_id(target)?.as_deref() != Some(id.as_str()) {
            return Err("flow.session_mismatch".into());
        }
        Ok(receipt.map(|timestamp| PromptReceipt {
            session_id: id.clone(),
            evidence: format!("Codex session {id}: exact user message accepted at {timestamp}"),
        }))
    }
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|p| PathBuf::from(p).join(".codex"))
        })
}

fn find_session(root: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut found = None;
    for (index, entry) in walkdir::WalkDir::new(root)
        .max_depth(4)
        .into_iter()
        .enumerate()
    {
        if index >= 100_000 || Instant::now() > deadline {
            return None;
        }
        let entry = entry.ok()?;
        if entry.file_type().is_file() && entry.file_name().to_str()?.ends_with(&suffix) {
            if found.is_some() {
                return None;
            }
            found = Some(entry.into_path());
        }
    }
    found
}

fn read_receipt(
    path: &Path,
    id: &str,
    prompt: &str,
    after: DateTime<FixedOffset>,
    until: DateTime<FixedOffset>,
) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut header = String::new();
    BufReader::new((&mut file).take(MAX_HEADER))
        .read_line(&mut header)
        .ok()?;
    let meta: Value = serde_json::from_str(&header).ok()?;
    if meta["type"] != "session_meta" || meta["payload"]["id"] != id {
        return None;
    }
    let offset = size.saturating_sub(MAX_TAIL);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut reader = BufReader::new(file.take(size - offset));
    if offset > 0 {
        reader.read_until(b'\n', &mut Vec::new()).ok()?;
    }
    let mut accepted = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        // An actively written, incomplete final record cannot confirm anything.
        if !line.ends_with('\n') {
            break;
        }
        let row: Value = serde_json::from_str(&line).ok()?;
        let payload = &row["payload"];
        if row["type"] != "response_item"
            || payload["type"] != "message"
            || payload["role"] != "user"
        {
            continue;
        }
        let timestamp = row["timestamp"].as_str()?;
        let time = DateTime::parse_from_rfc3339(timestamp).ok()?;
        if time <= after || time > until {
            continue;
        }
        let content = payload["content"].as_array()?;
        let texts: Option<Vec<&str>> = content
            .iter()
            .map(|part| {
                if part["type"] == "input_text" {
                    part["text"].as_str()
                } else {
                    None
                }
            })
            .collect();
        let Some(texts) = texts else { continue };
        if texts.join("\n") == prompt {
            // Multiple matching new messages are ambiguous; never guess which send succeeded.
            if accepted.is_some() {
                return None;
            }
            accepted = Some(timestamp.to_owned());
        }
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn check(rows: Vec<Value>, expected_id: &str, prompt: &str) -> Option<String> {
        let path =
            std::env::temp_dir().join(format!("kanban-receipt-{}.jsonl", uuid::Uuid::new_v4()));
        let text = rows
            .into_iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>();
        std::fs::write(&path, text).unwrap();
        let result = read_receipt(
            &path,
            expected_id,
            prompt,
            DateTime::parse_from_rfc3339("2026-10-25T07:24:05Z").unwrap(),
            DateTime::parse_from_rfc3339("2026-10-25T07:25:00Z").unwrap(),
        );
        std::fs::remove_file(path).unwrap();
        result
    }
    fn meta() -> Value {
        json!({"type":"session_meta","payload":{"id":"session-a"}})
    }
    fn message(time: &str, role: &str, text: &str) -> Value {
        json!({"timestamp":time,"type":"response_item","payload":{"type":"message","role":role,"content":[{"type":"input_text","text":text}]}})
    }
    #[test]
    fn exact_received_prompt_confirms_without_a_state_change() {
        assert!(
            check(
                vec![
                    meta(),
                    message("2026-10-25T07:24:08.525Z", "user", "中文\n$skill")
                ],
                "session-a",
                "中文\n$skill"
            )
            .is_some()
        );
    }
    #[test]
    fn stale_future_wrong_session_partial_and_assistant_messages_do_not_confirm() {
        for (time, role, text, session) in [
            ("2026-10-25T07:24:04Z", "user", "exact", "session-a"),
            ("2026-10-25T07:25:01Z", "user", "exact", "session-a"),
            ("2026-10-25T07:24:08Z", "assistant", "exact", "session-a"),
            ("2026-10-25T07:24:08Z", "user", "exact extra", "session-a"),
            ("2026-10-25T07:24:08Z", "user", "exact", "session-b"),
        ] {
            assert!(check(vec![meta(), message(time, role, text)], session, "exact").is_none());
        }
    }
    #[test]
    fn duplicate_new_messages_are_ambiguous_but_old_identical_messages_are_ignored() {
        let new = message("2026-10-25T07:24:08Z", "user", "exact");
        assert!(check(vec![meta(), new.clone(), new.clone()], "session-a", "exact").is_none());
        assert!(
            check(
                vec![
                    meta(),
                    message("2026-10-25T07:24:04Z", "user", "exact"),
                    new
                ],
                "session-a",
                "exact"
            )
            .is_some()
        );
    }
}
