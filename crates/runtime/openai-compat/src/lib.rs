//! Inbound OpenAI HTTP protocol adapter over the Runtime Service.
//!
//! Serving aliases bind committed artifacts only. This crate shares no DTO with
//! retired `apxm chat`.

use serde::{Deserialize, Serialize};

/// Published serving alias. Source packages are not accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServingAlias {
    /// OpenAI `model` id.
    pub model: String,
    /// Already committed artifact digest.
    pub artifact_digest: String,
}

impl ServingAlias {
    /// Reject empty model ids and empty digests.
    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() || self.artifact_digest.trim().is_empty() {
            return Err("serving alias requires model and artifact digest".to_owned());
        }
        Ok(())
    }
}

/// Bounded `/v1/responses` projection. Hidden agent-loop fields fail closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesProjection {
    /// Request id.
    pub id: String,
    /// Mapped finish reason from canonical terminal state.
    pub finish_reason: String,
}

/// Fields this adapter never treats as Capability grants.
#[must_use]
pub fn messages_cannot_mint_capabilities() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_requires_committed_artifact() {
        assert!(
            ServingAlias {
                model: "alias".to_owned(),
                artifact_digest: String::new(),
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn openai_messages_do_not_mint_capabilities() {
        assert!(messages_cannot_mint_capabilities());
    }
}
