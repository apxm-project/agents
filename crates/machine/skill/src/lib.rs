//! Shared APXM skill manifest, hash, validation, and provenance types.
//!
//! `skill.toml` manifests may use either top-level fields or a nested `[skill]`
//! table; parsing accepts both layouts so producers can converge without
//! duplicating server-side compatibility code.

use std::fs;
use std::path::{Path, PathBuf};

use apxm_artifact::{Artifact, section_kinds};
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

/// Classification of what side effects a skill is permitted to perform.
///
/// Static skills may be read-only or sandboxed. Direct write authority is no
/// longer expressed in `side_effect_policy`; runtime writes require
/// CapabilityGrant ids supplied at execution time via `capability_grant_ids`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityPolicy {
    ReadOnly,
    Sandboxed,
}

impl CapabilityPolicy {
    /// Parse a manifest's `side_effect_policy` string into a structured policy.
    /// Accepts `read_only` and `sandboxed`. Any other value yields `None`; the
    /// caller decides how to report it.
    pub fn from_manifest_value(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some(POLICY_NAME_READ_ONLY) => Some(Self::ReadOnly),
            Some(POLICY_NAME_SANDBOXED) => Some(Self::Sandboxed),
            Some(_) => None,
        }
    }

    /// Wire name for this policy (matches what producers write in `skill.toml`).
    pub fn name(&self) -> String {
        match self {
            Self::ReadOnly => POLICY_NAME_READ_ONLY.to_string(),
            Self::Sandboxed => POLICY_NAME_SANDBOXED.to_string(),
        }
    }

    /// Returns `true` if `other` is a subset of this policy for nested admission.
    ///
    /// A child execution must never widen beyond what its parent declared.
    pub fn admits(&self, other: &Self) -> bool {
        match (self, other) {
            (_, Self::ReadOnly) | (Self::Sandboxed, Self::Sandboxed) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

/// Error message returned when a skill-package artifact path resolves to a
/// symlink. Producers of the message must not editorialize on it further —
/// callers match/surface this exact string as the negative-path evidence.
pub const SYMLINKED_ARTIFACT_MESSAGE: &str = "skill artifact must not be a symlink";

/// `true` if `path` is itself a symlink (does not follow it).
///
/// Shared by the package-discovery walk ([`find_manifest_dirs`]) and the
/// artifact-load path: a symlinked package directory or a symlinked
/// `.apxmobj` artifact file could otherwise be used to point a trusted skill
/// root at content outside it without changing the on-disk path a reviewer
/// audited.
pub fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Recursively collect skill-package directories (those directly containing
/// `skill.toml`) under `root`.
///
/// Symlinked subdirectories are skipped rather than followed: a symlink
/// planted under a trusted skill root could otherwise smuggle a package in
/// from outside the audited tree while the directory listing still shows
/// only paths under `root`.
pub fn find_manifest_dirs(root: &Path, packages: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    if root.join(MANIFEST_FILE).is_file() {
        packages.push(root.to_path_buf());
        return;
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "failed to read skill root {}: {error}",
                root.display()
            ));
            return;
        }
    };

    let mut dirs = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.is_dir() && !is_symlink(&path) {
                    dirs.push(path);
                }
            }
            Err(error) => warnings.push(format!("failed to read skill entry: {error}")),
        }
    }
    dirs.sort();

    for dir in dirs {
        find_manifest_dirs(&dir, packages, warnings);
    }
}

/// Cross-check a compiled artifact's embedded `apxm.skill_manifest.v1`
/// section against the on-disk `skill.toml` (`manifest`) it claims to have
/// been compiled from.
///
/// Returns `Ok(())` when the artifact carries no embedded manifest section
/// (nothing to cross-check — older/bare artifacts) or when every compared
/// field matches. Returns `Err` naming the first mismatched field otherwise.
///
/// `artifact_hash` is handled as a special case: it is the BLAKE3 hash of
/// the artifact's own bytes, so it cannot legally be embedded *inside* the
/// artifact it hashes — the compile step strips it from the embedded copy by
/// design. The only legal pair is `embedded=None, manifest=Some(_)`;
/// anything else (present, whether matching or divergent) means a tampered
/// or stale embed and is rejected. Loosening this rule would let a
/// swapped-package attack succeed that is rejected today — do not relax it
/// without an explicit, reviewed threat-model update.
pub fn validate_embedded_skill_manifest(
    artifact: &Artifact,
    manifest: &SkillManifest,
) -> Result<(), String> {
    let Some(data) = artifact.section_data(section_kinds::SKILL_MANIFEST_V1) else {
        return Ok(());
    };
    let text = std::str::from_utf8(data)
        .map_err(|error| format!("embedded skill manifest is not UTF-8: {error}"))?;
    let embedded = parse_manifest(text)
        .map_err(|error| format!("failed to parse embedded skill manifest: {error}"))?;

    validate_embedded_manifest_field("skill_id", &embedded.skill_id, &manifest.skill_id)?;
    validate_embedded_manifest_field("version", &embedded.version, &manifest.version)?;
    validate_embedded_manifest_field("entry_flow", &embedded.entry_flow, &manifest.entry_flow)?;
    validate_embedded_manifest_option(
        "source_hash",
        embedded.source_hash.as_deref(),
        manifest.source_hash.as_deref(),
    )?;
    validate_embedded_manifest_option(
        "air_hash",
        embedded.air_hash.as_deref(),
        manifest.air_hash.as_deref(),
    )?;
    match (
        embedded.artifact_hash.as_deref(),
        manifest.artifact_hash.as_deref(),
    ) {
        (None, _) => {}
        (Some(e), Some(m)) if e == m => {}
        _ => {
            return Err(
                "embedded skill manifest artifact_hash does not match skill.toml".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_embedded_manifest_field(
    field: &str,
    embedded: &str,
    manifest: &str,
) -> Result<(), String> {
    if embedded == manifest {
        Ok(())
    } else {
        Err(format!(
            "embedded skill manifest {field} '{embedded}' does not match skill.toml '{manifest}'"
        ))
    }
}

fn validate_embedded_manifest_option(
    field: &str,
    embedded: Option<&str>,
    manifest: Option<&str>,
) -> Result<(), String> {
    if embedded == manifest {
        Ok(())
    } else {
        Err(format!(
            "embedded skill manifest {field} does not match skill.toml"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_artifact::{ArtifactMetadata, ArtifactSection};

    fn manifest(
        skill_id: &str,
        version: &str,
        entry_flow: &str,
        source_hash: Option<&str>,
        air_hash: Option<&str>,
        artifact_hash: Option<&str>,
    ) -> SkillManifest {
        SkillManifest {
            skill_id: skill_id.to_string(),
            version: version.to_string(),
            display_name: None,
            description: None,
            when_to_use: None,
            tags: Vec::new(),
            imports: Vec::new(),
            shared: false,
            entry_flow: entry_flow.to_string(),
            source_hash: source_hash.map(str::to_string),
            air_hash: air_hash.map(str::to_string),
            artifact_hash: artifact_hash.map(str::to_string),
            compiler_version: None,
            runtime_version: None,
            required_capabilities: Vec::new(),
            timeout_ms: None,
            token_limit: None,
            isolation_policy: None,
            side_effect_policy: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    fn artifact_with_embedded_manifest(embedded_toml: &str) -> Artifact {
        let mut artifact = Artifact::new(ArtifactMetadata::new(Some("test".to_string()), "test"), Vec::new());
        artifact.add_section(ArtifactSection {
            kind: section_kinds::SKILL_MANIFEST_V1.to_string(),
            data: embedded_toml.as_bytes().to_vec(),
        });
        artifact
    }

    fn matching_embedded_toml() -> String {
        r#"
            skill_id = "demo"
            version = "1.0.0"
            entry_flow = "main"
            source_hash = "blake3:aaaa"
            air_hash = "blake3:bbbb"
        "#
        .to_string()
    }

    fn matching_manifest() -> SkillManifest {
        manifest(
            "demo",
            "1.0.0",
            "main",
            Some("blake3:aaaa"),
            Some("blake3:bbbb"),
            Some("blake3:cccc"),
        )
    }

    #[test]
    fn validate_embedded_skill_manifest_accepts_matching_identity() {
        let artifact = artifact_with_embedded_manifest(&matching_embedded_toml());
        let manifest = matching_manifest();
        assert!(validate_embedded_skill_manifest(&artifact, &manifest).is_ok());
    }

    #[test]
    fn validate_embedded_skill_manifest_accepts_artifact_with_no_embedded_section() {
        let artifact = Artifact::new(
            ArtifactMetadata::new(Some("test".to_string()), "test"),
            Vec::new(),
        );
        let manifest = matching_manifest();
        assert!(validate_embedded_skill_manifest(&artifact, &manifest).is_ok());
    }

    #[test]
    fn validate_embedded_skill_manifest_rejects_skill_id_mismatch() {
        let artifact = artifact_with_embedded_manifest(&matching_embedded_toml());
        let mut manifest = matching_manifest();
        manifest.skill_id = "other".to_string();
        let error = validate_embedded_skill_manifest(&artifact, &manifest).unwrap_err();
        assert!(error.contains("skill_id"), "unexpected error: {error}");
    }

    #[test]
    fn validate_embedded_skill_manifest_rejects_entry_flow_mismatch() {
        let artifact = artifact_with_embedded_manifest(&matching_embedded_toml());
        let mut manifest = matching_manifest();
        manifest.entry_flow = "other_flow".to_string();
        let error = validate_embedded_skill_manifest(&artifact, &manifest).unwrap_err();
        assert!(error.contains("entry_flow"), "unexpected error: {error}");
    }

    #[test]
    fn validate_embedded_skill_manifest_rejects_embedded_artifact_hash_present_and_divergent() {
        // The compile step must strip `artifact_hash` from the embedded
        // copy (it hashes the artifact's own bytes, so it cannot live
        // inside them). An embedded artifact that carries a divergent
        // `artifact_hash` anyway is a tampered/stale embed.
        let embedded_toml = format!(
            "{}\nartifact_hash = \"blake3:dddd\"\n",
            matching_embedded_toml()
        );
        let artifact = artifact_with_embedded_manifest(&embedded_toml);
        let manifest = matching_manifest();
        let error = validate_embedded_skill_manifest(&artifact, &manifest).unwrap_err();
        assert!(error.contains("artifact_hash"), "unexpected error: {error}");
    }

    #[test]
    fn validate_embedded_skill_manifest_accepts_embedded_artifact_hash_present_and_matching() {
        // `(Some(e), Some(m)) if e == m` is accepted by design (pre-existing
        // behavior, preserved verbatim by this extraction): only the
        // present-and-divergent shape is rejected below.
        let embedded_toml = format!(
            "{}\nartifact_hash = \"blake3:cccc\"\n",
            matching_embedded_toml()
        );
        let artifact = artifact_with_embedded_manifest(&embedded_toml);
        let manifest = matching_manifest();
        assert!(validate_embedded_skill_manifest(&artifact, &manifest).is_ok());
    }

    #[test]
    fn is_symlink_true_for_symlinked_path_false_for_real_path() {
        let dir = std::env::temp_dir().join(format!("apxm-skill-test-{}", uuid_like()));
        fs::create_dir_all(&dir).unwrap();
        let real_file = dir.join("real.txt");
        fs::write(&real_file, b"data").unwrap();
        let link = dir.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link).unwrap();

        assert!(!is_symlink(&real_file));
        #[cfg(unix)]
        assert!(is_symlink(&link));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_manifest_dirs_skips_symlinked_subdirectories() {
        let root = std::env::temp_dir().join(format!("apxm-skill-scan-{}", uuid_like()));
        let real_pkg = root.join("real-skill");
        fs::create_dir_all(&real_pkg).unwrap();
        fs::write(real_pkg.join(MANIFEST_FILE), "skill_id=\"a\"\n").unwrap();

        // A symlinked package dir with a *valid* skill.toml, planted as a
        // sibling under the same root, must not appear in the discovered
        // package list even though it resolves to something real.
        let outside_pkg = std::env::temp_dir().join(format!("apxm-skill-outside-{}", uuid_like()));
        fs::create_dir_all(&outside_pkg).unwrap();
        fs::write(outside_pkg.join(MANIFEST_FILE), "skill_id=\"b\"\n").unwrap();
        let symlinked_pkg = root.join("linked-skill");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_pkg, &symlinked_pkg).unwrap();

        let mut packages = Vec::new();
        let mut warnings = Vec::new();
        find_manifest_dirs(&root, &mut packages, &mut warnings);

        assert!(packages.contains(&real_pkg));
        #[cfg(unix)]
        assert!(
            !packages.contains(&symlinked_pkg),
            "symlinked package directory must be skipped: {packages:?}"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside_pkg);
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{nanos}-{:?}", std::thread::current().id())
    }
}
