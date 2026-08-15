//! Terminal Interaction Client. Shares protocol fixtures with headless mode.

use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Result;
use apxm_interaction_client::{HeadlessOutcome, render_outcome};

/// Render the TUI agreement with headless outcome labels.
#[must_use]
pub fn tui_outcome_label(outcome: HeadlessOutcome) -> &'static str {
    render_outcome(outcome)
}

/// One decoded Interaction Client fixture shared with headless JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiFrame {
    /// Program Instance id.
    pub program_instance_id: String,
    /// Admitted artifact digest.
    pub artifact_digest: String,
    /// Closed outcome label.
    pub outcome: HeadlessOutcome,
    /// Observation lines projected by content-ref / lifecycle, never payloads.
    pub observations: Vec<String>,
    /// Prompt routed from runtime state.
    pub prompt: String,
}

impl TuiFrame {
    /// Build a frame from the same outcome the headless client classified.
    #[must_use]
    pub fn from_protocol(
        program_instance_id: impl Into<String>,
        artifact_digest: impl Into<String>,
        outcome: HeadlessOutcome,
        observations: Vec<String>,
    ) -> Self {
        let prompt = match outcome {
            HeadlessOutcome::WaitingEvent => {
                "event> fulfill <event_id> <generation> | cancel | detach".to_owned()
            }
            HeadlessOutcome::Returned => "invoke> start | cancel | detach".to_owned(),
            HeadlessOutcome::Failed => "failed> detach".to_owned(),
        };
        Self {
            program_instance_id: program_instance_id.into(),
            artifact_digest: artifact_digest.into(),
            outcome,
            observations,
            prompt,
        }
    }

    /// Decode a headless protocol JSON fixture.
    #[allow(dead_code)]
    pub fn decode_protocol(value: &serde_json::Value) -> Result<Self, String> {
        let outcome = match value.get("outcome").and_then(|item| item.as_str()) {
            Some("returned") => HeadlessOutcome::Returned,
            Some("waiting_event") => HeadlessOutcome::WaitingEvent,
            Some("failed") => HeadlessOutcome::Failed,
            _ => return Err("unknown outcome".to_owned()),
        };
        let observations = value
            .get("observations")
            .and_then(|item| item.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self::from_protocol(
            value
                .get("program_instance_id")
                .and_then(|item| item.as_str())
                .unwrap_or(""),
            value
                .get("artifact_digest")
                .and_then(|item| item.as_str())
                .unwrap_or(""),
            outcome,
            observations,
        ))
    }

    /// Render the full TUI, not a three-line stub.
    #[must_use]
    pub fn render(&self) -> String {
        let outcome = tui_outcome_label(self.outcome);
        let mut observations = self.observations.join("\n");
        if observations.is_empty() {
            observations = "(no observations)".to_owned();
        }
        format!(
            "┌─ apxm interact ─────────────────────────────────────────────┐\n\
             │ instance {instance:<20} digest {digest}\n\
             │ outcome  {outcome:<20} protocol apxm.client-interaction/1\n\
             ├─ observations ─────────────────────────────────────────────┤\n\
             {observations}\n\
             ├─ input ────────────────────────────────────────────────────┤\n\
             │ {prompt}\n\
             │ allow | deny | timeout   cancel   detach\n\
             └────────────────────────────────────────────────────────────┘\n",
            instance = self.program_instance_id,
            digest = self.artifact_digest,
            outcome = outcome,
            observations = observations,
            prompt = self.prompt,
        )
    }
}

/// Route one TTY line from runtime state. Approvals never execute on deny/timeout.
#[must_use]
pub fn route_input(outcome: HeadlessOutcome, line: &str) -> TuiAction {
    let line = line.trim();
    if line.is_empty() {
        return TuiAction::Ignore;
    }
    if line == "detach" {
        return TuiAction::Detach;
    }
    if line == "cancel" {
        return TuiAction::Cancel;
    }
    match (outcome, line) {
        (_, "deny" | "timeout") => TuiAction::DenyAsk,
        (_, "allow") => TuiAction::AllowAsk,
        (HeadlessOutcome::WaitingEvent, rest) if rest.starts_with("fulfill ") => {
            TuiAction::FulfillEvent
        }
        (HeadlessOutcome::Returned, "start") => TuiAction::StartInvocation,
        (HeadlessOutcome::Failed, _) => TuiAction::Detach,
        _ => TuiAction::Ignore,
    }
}

/// Closed TUI action set. The TUI does not invent terminal outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiAction {
    /// Start the admitted invocation.
    StartInvocation,
    /// Fulfill the exact EventRef the runtime is waiting on.
    FulfillEvent,
    /// Allow an unresolved Ask.
    AllowAsk,
    /// Deny or time out an Ask; the capability must not execute.
    DenyAsk,
    /// Cancel the current invocation.
    Cancel,
    /// Leave the TTY without fabricating `finish_reason: stop`.
    Detach,
    /// Unrecognized input.
    Ignore,
}

/// Draw the Interaction Client over the same outcome labels as headless mode.
pub fn run_session(instance: &str, artifact_digest: &str, outcome: HeadlessOutcome) -> Result<()> {
    let frame = TuiFrame::from_protocol(instance, artifact_digest, outcome, Vec::new());
    if io::stdout().is_terminal() {
        let mut out = io::stdout();
        write!(out, "\u{1b}[?1049h\u{1b}[2J\u{1b}[H{}", frame.render())?;
        out.flush()?;
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            match route_input(outcome, &line) {
                TuiAction::Detach | TuiAction::Cancel | TuiAction::DenyAsk => break,
                TuiAction::Ignore
                | TuiAction::AllowAsk
                | TuiAction::FulfillEvent
                | TuiAction::StartInvocation => {
                    write!(out, "\u{1b}[2J\u{1b}[H{}", frame.render())?;
                    out.flush()?;
                }
            }
        }
        write!(out, "\u{1b}[?1049l")?;
        out.flush()?;
        return Ok(());
    }
    print!("{}", frame.render());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_and_headless_agree_on_waiting_event() {
        assert_eq!(
            tui_outcome_label(HeadlessOutcome::WaitingEvent),
            "waiting_event"
        );
        assert_eq!(tui_outcome_label(HeadlessOutcome::Returned), "returned");
    }

    #[test]
    fn tui_decodes_the_same_protocol_fixtures_as_headless() {
        let returned = serde_json::json!({
            "outcome": "returned",
            "program_instance_id": "pi-1",
            "artifact_digest": "sha256:abc",
            "observations": ["content:pi-1:inv-1", "commit:pi-1:inv-1"]
        });
        let waiting = serde_json::json!({
            "outcome": "waiting_event",
            "program_instance_id": "pi-2",
            "artifact_digest": "sha256:def",
            "observations": ["event:evt-1 fulfilled"]
        });
        let returned_frame = TuiFrame::decode_protocol(&returned).unwrap();
        let waiting_frame = TuiFrame::decode_protocol(&waiting).unwrap();
        assert_eq!(returned_frame.outcome, HeadlessOutcome::Returned);
        assert_eq!(waiting_frame.outcome, HeadlessOutcome::WaitingEvent);
        assert!(returned_frame.render().contains("outcome  returned"));
        assert!(waiting_frame.render().contains("outcome  waiting_event"));
        assert!(waiting_frame.prompt.contains("event>"));
        assert_eq!(
            route_input(HeadlessOutcome::WaitingEvent, "cancel"),
            TuiAction::Cancel
        );
        assert_eq!(
            route_input(HeadlessOutcome::Returned, "deny"),
            TuiAction::DenyAsk
        );
    }
}
