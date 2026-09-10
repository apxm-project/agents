//! Compiler-owned typed Event requirements, independent of runtime reservations.

use serde::{Deserialize, Serialize};

use crate::input_schema::EntrypointInputSchema;

/// One wait callsite's exact payload contract; it contains no runtime reference or authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRequirement {
    pub node_id: String,
    pub type_id: String,
    pub payload_schema: EntrypointInputSchema,
    pub schema_digest: String,
}

impl EventRequirement {
    /// Derive the schema commitment in the canonical owner, never in a frontend.
    pub fn new(
        node_id: String,
        type_id: String,
        payload_schema: EntrypointInputSchema,
    ) -> Result<Self, &'static str> {
        let schema_digest = payload_schema.canonical_digest()?;
        Ok(Self {
            node_id,
            type_id,
            payload_schema,
            schema_digest,
        })
    }

    /// Validate identities, finite schema and exact schema digest together.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::grammar::is_identifier(&self.node_id)
            || !crate::grammar::is_identifier(&self.type_id)
        {
            return Err("Event requirement identities must be contract identifiers");
        }
        if self.schema_digest != self.payload_schema.canonical_digest()? {
            return Err("Event requirement schema digest mismatch");
        }
        Ok(())
    }
}
