//! Windows Codex input boundary and bounded acknowledgement of a new submission.
use super::Cli;
use crate::workflow::{SendError, Target};
use serde_json::Value;
use std::time::{Duration, Instant};

const INPUT_SETTLE: Duration = Duration::from_millis(300);
const SUGGESTION_SETTLE: Duration = Duration::from_millis(100);
const ACK_INTERVAL: Duration = Duration::from_millis(200);
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
const ACK_POLLS: usize = 50;

struct SubmissionState {
    sequence: u64,
    status: String,
    ready: bool,
}

impl SubmissionState {
    fn parse(agent: &Value) -> Result<Self, SendError> {
        Ok(Self {
            sequence: agent["state_change_seq"]
                .as_u64()
                .ok_or_else(|| error("herdr.invalid_response"))?,
            status: agent["agent_status"]
                .as_str()
                .ok_or_else(|| error("herdr.invalid_response"))?
                .into(),
            ready: agent["interactive_ready"] == true && agent["launch_pending"] != true,
        })
    }

    fn ensure_idle(&self) -> Result<(), SendError> {
        if self.ready && matches!(self.status.as_str(), "idle" | "done") {
            Ok(())
        } else {
            Err(error("flow.not_idle"))
        }
    }

    fn advanced_from(&self, before: &Self) -> bool {
        self.sequence > before.sequence
            && matches!(
                self.status.as_str(),
                "working" | "blocked" | "idle" | "done"
            )
    }
}

fn error(message: &str) -> SendError {
    SendError {
        message: message.into(),
        uncertain: false,
    }
}

fn uncertain(mut error: SendError) -> SendError {
    error.uncertain = true;
    error
}

impl Cli {
    pub(super) fn submit_windows_codex(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        submit(
            &target.pane_id,
            prompt,
            |args| {
                check_current().map_err(|message| error(&message))?;
                self.request(args).map(|_| ())
            },
            || {
                check_current().map_err(|message| error(&message))?;
                let doc = self
                    .json(&["agent", "get", &target.pane_id])
                    .map_err(|message| error(&message))?;
                let agent = doc
                    .pointer("/result/agent")
                    .ok_or_else(|| error("herdr.invalid_response"))?;
                let live = self
                    .live_target(&target.workspace_id, agent)
                    .map_err(|message| error(&message))?;
                if !live.same_identity(target) {
                    return Err(error("binding.identity_changed"));
                }
                check_current().map_err(|message| error(&message))?;
                SubmissionState::parse(agent)
            },
            std::thread::sleep,
        )
    }
}

fn submit(
    pane: &str,
    prompt: &str,
    mut request: impl FnMut(&[&str]) -> Result<(), SendError>,
    mut observe: impl FnMut() -> Result<SubmissionState, SendError>,
    mut pause: impl FnMut(Duration),
) -> Result<(), SendError> {
    // A busy turn cannot acknowledge this particular prompt; reject before writing.
    observe()?.ensure_idle()?;
    request(&["pane", "send-text", pane, prompt])?;
    let result = (|| {
        observe()?.ensure_idle()?;
        // A trailing skill invocation can consume a batched Right+Enter. Give the
        // non-character paste boundary its own write and processing interval.
        request(&["agent", "send-keys", pane, "right"])?;
        pause(INPUT_SETTLE);
        observe()?.ensure_idle()?;
        // An exact trailing $skill still opens Codex's suggestion list. Enter
        // would accept the suggestion instead of submitting the prompt.
        request(&["agent", "send-keys", pane, "esc"])?;
        pause(SUGGESTION_SETTLE);
        let before_enter = observe()?;
        before_enter.ensure_idle()?;
        request(&["agent", "send-keys", pane, "enter"])?;
        // The prepare flow independently waits for the exact process to exit.
        if prompt.trim() == "/quit" {
            return Ok(());
        }
        let deadline = Instant::now() + ACK_TIMEOUT;
        for poll in 0..=ACK_POLLS {
            if observe()?.advanced_from(&before_enter) {
                return Ok(());
            }
            if poll == ACK_POLLS || Instant::now() >= deadline {
                break;
            }
            pause(ACK_INTERVAL);
        }
        Err(error("dispatch.submit_verify_required"))
    })();
    // Once text was written, failures retain ownership. Never replay text or Enter.
    result.map_err(uncertain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn state(sequence: u64, status: &str) -> SubmissionState {
        SubmissionState {
            sequence,
            status: status.into(),
            ready: true,
        }
    }

    #[test]
    fn skill_prompt_waits_between_boundary_and_one_enter_then_confirms_a_new_turn() {
        let mut calls = Vec::new();
        let mut observations = 0;
        let mut pauses = Vec::new();
        let prompt = "右侧的【已完成】区域 需要补上一键归档或一键删除  $kanban-dev";
        submit(
            "w1:p1",
            prompt,
            |args| {
                calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                Ok(())
            },
            || {
                observations += 1;
                Ok(if observations <= 5 {
                    state(9, "idle")
                } else {
                    state(10, "working")
                })
            },
            |duration| pauses.push(duration),
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            calls,
            vec![
                vec!["pane", "send-text", "w1:p1", prompt],
                vec!["agent", "send-keys", "w1:p1", "right"],
                vec!["agent", "send-keys", "w1:p1", "esc"],
                vec!["agent", "send-keys", "w1:p1", "enter"],
            ]
        );
        assert_eq!(pauses, vec![INPUT_SETTLE, SUGGESTION_SETTLE, ACK_INTERVAL]);
    }

    #[test]
    fn successful_input_requests_without_a_new_turn_remain_uncertain_and_never_replay() {
        let mut calls = Vec::new();
        let error = submit(
            "pane",
            "once",
            |args| {
                calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
                Ok(())
            },
            || Ok(state(7, "done")),
            |_| {},
        )
        .unwrap_err();
        assert!(error.uncertain);
        assert_eq!(error.message, "dispatch.submit_verify_required");
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls
                .iter()
                .filter(|args| args.iter().any(|arg| arg == "enter"))
                .count(),
            1
        );
    }

    #[test]
    fn failures_after_text_at_each_boundary_preserve_uncertainty() {
        for failing_read in 2..=5 {
            let mut reads = 0;
            let mut writes = 0;
            let error = submit(
                "pane",
                "once",
                |_| {
                    writes += 1;
                    Ok(())
                },
                || {
                    reads += 1;
                    if reads == failing_read {
                        Err(super::error("binding.identity_changed"))
                    } else {
                        Ok(state(1, "idle"))
                    }
                },
                |_| {},
            )
            .unwrap_err();
            assert!(error.uncertain);
            assert_eq!(writes, failing_read - 1);
        }
        for failing_write in 2..=4 {
            let mut writes = 0;
            let error = submit(
                "pane",
                "once",
                |_| {
                    writes += 1;
                    if writes == failing_write {
                        Err(super::error("herdr.unavailable"))
                    } else {
                        Ok(())
                    }
                },
                || Ok(state(1, "idle")),
                |_| {},
            )
            .unwrap_err();
            assert!(error.uncertain);
            assert_eq!(writes, failing_write);
        }
    }

    #[test]
    fn already_busy_or_unready_agents_are_rejected_before_writing() {
        for status in ["working", "blocked", "unknown"] {
            let error = submit(
                "pane",
                "once",
                |_| panic!("must not write"),
                || Ok(state(1, status)),
                |_| {},
            )
            .unwrap_err();
            assert!(!error.uncertain);
            assert_eq!(error.message, "flow.not_idle");
        }
        let error = submit(
            "pane",
            "once",
            |_| panic!("must not write"),
            || {
                Ok(SubmissionState {
                    ready: false,
                    ..state(1, "idle")
                })
            },
            |_| {},
        )
        .unwrap_err();
        assert!(!error.uncertain);
    }

    #[test]
    fn acknowledgement_handles_fast_completion_but_not_stale_or_unknown_states() {
        for status in ["working", "blocked", "idle", "done"] {
            assert!(state(2, status).advanced_from(&state(1, "idle")));
            assert!(!state(1, status).advanced_from(&state(1, "idle")));
        }
        assert!(!state(2, "unknown").advanced_from(&state(1, "idle")));
        assert!(SubmissionState::parse(&serde_json::json!({"agent_status":"idle"})).is_err());
    }

    #[test]
    fn quit_does_not_wait_for_a_turn_from_the_exiting_process() {
        let reads = Cell::new(0);
        submit(
            "pane",
            "/quit",
            |_| Ok(()),
            || {
                reads.set(reads.get() + 1);
                assert!(reads.get() <= 4);
                Ok(state(1, "idle"))
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
    }
}
