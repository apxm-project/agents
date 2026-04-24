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
    PassCategory, PassOption, PassSpec, SCHEDULING, SCHEMA_NARROWING, SYMBOL_DCE,
    TEMPLATE_SPECIALIZATION, UNCONSUMED_VALUE_WARNING, find_pass_by_name, get_ais_passes,
    get_all_passes,
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

#[cfg(test)]
mod tests {
    use super::{NORMALIZE, find_pass_metadata, get_all_passes, list_pass_metadata};

    #[test]
    fn pass_metadata_catalog_is_populated() {
        let passes = list_pass_metadata();
        assert!(!passes.is_empty());
        assert!(passes.iter().any(|pass| pass.name == "normalize"));
    }

    #[test]
    fn find_pass_metadata_matches_by_name() {
        let pass =
            find_pass_metadata("prompt-canonicalization").expect("known pass must be exposed");
        assert_eq!(
            pass.summary,
            "Reorder prompts to maximize shared-prefix KV-cache reuse"
        );
    }

    #[test]
    fn canonical_pass_catalog_is_exposed() {
        assert!(get_all_passes().any(|pass| pass.name == NORMALIZE.name));
    }
}
