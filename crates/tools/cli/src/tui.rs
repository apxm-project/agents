//! Terminal Interaction Client. Shares protocol fixtures with headless mode.

use apxm_interaction_client::{HeadlessOutcome, render_outcome};

/// Render the TUI agreement with headless outcome labels.
#[must_use]
pub fn tui_outcome_label(outcome: HeadlessOutcome) -> &'static str {
    render_outcome(outcome)
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
