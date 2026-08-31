//! Terminal Interaction Client. Shares protocol fixtures with headless mode.

use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Result;
use apxm_interaction_client::ProgramInvocationStatus;

/// One decoded Interaction Client fixture shared with headless JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiFrame {
    /// Program Instance id.
    pub program_instance_id: String,
    /// Admitted artifact digest.
    pub artifact_digest: String,
    /// Runtime-owned invocation status.
    pub status: ProgramInvocationStatus,
    /// Observation lines projected by content-ref / lifecycle, never payloads.
    pub observations: Vec<String>,
    /// Prompt routed from runtime state.
    pub prompt: String,
}

impl TuiFrame {
    /// Build a frame from the runtime-owned invocation status.
    #[must_use]
    pub fn from_protocol(
        program_instance_id: impl Into<String>,
        artifact_digest: impl Into<String>,
        status: ProgramInvocationStatus,
        observations: Vec<String>,
    ) -> Self {
        let prompt = match status {
            ProgramInvocationStatus::WaitingEvent => {
                "event> fulfill <event_id> <generation> | cancel | detach".to_owned()
            }
            ProgramInvocationStatus::CommittedReturn | ProgramInvocationStatus::CommittedYield => {
                "invoke> start | cancel | detach".to_owned()
            }
            _ => "runtime> inspect | cancel | detach".to_owned(),
        };
        Self {
            program_instance_id: program_instance_id.into(),
            artifact_digest: artifact_digest.into(),
            status,
            observations,
            prompt,
        }
    }

    /// Decode a headless protocol JSON fixture.
    #[cfg(test)]
    pub fn decode_protocol(value: &serde_json::Value) -> Result<Self, String> {
        let status = match value.get("status").and_then(|item| item.as_str()) {
            Some(status) => serde_json::from_value(serde_json::Value::String(status.to_owned()))
                .map_err(|_| "unknown invocation status".to_owned())?,
            None => return Err("missing invocation status".to_owned()),
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
            status,
            observations,
        ))
    }

    /// Render the full TUI, not a three-line stub.
    #[must_use]
    pub fn render(&self) -> String {
        let status =
            serde_json::to_string(&self.status).unwrap_or_else(|_| "\"unknown\"".to_owned());
        let observations = self.observations.join("\n");
        let observation_section = if observations.is_empty() {
            String::new()
        } else {
            format!(
                "             ├─ observations ─────────────────────────────────────────────┤\n             {observations}\n"
            )
        };
        format!(
            "┌─ apxm interact ─────────────────────────────────────────────┐\n\
             │ instance {instance:<20} digest {digest}\n\
             │ status   {status:<20} protocol apxm.client-interaction/1\n\
             {observation_section}\
             ├─ input ────────────────────────────────────────────────────┤\n\
             │ {prompt}\n\
             │ allow | deny | timeout   cancel   detach\n\
             └────────────────────────────────────────────────────────────┘\n",
            instance = self.program_instance_id,
            digest = self.artifact_digest,
            status = status.trim_matches('"'),
            observation_section = observation_section,
            prompt = self.prompt,
        )
    }
}

/// Route one TTY line from runtime state. Approvals never execute on deny/timeout.
#[must_use]
pub fn route_input(status: ProgramInvocationStatus, line: &str) -> TuiAction {
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
    match (status, line) {
        (_, "deny" | "timeout") => TuiAction::DenyAsk,
        (_, "allow") => TuiAction::AllowAsk,
        (ProgramInvocationStatus::WaitingEvent, rest) if rest.starts_with("fulfill ") => {
            TuiAction::FulfillEvent
        }
        (
            ProgramInvocationStatus::CommittedReturn | ProgramInvocationStatus::CommittedYield,
            "start",
        ) => TuiAction::StartInvocation,
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

/// Draw the Interaction Client over canonical runtime state.
pub fn run_session(frame: TuiFrame) -> Result<()> {
    if io::stdout().is_terminal() {
        let mut out = io::stdout();
        write!(out, "\u{1b}[?1049h\u{1b}[2J\u{1b}[H{}", frame.render())?;
        out.flush()?;
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            match route_input(frame.status, &line) {
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
    fn tui_decodes_the_same_protocol_fixtures_as_headless() {
        let returned = serde_json::json!({
            "status": "committed_return",
            "program_instance_id": "pi-1",
            "artifact_digest": "sha256:abc",
            "observations": ["content:pi-1:inv-1", "commit:pi-1:inv-1"]
        });
        let waiting = serde_json::json!({
            "status": "waiting_event",
            "program_instance_id": "pi-2",
            "artifact_digest": "sha256:def",
            "observations": ["event:evt-1 fulfilled"]
        });
        let returned_frame = TuiFrame::decode_protocol(&returned).unwrap();
        let waiting_frame = TuiFrame::decode_protocol(&waiting).unwrap();
        assert_eq!(
            returned_frame.status,
            ProgramInvocationStatus::CommittedReturn
        );
        assert_eq!(waiting_frame.status, ProgramInvocationStatus::WaitingEvent);
        assert!(
            returned_frame
                .render()
                .contains("status   committed_return")
        );
        assert!(waiting_frame.render().contains("status   waiting_event"));
        assert!(waiting_frame.prompt.contains("event>"));
        assert_eq!(
            route_input(ProgramInvocationStatus::WaitingEvent, "cancel"),
            TuiAction::Cancel
        );
        assert_eq!(
            route_input(ProgramInvocationStatus::CommittedReturn, "deny"),
            TuiAction::DenyAsk
        );
    }
}
