//! Terminal Interaction Client. Shares protocol fixtures with headless mode.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use apxm_interaction_client::{HeadlessOutcome, render_outcome};

/// Render the TUI agreement with headless outcome labels.
#[must_use]
pub fn tui_outcome_label(outcome: HeadlessOutcome) -> &'static str {
    render_outcome(outcome)
}

/// Draw the Interaction Client over the same outcome labels as headless mode.
pub fn run_session(instance: &str, artifact_digest: &str, outcome: HeadlessOutcome) -> Result<()> {
    let body = format!(
        "apxm interact\nprogram_instance_id={instance}\nartifact_digest={artifact_digest}\noutcome={}\n",
        tui_outcome_label(outcome)
    );
    if io::stdout().is_terminal() {
        let mut out = io::stdout();
        write!(out, "\u{1b}[2J\u{1b}[H{body}")?;
        out.flush()?;
        return Ok(());
    }
    print!("{body}");
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
    }
}
