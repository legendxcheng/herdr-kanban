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
const RECEIPT_GRACE: Duration = Duration::from_secs(30);
const RECEIPT_INTERVAL: Duration = Duration::from_millis(500);
const RECEIPT_POLLS: usize = 60;
const OBSERVATION_RETRIES: usize = 2;
const OBSERVATION_RETRY_DELAY: Duration = Duration::from_millis(300);

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

fn observe_recovering(
    observe: &mut impl FnMut() -> Result<SubmissionState, SendError>,
    pause: &mut impl FnMut(Duration),
    retries: &mut usize,
    stage: &str,
) -> Result<SubmissionState, SendError> {
    loop {
        match observe() {
            Ok(state) => return Ok(state),
            Err(error) => {
                crate::dispatch_trace::record(stage, "observation_failed", Some(&error.message));
                if *retries == 0
                    || !crate::remote_control::transient_observation_error(&error.message)
                    || !crate::remote_control::recover_observation()
                {
                    return Err(error);
                }
                *retries -= 1;
                crate::dispatch_trace::record(stage, "observation_retry", Some(&error.message));
                pause(OBSERVATION_RETRY_DELAY);
                // Production observe checks the current operation before and after
                // the read and revalidates the complete target on every attempt.
            }
        }
    }
}

fn write_input(
    request: &mut impl FnMut(&[&str]) -> Result<(), SendError>,
    stage: &str,
    args: &[&str],
) -> Result<(), SendError> {
    crate::dispatch_trace::record(stage, "requested", None);
    let result = request(args);
    crate::dispatch_trace::record(
        stage,
        if result.is_ok() {
            "acknowledged"
        } else {
            "failed"
        },
        result.as_ref().err().map(|error| error.message.as_str()),
    );
    result
}

fn submission_error(mut error: SendError, stage: &str, enter_requested: bool) -> SendError {
    if !enter_requested {
        error.message = format!(
            "dispatch.enter_not_sent_verify_required: {stage}: {}",
            error.message
        );
    } else if error.message != "dispatch.submit_verify_required" {
        error.message = format!(
            "dispatch.enter_unknown_verify_required: {stage}: {}",
            error.message
        );
    }
    uncertain(error)
}

impl Cli {
    pub(super) fn submit_windows_codex(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
        received: &dyn Fn() -> Result<bool, String>,
    ) -> Result<(), SendError> {
        submit_observed(
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
            || {
                check_current().map_err(|message| error(&message))?;
                let found = received().map_err(|message| error(&message))?;
                check_current().map_err(|message| error(&message))?;
                Ok(found)
            },
        )
    }
}

#[cfg(test)]
fn submit(
    pane: &str,
    prompt: &str,
    request: impl FnMut(&[&str]) -> Result<(), SendError>,
    observe: impl FnMut() -> Result<SubmissionState, SendError>,
    pause: impl FnMut(Duration),
) -> Result<(), SendError> {
    submit_observed(pane, prompt, request, observe, pause, || Ok(false))
}

fn submit_observed(
    pane: &str,
    prompt: &str,
    mut request: impl FnMut(&[&str]) -> Result<(), SendError>,
    mut observe: impl FnMut() -> Result<SubmissionState, SendError>,
    mut pause: impl FnMut(Duration),
    mut received: impl FnMut() -> Result<bool, SendError>,
) -> Result<(), SendError> {
    let mut retries = OBSERVATION_RETRIES;
    // A busy turn cannot acknowledge this particular prompt; reject before writing.
    observe_recovering(&mut observe, &mut pause, &mut retries, "before_text")?.ensure_idle()?;
    // send-text is raw terminal input, not a paste. Codex can discard raw LF
    // key events, so frame multiline content as one literal bracketed paste.
    // Keep the delimiters out of the durable prompt and exact receipt comparison.
    let pasted;
    let input = if prompt.contains(['\r', '\n', '\t']) {
        pasted = format!("\u{1b}[200~{prompt}\u{1b}[201~");
        &pasted
    } else {
        prompt
    };
    write_input(&mut request, "text", &["pane", "send-text", pane, input]).map_err(|error| {
        if error.uncertain {
            submission_error(error, "text", false)
        } else {
            error
        }
    })?;
    let mut stage = "after_text";
    let mut enter_requested = false;
    let result = (|| {
        observe_recovering(&mut observe, &mut pause, &mut retries, stage)?.ensure_idle()?;
        // A trailing skill invocation can consume a batched Right+Enter. Give the
        // non-character paste boundary its own write and processing interval.
        stage = "right";
        write_input(&mut request, stage, &["agent", "send-keys", pane, "right"])?;
        pause(INPUT_SETTLE);
        stage = "after_right";
        observe_recovering(&mut observe, &mut pause, &mut retries, stage)?.ensure_idle()?;
        // An exact trailing $skill still opens Codex's suggestion list. Enter
        // would accept the suggestion instead of submitting the prompt.
        stage = "escape";
        write_input(&mut request, stage, &["agent", "send-keys", pane, "esc"])?;
        pause(SUGGESTION_SETTLE);
        stage = "before_enter";
        let before_enter = observe_recovering(&mut observe, &mut pause, &mut retries, stage)?;
        before_enter.ensure_idle()?;
        stage = "enter";
        enter_requested = true;
        write_input(&mut request, stage, &["agent", "send-keys", pane, "enter"])?;
        // The prepare flow independently waits for the exact process to exit.
        if prompt.trim() == "/quit" {
            return Ok(());
        }
        let deadline = Instant::now() + ACK_TIMEOUT;
        stage = "ack";
        for poll in 0..=ACK_POLLS {
            if observe_recovering(&mut observe, &mut pause, &mut retries, stage)?
                .advanced_from(&before_enter)
            {
                crate::dispatch_trace::record(stage, "state_advanced", None);
                return Ok(());
            }
            // A missing status event need not consume the whole 10-second window.
            // This is only a hint to the workflow's existing exact-receipt path:
            // it must revalidate and commit evidence/session/state atomically.
            if poll % 5 == 0 && received()? {
                crate::dispatch_trace::record("receipt", "found", None);
                return Err(error("dispatch.submit_verify_required"));
            }
            if poll == ACK_POLLS || Instant::now() >= deadline {
                break;
            }
            pause(ACK_INTERVAL);
        }
        // Startup/account initialization can outlive the status-ack window.
        // Continue only read-only, exact receipt probes. Late status changes alone
        // are not enough, and neither the body nor Enter is ever sent again.
        let deadline = Instant::now() + RECEIPT_GRACE;
        stage = "receipt";
        for poll in 0..=RECEIPT_POLLS {
            if received()? {
                crate::dispatch_trace::record(stage, "found", None);
                // Workflow revalidates identity and commits the receipt atomically.
                return Err(error("dispatch.submit_verify_required"));
            }
            if poll == RECEIPT_POLLS || Instant::now() >= deadline {
                break;
            }
            pause(RECEIPT_INTERVAL);
        }
        Err(error("dispatch.submit_verify_required"))
    })();
    // Once text was written, failures retain ownership. Never replay text or Enter.
    if let Err(error) = &result {
        crate::dispatch_trace::record(stage, "unconfirmed", Some(&error.message));
    }
    result.map_err(|error| submission_error(error, stage, enter_requested))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn transient_observation_after_text_recovers_without_replaying_input() {
        for failed_read in 2..=4 {
            let mut reads = 0;
            let mut writes = Vec::new();
            let result = submit(
                "pane",
                "once",
                |args| {
                    writes.push(args[1..].iter().map(|s| s.to_string()).collect::<Vec<_>>());
                    Ok(())
                },
                || {
                    reads += 1;
                    if reads == failed_read {
                        return Err(error("machine.connection_failed"));
                    }
                    Ok(state(if reads > 5 { 2 } else { 1 }, "idle"))
                },
                |_| {},
            );
            assert!(
                result.is_ok(),
                "temporary read failure interrupted input: {:?}",
                result.err().map(|e| e.message)
            );
            assert_eq!(writes.len(), 4);
            assert_eq!(writes[0], ["send-text", "pane", "once"]);
            assert_eq!(writes[3], ["send-keys", "pane", "enter"]);
        }
    }

    #[test]
    fn exhausted_observation_reports_enter_not_requested_and_keeps_text_once() {
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
                if reads > 1 {
                    Err(error("machine.timeout"))
                } else {
                    Ok(state(1, "idle"))
                }
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(
            reads, 4,
            "initial observation plus at most three attempts after text"
        );
        assert_eq!(writes, 1);
        assert!(error.uncertain);
        assert!(
            error
                .message
                .starts_with("dispatch.enter_not_sent_verify_required:")
        );
    }

    fn state(sequence: u64, status: &str) -> SubmissionState {
        SubmissionState {
            sequence,
            status: status.into(),
            ready: true,
        }
    }

    #[test]
    fn multiline_submission_reaches_the_composer_as_one_literal_paste() {
        let prompt = "第一行\n\n第二行\t保留缩进\n末尾 $kanban-dev";
        let mut pasted = String::new();
        let mut enters = 0;
        let mut observations = 0;
        submit(
            "pane",
            prompt,
            |args| {
                if args.starts_with(&["pane", "send-text"]) {
                    // Codex treats raw LF input as key events. Only a bracketed
                    // paste transports literal multiline text into the composer.
                    pasted = args[3]
                        .strip_prefix("\u{1b}[200~")
                        .and_then(|text| text.strip_suffix("\u{1b}[201~"))
                        .unwrap_or("")
                        .to_owned();
                }
                if args.last() == Some(&"enter") {
                    enters += 1;
                }
                Ok(())
            },
            || {
                observations += 1;
                Ok(state(if observations > 4 { 2 } else { 1 }, "idle"))
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(pasted, prompt);
        assert_eq!(enters, 1);
    }

    #[test]
    fn receipt_arriving_after_ack_timeout_is_checked_without_more_input() {
        let writes = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let found = Cell::new(false);
        let result = submit_observed(
            "pane",
            "delayed",
            |_| {
                writes.set(writes.get() + 1);
                Ok(())
            },
            || Ok(state(1, "idle")),
            |duration| elapsed.set(elapsed.get() + duration),
            || {
                let accepted = elapsed.get() >= Duration::from_secs(23);
                found.set(accepted);
                Ok(accepted)
            },
        );
        assert!(found.get(), "late exact receipt was never checked");
        // The workflow rechecks and atomically commits this evidence.
        assert_eq!(
            result.unwrap_err().message,
            "dispatch.submit_verify_required"
        );
        assert_eq!(writes.get(), 4);
    }

    #[test]
    fn receipt_grace_stops_on_expiry_or_stale_operation_without_replaying() {
        for cancelled in [false, true] {
            let writes = Cell::new(0);
            let elapsed = Cell::new(Duration::ZERO);
            let result = submit_observed(
                "pane",
                "once",
                |_| {
                    writes.set(writes.get() + 1);
                    Ok(())
                },
                || Ok(state(1, "idle")),
                |duration| elapsed.set(elapsed.get() + duration),
                || {
                    if cancelled && elapsed.get() >= Duration::from_secs(15) {
                        Err(error("dispatch.operation_conflict"))
                    } else {
                        Ok(false)
                    }
                },
            )
            .unwrap_err();
            assert!(result.uncertain);
            assert_eq!(writes.get(), 4);
            if cancelled {
                assert_eq!(
                    result.message,
                    "dispatch.enter_unknown_verify_required: receipt: dispatch.operation_conflict"
                );
                assert!(elapsed.get() < Duration::from_secs(16));
            } else {
                assert_eq!(result.message, "dispatch.submit_verify_required");
                assert!(elapsed.get() >= Duration::from_secs(40));
                assert!(elapsed.get() < Duration::from_secs(41));
            }
        }
    }

    #[test]
    fn exact_receipt_hint_ends_stale_ack_wait_after_one_enter_without_resending() {
        let writes = Cell::new(0);
        let polls = Cell::new(0);
        let error = submit_observed(
            "pane",
            "测试",
            |_| {
                writes.set(writes.get() + 1);
                Ok(())
            },
            || Ok(state(9, "idle")),
            |duration| {
                if duration == ACK_INTERVAL {
                    polls.set(polls.get() + 1);
                }
            },
            || {
                assert_eq!(writes.get(), 4);
                Ok(true)
            },
        )
        .unwrap_err();
        assert!(error.uncertain);
        assert_eq!(error.message, "dispatch.submit_verify_required");
        assert_eq!(writes.get(), 4);
        assert_eq!(polls.get(), 0);
    }

    #[test]
    fn receipt_read_failure_keeps_submission_uncertain_and_does_not_write_again() {
        let writes = Cell::new(0);
        let error = submit_observed(
            "pane",
            "测试",
            |_| {
                writes.set(writes.get() + 1);
                Ok(())
            },
            || Ok(state(9, "idle")),
            |_| {},
            || Err(super::error("binding.identity_changed")),
        )
        .unwrap_err();
        assert!(error.uncertain);
        assert_eq!(writes.get(), 4);
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
            assert!(error.message.starts_with(if failing_read <= 4 {
                "dispatch.enter_not_sent_verify_required:"
            } else {
                "dispatch.enter_unknown_verify_required:"
            }));
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
            assert!(error.message.starts_with(if failing_write < 4 {
                "dispatch.enter_not_sent_verify_required:"
            } else {
                "dispatch.enter_unknown_verify_required:"
            }));
        }
    }

    #[test]
    fn cancellation_or_busy_state_after_a_transient_read_prevents_remaining_input() {
        for cause in [
            "task.missing",
            "dispatch.operation_conflict",
            "binding.identity_changed",
            "busy",
        ] {
            let mut writes = 0;
            let mut reads = 0;
            let result = submit(
                "pane",
                "once",
                |_| {
                    writes += 1;
                    Ok(())
                },
                || {
                    reads += 1;
                    match reads {
                        1 => Ok(state(1, "idle")),
                        2 => Err(error("machine.timeout")),
                        _ if cause == "busy" => Ok(state(2, "working")),
                        _ => Err(error(cause)),
                    }
                },
                |_| {},
            )
            .unwrap_err();
            assert!(result.uncertain);
            assert_eq!(reads, 3);
            assert_eq!(writes, 1);
        }
    }

    #[test]
    fn transient_observations_share_one_retry_budget_across_the_submission() {
        let mut reads = 0;
        let mut writes = 0;
        let result = submit(
            "pane",
            "once",
            |_| {
                writes += 1;
                Ok(())
            },
            || {
                reads += 1;
                if matches!(reads, 2 | 4 | 6) {
                    Err(error("herdr.timeout"))
                } else {
                    Ok(state(1, "idle"))
                }
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(reads, 6);
        assert_eq!(
            writes, 3,
            "text/right/escape were acknowledged; Enter was never requested"
        );
        assert!(
            result
                .message
                .starts_with("dispatch.enter_not_sent_verify_required:")
        );
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
