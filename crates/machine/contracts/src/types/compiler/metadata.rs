//! Compiler pass metadata contract.
//!
//! `apxm-ais` remains the pass authoring/codegen source. `apxm-core` exposes
//! the generated pass catalog and stable tool-facing metadata projection.

use std::sync::OnceLock;

use super::PassCategory as RuntimePassCategory;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/apxm_pass_catalog.rs"));
}

pub use generated::{
    AIS_PASSES, ALL_PASSES, ASSIGN_PRIORITY, BUILD_PROMPT, CANONICALIZER, CONDENSE_OPS, CSE,
    DEAD_CONTEXT_ELIMINATION, DSPY_OPTIMIZE, FUSE_ASK_OPS, NORMALIZE, PROMPT_CANONICALIZATION,
    PassCategory, PassOption, PassSpec, SCHEDULING, SCHEMA_NARROWING, SHARED_PREFIX_ANALYSIS,
    SYMBOL_DCE, TEMPLATE_SPECIALIZATION, UNCONSUMED_VALUE_WARNING, find_pass_by_name,
    get_ais_passes, get_all_passes,
};

/// Stable read-only compiler pass metadata for downstream tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassMetadata {
    pub name: &'static str,
    pub summary: &'static str,
    pub description: &'static str,
    pub category: RuntimePassCategory,
}

static PASS_METADATA: OnceLock<Vec<PassMetadata>> = OnceLock::new();

fn map_category(category: PassCategory) -> RuntimePassCategory {
    match category {
        PassCategory::Analysis => RuntimePassCategory::Analysis,
        PassCategory::Transform => RuntimePassCategory::Transform,
        PassCategory::Optimization => RuntimePassCategory::Optimization,
        PassCategory::Lowering => RuntimePassCategory::Lowering,
    }
}

fn init_pass_metadata() -> Vec<PassMetadata> {
    get_all_passes()
        .map(|pass| PassMetadata {
            name: pass.name,
            summary: pass.summary,
            description: pass.description,
            category: map_category(pass.category),
        })
        .collect()
}

/// List all compiler passes visible to downstream tools.
pub fn list_pass_metadata() -> &'static [PassMetadata] {
    PASS_METADATA.get_or_init(init_pass_metadata).as_slice()
}

/// Find a pass by its CLI-visible name.
pub fn find_pass_metadata(name: &str) -> Option<&'static PassMetadata> {
    list_pass_metadata().iter().find(|pass| pass.name == name)
}
