//! The `search_skills` discovery capability.
//!
//! Description-based, **scope-aware** skill discovery. It ranks the live skill
//! catalogue against a natural-language request, restricted to the caller's
//! *visible set* — the global shared tier (`shared = true`) plus the libraries
//! the agent imported. The agent never sees the whole catalogue.
//!
//! Bodies are never returned: each hit carries only `skill_id` + `when_to_use`
//! + `description` so the agent can decide which skill to load/call next
//! (progressive disclosure). The ranking core lives in
//! [`apxm_skill::discovery`] and is unit-tested there.

use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::Runtime;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;
use apxm_skill::discovery::{SkillCard, VisibleSet, rank};
use async_trait::async_trait;
use tracing::{info, warn};

use crate::skills::SkillLibrary;

const NAME: &str = "search_skills";
const DEFAULT_K: usize = 5;

/// Discovery capability backed by the live [`SkillLibrary`].
pub(crate) struct SearchSkillsCapability {
    metadata: CapabilityMetadata,
    library: SkillLibrary,
}

impl SearchSkillsCapability {
    pub(crate) fn new(library: SkillLibrary) -> Self {
        let metadata = CapabilityMetadata::new(
            NAME,
            "Find installed skills relevant to a request, ranked by description. \
             Restricted to your visible set: the global shared tier plus the \
             libraries you imported — never the whole catalogue. Returns skill \
             ids and when-to-use so you can decide which one to call next.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "Natural-language description of what you need to do."
                    },
                    "imports": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Libraries/skills in scope for this agent (lib, lib::skill, or skill ids). Shared-tier skills are always visible even if omitted."
                    },
                    "k": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of results (default 5)."
                    }
                },
                "required": ["request"]
            }),
        )
        .with_returns("JSON array of {skill_id, library, when_to_use, description}")
        .with_groups(vec!["skills".to_string()])
        .with_read_only();
        Self { metadata, library }
    }

    /// Build discovery cards from the live catalogue scan.
    fn cards(&self) -> Vec<SkillCard> {
        self.library
            .scan()
            .data
            .into_iter()
            .filter_map(|rec| {
                let manifest = rec.manifest;
                let skill_id = rec
                    .skill_id
                    .or_else(|| manifest.as_ref().map(|m| m.skill_id.clone()))?;
                let (description, when_to_use, tags, shared) = match &manifest {
                    Some(m) => (
                        m.description.clone().unwrap_or_default(),
                        m.when_to_use.clone().unwrap_or_default(),
                        m.tags.clone(),
                        m.shared,
                    ),
                    None => (String::new(), String::new(), Vec::new(), false),
                };
                Some(SkillCard {
                    skill_id,
                    library: rec.pack.map(|p| p.pack_id),
                    description,
                    when_to_use,
                    tags,
                    shared,
                })
            })
            .collect()
    }
}

#[async_trait]
impl CapabilityExecutor for SearchSkillsCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let request = args
            .get("request")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RuntimeError::Capability {
                capability: NAME.to_string(),
                message: "missing required 'request' argument".to_string(),
            })?;

        let imports: Vec<String> = args
            .get("imports")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let k = args
            .get("k")
            .and_then(|v| v.as_i64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(DEFAULT_K);

        let visible = VisibleSet::from_imports(imports);
        let cards = self.cards();
        let matches = rank(&request, &cards, &visible, k);

        let out: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "skill_id": m.skill_id,
                    "library": m.library,
                    "when_to_use": m.when_to_use,
                    "description": m.description,
                })
            })
            .collect();

        let json = serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string());
        Ok(Value::String(json))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

/// Register `search_skills` into the runtime's capability system, backed by the
/// server's live skill library. Best-effort: a failure only disables discovery.
pub(crate) fn register(runtime: &Runtime, library: SkillLibrary) {
    let cap: Arc<dyn CapabilityExecutor> = Arc::new(SearchSkillsCapability::new(library));
    match runtime.capability_system().register(cap) {
        Ok(()) => info!(capability = NAME, "registered skill-discovery capability"),
        Err(error) => {
            warn!(capability = NAME, %error, "failed to register search_skills capability")
        }
    }
}
