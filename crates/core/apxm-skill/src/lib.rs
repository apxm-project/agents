//! Shared APXM skill manifest, hash, validation, and provenance types.
//!
//! `skill.toml` manifests may use either top-level fields or a nested `[skill]`
//! table; parsing accepts both layouts so producers can converge without
//! duplicating server-side compatibility code.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub mod discovery;

pub const MANIFEST_FILE: &str = "skill.toml";
pub const HASH_PREFIX: &str = "blake3:";

/// Canonical side-effect policy values surfaced on the wire and in manifests.
///
/// Producers (skill packs) and consumers (admission code) must reference these
/// constants instead of duplicating the string literals.
pub const POLICY_NAME_READ_ONLY: &str = "read_only";
pub const POLICY_NAME_SANDBOXED: &str = "sandboxed";
/// Prefix of the `Broader` wire form: `broader[cap.a,cap.b]`. The bracketed,
/// comma-joined set of admitted capabilities is the durable form of an operator
/// write grant (e.g. a studio workflow packaged as a skill).
pub const POLICY_PREFIX_BROADER: &str = "broader[";

/// Classification of what side effects a skill is permitted to perform.
///
/// `ReadOnly` and `Sandboxed` are the two policies that the static skill
/// executor admits today. `Broader { admits }` is a forward-compatible
/// variant the admission layer can match on once specific capability sets
/// are sanctioned (e.g. "writes to a single temp dir", "network egress to
/// a pinned host"). Until then it is admitted only when the caller passes
/// in an explicit allow-list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityPolicy {
    ReadOnly,
    Sandboxed,
    Broader { admits: BTreeSet<String> },
}

impl CapabilityPolicy {
    /// Parse a manifest's `side_effect_policy` string into a structured policy.
    /// Accepts `read_only`, `sandboxed`, and the `broader[a,b,…]` grant form
    /// (the round-trip of [`Self::name`]). Any other value yields `None`; the
    /// caller decides whether to reject it.
    pub fn from_manifest_value(value: Option<&str>) -> Option<Self> {
        match value {
            None => Some(Self::ReadOnly),
            Some(POLICY_NAME_READ_ONLY) => Some(Self::ReadOnly),
            Some(POLICY_NAME_SANDBOXED) => Some(Self::Sandboxed),
            Some(other) => {
                let inner = other
                    .strip_prefix(POLICY_PREFIX_BROADER)?
                    .strip_suffix(']')?;
                let admits: BTreeSet<String> = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(String::from)
                    .collect();
                Some(Self::Broader { admits })
            }
        }
    }

    /// Wire name for this policy (matches what producers write in
    /// `skill.toml`). `Broader` does not have a single canonical name; it
    /// surfaces as the joined set of admitted capability tokens.
    pub fn name(&self) -> String {
        match self {
            Self::ReadOnly => POLICY_NAME_READ_ONLY.to_string(),
            Self::Sandboxed => POLICY_NAME_SANDBOXED.to_string(),
            Self::Broader { admits } => {
                let joined: Vec<&str> = admits.iter().map(String::as_str).collect();
                format!("{}{}]", POLICY_PREFIX_BROADER, joined.join(","))
            }
        }
    }

    /// Returns `true` if `other` is a subset of this policy. Used by nested
    /// admission to guarantee a child execution never widens beyond what
    /// its parent declared.
    pub fn admits(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ReadOnly, Self::ReadOnly) => true,
            (Self::Sandboxed, Self::ReadOnly | Self::Sandboxed) => true,
            (Self::Broader { admits: parent }, Self::Broader { admits: child }) => {
                child.is_subset(parent)
            }
            // ReadOnly is the universal privilege floor: any grant admits a
            // read-only child (it is strictly less privileged than any policy).
            (Self::Broader { .. }, Self::ReadOnly) => true,
            (Self::Broader { admits }, Self::Sandboxed) => {
                admits.iter().any(|name| name == POLICY_NAME_SANDBOXED)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillManifest {
    pub skill_id: String,
    pub version: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// "Use when …" trigger phrase for description-based discovery.
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// Discovery tags ranked against natural-language requests.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Skill libraries / ids imported into this program's visible set (lib ids
    /// or namespaced ids like `apxm-app-github::issue_triage`).
    #[serde(default)]
    pub imports: Vec<String>,
    /// When true, belongs to the global shared tier (visible without an import).
    #[serde(default)]
    pub shared: bool,
    pub entry_flow: String,
    #[serde(default)]
    pub source_hash: Option<String>,
    #[serde(default)]
    pub air_hash: Option<String>,
    #[serde(default)]
    pub artifact_hash: Option<String>,
    #[serde(default)]
    pub compiler_version: Option<String>,
    #[serde(default)]
    pub runtime_version: Option<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub token_limit: Option<u64>,
    #[serde(default)]
    pub isolation_policy: Option<String>,
    #[serde(default)]
    pub side_effect_policy: Option<String>,
    #[serde(default)]
    pub inputs: Vec<SkillParam>,
    #[serde(default)]
    pub outputs: Vec<SkillParam>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillParam {
    pub name: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillPackageHashes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub air_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillValidationReport {
    pub status: ValidationStatus,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillExecutionProvenance {
    pub skill_id: String,
    pub skill_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_skill_version: Option<String>,
    /// Runtime scope id of the calling flow_call frame, when the execution
    /// was launched as a nested child (None for top-level executions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
}

pub fn parse_manifest(contents: &str) -> Result<SkillManifest, toml::de::Error> {
    match toml::from_str::<SkillManifest>(contents) {
        Ok(manifest) => Ok(manifest),
        Err(top_level_error) => {
            let value = toml::from_str::<toml::Value>(contents)?;
            if let Some(skill_table) = value.get("skill") {
                skill_table.clone().try_into()
            } else {
                Err(top_level_error)
            }
        }
    }
}

pub fn parse_manifest_file(path: &Path) -> Result<SkillManifest, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {MANIFEST_FILE}: {error}"))?;
    parse_manifest(&contents).map_err(|error| format!("failed to parse {MANIFEST_FILE}: {error}"))
}

pub fn manifest_shape_errors(manifest: &SkillManifest) -> Vec<String> {
    let mut errors = Vec::new();
    if manifest.skill_id.trim().is_empty() {
        errors.push("skill_id is required".to_string());
    }
    if manifest.version.trim().is_empty() {
        errors.push("version is required".to_string());
    }
    if manifest.entry_flow.trim().is_empty() {
        errors.push("entry_flow is required".to_string());
    }
    if matches!(manifest.timeout_ms, Some(0)) {
        errors.push("timeout_ms must be greater than zero".to_string());
    }
    if matches!(manifest.token_limit, Some(0)) {
        errors.push("token_limit must be greater than zero".to_string());
    }
    errors
}

pub fn validate_declared_hash(
    field: &str,
    declared: Option<&str>,
    actual: Option<&str>,
    file_exists: bool,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(declared) = declared else {
        return;
    };
    if !file_exists {
        warnings.push(format!("{field} declared but matching file is missing"));
        return;
    }
    let Some(actual) = actual else {
        return;
    };
    if !hashes_match(declared, actual) {
        errors.push(format!(
            "{field} mismatch: declared {declared}, actual {actual}"
        ));
    }
}

pub fn hashes_match(declared: &str, actual: &str) -> bool {
    let declared = declared.strip_prefix(HASH_PREFIX).unwrap_or(declared);
    let actual = actual.strip_prefix(HASH_PREFIX).unwrap_or(actual);
    declared.eq_ignore_ascii_case(actual)
}

pub fn file_hash_if_present(path: &Path, errors: &mut Vec<String>) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    match fs::read(path) {
        Ok(bytes) => Some(tagged_blake3(&bytes)),
        Err(error) => {
            errors.push(format!("failed to hash {}: {error}", path.display()));
            None
        }
    }
}

pub fn tagged_blake3(bytes: &[u8]) -> String {
    format!("{HASH_PREFIX}{}", blake3::hash(bytes).to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SKILL_ID: &str = "checkout-context-triage";
    const TEST_SKILL_VERSION: &str = "0.1.0";
    const TEST_ENTRY_FLOW: &str = "main";

    #[test]
    fn parses_top_level_manifest() {
        let manifest = parse_manifest(&format!(
            r#"
skill_id = "{TEST_SKILL_ID}"
version = "{TEST_SKILL_VERSION}"
entry_flow = "{TEST_ENTRY_FLOW}"
"#
        ))
        .expect("parse top-level manifest");

        assert_eq!(manifest.skill_id, TEST_SKILL_ID);
        assert_eq!(manifest.version, TEST_SKILL_VERSION);
        assert_eq!(manifest.entry_flow, TEST_ENTRY_FLOW);
    }

    #[test]
    fn parses_nested_skill_manifest() {
        let manifest = parse_manifest(&format!(
            r#"
[skill]
skill_id = "{TEST_SKILL_ID}"
version = "{TEST_SKILL_VERSION}"
entry_flow = "{TEST_ENTRY_FLOW}"
"#
        ))
        .expect("parse nested manifest");

        assert_eq!(manifest.skill_id, TEST_SKILL_ID);
        assert_eq!(manifest.version, TEST_SKILL_VERSION);
        assert_eq!(manifest.entry_flow, TEST_ENTRY_FLOW);
    }

    #[test]
    fn validates_manifest_shape() {
        let manifest = SkillManifest {
            skill_id: String::new(),
            version: " ".to_string(),
            entry_flow: String::new(),
            display_name: None,
            description: None,
            when_to_use: None,
            tags: Vec::new(),
            imports: Vec::new(),
            shared: false,
            source_hash: None,
            air_hash: None,
            artifact_hash: None,
            compiler_version: None,
            runtime_version: None,
            required_capabilities: Vec::new(),
            allowed_tools: Vec::new(),
            timeout_ms: Some(0),
            token_limit: Some(0),
            isolation_policy: None,
            side_effect_policy: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };

        let errors = manifest_shape_errors(&manifest);

        assert!(errors.contains(&"skill_id is required".to_string()));
        assert!(errors.contains(&"version is required".to_string()));
        assert!(errors.contains(&"entry_flow is required".to_string()));
        assert!(errors.contains(&"timeout_ms must be greater than zero".to_string()));
        assert!(errors.contains(&"token_limit must be greater than zero".to_string()));
    }

    #[test]
    fn compares_tagged_and_untagged_hashes_case_insensitively() {
        assert!(hashes_match("blake3:ABCD", "abcd"));
        assert!(hashes_match("abcd", "blake3:ABCD"));
        assert!(!hashes_match("abcd", "abce"));
    }

    #[test]
    fn capability_policy_parses_known_values() {
        assert_eq!(
            CapabilityPolicy::from_manifest_value(None),
            Some(CapabilityPolicy::ReadOnly)
        );
        assert_eq!(
            CapabilityPolicy::from_manifest_value(Some(POLICY_NAME_READ_ONLY)),
            Some(CapabilityPolicy::ReadOnly)
        );
        assert_eq!(
            CapabilityPolicy::from_manifest_value(Some(POLICY_NAME_SANDBOXED)),
            Some(CapabilityPolicy::Sandboxed)
        );
        assert!(CapabilityPolicy::from_manifest_value(Some("write_files")).is_none());
    }

    #[test]
    fn capability_policy_broader_round_trips() {
        let mut admits = BTreeSet::new();
        admits.insert("provider.media_create".to_string());
        admits.insert("provider.media_publish".to_string());
        let policy = CapabilityPolicy::Broader { admits };
        // name() -> "broader[...]" must parse back to the same policy.
        let wire = policy.name();
        assert_eq!(
            wire,
            "broader[provider.media_create,provider.media_publish]"
        );
        assert_eq!(
            CapabilityPolicy::from_manifest_value(Some(&wire)),
            Some(policy)
        );
        // tolerant of surrounding whitespace in hand-written manifests.
        let spaced = CapabilityPolicy::from_manifest_value(Some("broader[ a , b ]"));
        let mut expect = BTreeSet::new();
        expect.insert("a".to_string());
        expect.insert("b".to_string());
        assert_eq!(spaced, Some(CapabilityPolicy::Broader { admits: expect }));
    }

    #[test]
    fn capability_policy_admits_subset_relationship() {
        let read_only = CapabilityPolicy::ReadOnly;
        let sandboxed = CapabilityPolicy::Sandboxed;
        assert!(read_only.admits(&CapabilityPolicy::ReadOnly));
        assert!(sandboxed.admits(&CapabilityPolicy::ReadOnly));
        assert!(sandboxed.admits(&CapabilityPolicy::Sandboxed));
        assert!(!read_only.admits(&CapabilityPolicy::Sandboxed));

        let mut parent_admits = BTreeSet::new();
        parent_admits.insert("read_only".to_string());
        parent_admits.insert("net.egress.api.example.com".to_string());
        let parent = CapabilityPolicy::Broader {
            admits: parent_admits,
        };

        let mut child_admits = BTreeSet::new();
        child_admits.insert("net.egress.api.example.com".to_string());
        let narrower_child = CapabilityPolicy::Broader {
            admits: child_admits,
        };
        assert!(parent.admits(&narrower_child));

        let mut wider_admits = BTreeSet::new();
        wider_admits.insert("net.egress.api.example.com".to_string());
        wider_admits.insert("fs.write.tmp".to_string());
        let wider_child = CapabilityPolicy::Broader {
            admits: wider_admits,
        };
        assert!(!parent.admits(&wider_child));
    }

    #[test]
    fn broader_grant_admits_read_only_child_as_floor() {
        // A broader grant that does NOT list "read_only" must still admit a
        // read-only child: read-only is strictly less privileged than any
        // grant, so it is the universal floor.
        let mut admits = BTreeSet::new();
        admits.insert("net.egress.api.example.com".to_string());
        let parent = CapabilityPolicy::Broader { admits };
        assert!(parent.admits(&CapabilityPolicy::ReadOnly));
        // It still does not admit a sandboxed child unless sandboxed is listed.
        assert!(!parent.admits(&CapabilityPolicy::Sandboxed));
    }
}
