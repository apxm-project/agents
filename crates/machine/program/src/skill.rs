//! `apxm.skill-package`, `apxm.package-local-skill`, and
//! `apxm.skill-discovery-root` — closed consumer types and verification.
//!
//! An Agent Skill is trusted instruction context, never executable deployment.
//! Two contracts state that in two carriers, and this module is the reader for
//! both plus the metadata-only discovery root that publishes them:
//!
//! - [`DiscoveryRootSkill`] is standalone. It owns its identity, its version,
//!   and the complete file inventory its `package_digest` is taken over, so its
//!   `resources` must carry its own instruction entry. It has no Agent Program
//!   source bundle, so only the file-carried instruction branch is admissible.
//! - [`PackageLocalSkill`] is carried inside an executable Agent Program
//!   package. It has no identity apart from its carrier, its `resources` list
//!   only what it carries *beyond* its instruction source, and both instruction
//!   branches are admissible.
//!
//! The two instruction branches differ in how an edit reaches the artifact
//! digest, which is why they are discriminated rather than merged. An
//! [`InstructionSource::Entry`] is a package path hashed into the carrying
//! package's integrity chain. An [`InstructionSource::Inline`] is program source
//! already inside the source bundle, so it anchors to `source_bundle_digest` and
//! states no path and no per-file digest of its own. Editing either changes the
//! artifact digest; they simply arrive there by different routes.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict};
use crate::grammar::{is_digest, is_identifier};

// ---------------------------------------------------------------------------
// Contract constants, mirrored as closed types
// ---------------------------------------------------------------------------

/// A boolean the published contract pins with `"const": true`. Decoding
/// `false` fails, exactly as the schema does, so the constraint block cannot be
/// silently inverted by a producer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstTrue;

/// A boolean the published contract pins with `"const": false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstFalse;

fn expect_const_bool<'de, D: Deserializer<'de>>(
    deserializer: D,
    expected: bool,
) -> Result<(), D::Error> {
    let actual = bool::deserialize(deserializer)?;
    if actual == expected {
        Ok(())
    } else {
        Err(serde::de::Error::custom(format!(
            "expected the constant {expected}, found {actual}"
        )))
    }
}

impl<'de> Deserialize<'de> for ConstTrue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        expect_const_bool(deserializer, true).map(|()| Self)
    }
}

impl Serialize for ConstTrue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for ConstFalse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        expect_const_bool(deserializer, false).map(|()| Self)
    }
}

impl Serialize for ConstFalse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}

/// The single accepted `schema_version` for a discovery-root skill package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryRootSkillVersion {
    #[serde(rename = "apxm.skill-package")]
    V1,
}

/// The single accepted `schema_version` for a package-local skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageLocalSkillVersion {
    #[serde(rename = "apxm.package-local-skill")]
    V1,
}

/// The single accepted `schema_version` for a skill discovery root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillDiscoveryRootVersion {
    #[serde(rename = "apxm.skill-discovery-root")]
    V1,
}

/// The pinned instruction path. The contract admits exactly one file name so a
/// reader never has to guess which document carries the instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstructionPath {
    #[serde(rename = "SKILL.md")]
    SkillMd,
}

impl InstructionPath {
    /// The canonical wire string.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::SkillMd => "SKILL.md",
        }
    }
}

/// The pinned instruction media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstructionMediaType {
    #[serde(rename = "text/markdown")]
    TextMarkdown,
}

impl InstructionMediaType {
    /// The canonical wire string.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::TextMarkdown => "text/markdown",
        }
    }
}

/// The `entry` discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    #[serde(rename = "entry")]
    Entry,
}

/// The `inline` discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineKind {
    #[serde(rename = "inline")]
    Inline,
}

/// The discovery-root tier a root publishes at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RootTier {
    Global,
    Project,
    Local,
}

/// Discovery publishes cards only. Listing a skill never loads its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryMode {
    #[serde(rename = "metadata_only")]
    MetadataOnly,
}

// ---------------------------------------------------------------------------
// Shared skill shapes
// ---------------------------------------------------------------------------

/// Discovery metadata shared by both skill carriers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub tags: Vec<String>,
    pub imports: Vec<String>,
    pub shared: bool,
}

/// Instructions carried as a package file, hashed into the integrity chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionEntrySource {
    pub kind: EntryKind,
    pub path: InstructionPath,
    pub digest: String,
    pub media_type: InstructionMediaType,
    pub bytes: u64,
}

/// Instructions written inline in Agent Program source, already inside the
/// source bundle and therefore anchored to its digest rather than a file path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionInlineSource {
    pub kind: InlineKind,
    pub media_type: InstructionMediaType,
    pub bytes: u64,
    pub source_bundle_digest: String,
}

/// The discriminated instruction source. Exactly one branch decodes: each is
/// closed, so a document mixing a path with a source-bundle digest matches
/// neither and is rejected at the decode boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InstructionSource {
    Entry(InstructionEntrySource),
    Inline(InstructionInlineSource),
}

/// One ordinary, non-executable file the skill carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceEntry {
    pub path: String,
    pub digest: String,
    pub media_type: String,
    pub bytes: u64,
}

/// A discovery-root skill's constraint block: standalone and not executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRootSkillConstraints {
    pub instruction_only: ConstTrue,
    pub executable_package: ConstFalse,
    pub executable_artifacts: ConstFalse,
    pub ambient_context_mutation: ConstFalse,
}

/// A package-local skill's constraint block. It replaces
/// `executable_package: false` with the positive assertion that an executable
/// package carries it — the one field that distinguishes the two carriers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageLocalSkillConstraints {
    pub instruction_only: ConstTrue,
    pub carried_by_executable_package: ConstTrue,
    pub executable_artifacts: ConstFalse,
    pub ambient_context_mutation: ConstFalse,
}

/// A discovery root's constraint block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDiscoveryRootConstraints {
    pub body_loaded_on_call: ConstTrue,
    pub activates_on_listing: ConstFalse,
    pub mutates_authority: ConstFalse,
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

/// A standalone instruction-only skill package published in a discovery root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRootSkill {
    pub schema_version: DiscoveryRootSkillVersion,
    pub skill_id: String,
    pub library_id: String,
    pub package_version: String,
    pub metadata: SkillMetadata,
    /// Only the file-carried branch: a standalone skill has no Agent Program
    /// source bundle for inline instructions to be part of.
    pub instruction_source: InstructionEntrySource,
    pub resources: Vec<ResourceEntry>,
    pub constraints: DiscoveryRootSkillConstraints,
}

/// A skill carried inside an executable Agent Program package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageLocalSkill {
    pub schema_version: PackageLocalSkillVersion,
    pub skill_id: String,
    pub carrier_package_id: String,
    pub metadata: SkillMetadata,
    pub instruction_source: InstructionSource,
    pub resources: Vec<ResourceEntry>,
    pub constraints: PackageLocalSkillConstraints,
}

/// One published discovery card: identity, digests, and metadata, never a body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCard {
    pub skill_id: String,
    pub library_id: String,
    pub package_digest: String,
    pub instruction_digest: String,
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub tags: Vec<String>,
    pub imports: Vec<String>,
    pub shared: bool,
}

impl SkillCard {
    /// The `(library_id, skill_id)` pair that identifies this card in a root.
    #[must_use]
    pub fn identity(&self) -> (&str, &str) {
        (self.library_id.as_str(), self.skill_id.as_str())
    }
}

/// A metadata-only discovery root publishing visible skill cards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDiscoveryRoot {
    pub schema_version: SkillDiscoveryRootVersion,
    pub root_id: String,
    pub root_tier: RootTier,
    pub root_ref: String,
    pub discovery_mode: DiscoveryMode,
    pub skill_cards: Vec<SkillCard>,
    pub constraints: SkillDiscoveryRootConstraints,
}

// ---------------------------------------------------------------------------
// Shared verification helpers
// ---------------------------------------------------------------------------

/// Published `maxLength` bounds for the metadata prose fields.
const MAX_NAME: usize = 128;
const MAX_DESCRIPTION: usize = 512;
const MAX_WHEN_TO_USE: usize = 2048;
const MAX_MEDIA_TYPE: usize = 255;

/// Is `path` an executable artifact rather than instruction-only content?
///
/// A skill contributes instructions and ordinary resources. A compiled object,
/// a serialized AIR module, or a package manifest is a deployment unit, and
/// carrying one inside a skill is how "instruction-only" would quietly stop
/// being true. The check is on the file name, so nesting it under a
/// subdirectory does not evade it.
#[must_use]
pub fn is_executable_skill_resource(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name == "skill.toml" {
        return true;
    }
    matches!(
        name.rsplit_once('.').map(|(_, extension)| extension),
        Some("apxmobj" | "air")
    )
}

/// `^[0-9]+\.[0-9]+\.[0-9]+(-…)?(\+…)?$`, checked without a regex dependency.
///
/// Build metadata is stripped before the pre-release suffix, so a `-` inside
/// `+build-5` is not mistaken for the start of a pre-release.
fn is_package_version(value: &str) -> bool {
    let without_build = value.split_once('+').map_or(value, |(before, _)| before);
    let core = without_build
        .split_once('-')
        .map_or(without_build, |(before, _)| before);
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    [major, minor, patch]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn check_identifier(verdict: &mut Verdict, location: &str, value: &str, what: &str) {
    if !is_identifier(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            location.to_string(),
            format!("{what} is not a contract identifier"),
        ));
    }
}

fn check_digest(verdict: &mut Verdict, location: &str, value: &str, what: &str) {
    if !is_digest(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidDigest,
            location.to_string(),
            format!("{what} is not a lowercase sha256 digest"),
        ));
    }
}

fn check_bounded(verdict: &mut Verdict, location: &str, value: &str, what: &str, max: usize) {
    if value.is_empty() || value.chars().count() > max {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            location.to_string(),
            format!("{what} must be between 1 and {max} characters"),
        ));
    }
}

/// A list the contract requires in strict ascending order. Sortedness is what
/// makes a generated discovery document byte-reproducible, and strictness is
/// what makes a repeat a rejection rather than a silent duplicate.
fn check_strictly_ascending(verdict: &mut Verdict, location: &str, values: &[String], what: &str) {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::UnsortedIdentifierList,
            location.to_string(),
            format!("{what} must be in strict ascending order"),
        ));
    }
}

fn check_metadata(verdict: &mut Verdict, location: &str, metadata: &SkillMetadata) {
    check_bounded(verdict, location, &metadata.name, "metadata name", MAX_NAME);
    check_bounded(
        verdict,
        location,
        &metadata.description,
        "metadata description",
        MAX_DESCRIPTION,
    );
    check_bounded(
        verdict,
        location,
        &metadata.when_to_use,
        "metadata when_to_use",
        MAX_WHEN_TO_USE,
    );
    for tag in &metadata.tags {
        check_identifier(verdict, location, tag, "metadata tag");
    }
    for import in &metadata.imports {
        check_identifier(verdict, location, import, "metadata import");
    }
    check_strictly_ascending(verdict, location, &metadata.tags, "metadata tags");
    check_strictly_ascending(verdict, location, &metadata.imports, "metadata imports");
}

fn check_resources(verdict: &mut Verdict, location: &str, resources: &[ResourceEntry]) {
    for resource in resources {
        check_digest(
            verdict,
            location,
            &resource.digest,
            &format!("resource '{}' digest", resource.path),
        );
        check_bounded(
            verdict,
            location,
            &resource.media_type,
            &format!("resource '{}' media type", resource.path),
            MAX_MEDIA_TYPE,
        );
        if is_executable_skill_resource(&resource.path) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SkillExecutableResource,
                location.to_string(),
                format!(
                    "resource '{}' is an executable artifact; a skill is instruction-only",
                    resource.path
                ),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

impl DiscoveryRootSkill {
    /// Verify a decoded discovery-root skill package.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();
        let location = &self.skill_id;

        check_identifier(&mut verdict, location, &self.skill_id, "skill_id");
        check_identifier(&mut verdict, location, &self.library_id, "library_id");
        if !is_package_version(&self.package_version) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                location.clone(),
                "package_version is not a published version string",
            ));
        }
        check_metadata(&mut verdict, location, &self.metadata);
        check_digest(
            &mut verdict,
            location,
            &self.instruction_source.digest,
            "instruction digest",
        );
        check_resources(&mut verdict, location, &self.resources);

        // A standalone package's `resources` are its complete inventory, so the
        // instruction entry has to appear there and has to agree with itself.
        // Without this, `package_digest` would be taken over a set that does not
        // include the very file the skill exists to carry.
        let instruction_path = self.instruction_source.path.wire();
        match self
            .resources
            .iter()
            .find(|resource| resource.path == instruction_path)
        {
            None => verdict.push(Diagnostic::new(
                DiagnosticCode::SkillInstructionNotCarried,
                location.clone(),
                format!(
                    "resources do not carry the instruction entry '{instruction_path}'; a \
                     discovery-root skill's resources are its complete package inventory"
                ),
            )),
            Some(carried) => {
                if carried.digest != self.instruction_source.digest
                    || carried.bytes != self.instruction_source.bytes
                    || carried.media_type != self.instruction_source.media_type.wire()
                {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SkillInstructionEntryMismatch,
                        location.clone(),
                        format!(
                            "the resource entry for '{instruction_path}' disagrees with \
                             instruction_source on digest, size, or media type"
                        ),
                    ));
                }
            }
        }

        verdict.finish()
    }
}

impl PackageLocalSkill {
    /// Verify a decoded package-local skill.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();
        let location = &self.skill_id;

        check_identifier(&mut verdict, location, &self.skill_id, "skill_id");
        check_identifier(
            &mut verdict,
            location,
            &self.carrier_package_id,
            "carrier_package_id",
        );
        check_metadata(&mut verdict, location, &self.metadata);
        check_resources(&mut verdict, location, &self.resources);

        match &self.instruction_source {
            InstructionSource::Entry(entry) => {
                check_digest(&mut verdict, location, &entry.digest, "instruction digest");
                // The carried form states the instruction once. Repeating it in
                // `resources` is the discovery-root convention leaking into a
                // carrier that already hashes the path through its own chain,
                // and it would double-count the file.
                let instruction_path = entry.path.wire();
                if self
                    .resources
                    .iter()
                    .any(|resource| resource.path == instruction_path)
                {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SkillInstructionNotCarried,
                        location.clone(),
                        format!(
                            "resources repeat the instruction entry '{instruction_path}'; a \
                             package-local skill lists only what it carries beyond it"
                        ),
                    ));
                }
            }
            InstructionSource::Inline(inline) => {
                check_digest(
                    &mut verdict,
                    location,
                    &inline.source_bundle_digest,
                    "instruction source_bundle_digest",
                );
            }
        }

        verdict.finish()
    }
}

impl SkillDiscoveryRoot {
    /// Verify a decoded discovery root.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();
        let location = &self.root_id;

        check_identifier(&mut verdict, location, &self.root_id, "root_id");
        if self.root_ref.is_empty() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                location.clone(),
                "root_ref must be a non-empty reference",
            ));
        }
        if self.skill_cards.is_empty() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                location.clone(),
                "a discovery root publishes at least one skill card",
            ));
        }

        let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
        for card in &self.skill_cards {
            let card_location = format!("{}/{}", card.library_id, card.skill_id);
            check_identifier(&mut verdict, &card_location, &card.skill_id, "skill_id");
            check_identifier(&mut verdict, &card_location, &card.library_id, "library_id");
            check_digest(
                &mut verdict,
                &card_location,
                &card.package_digest,
                "package_digest",
            );
            check_digest(
                &mut verdict,
                &card_location,
                &card.instruction_digest,
                "instruction_digest",
            );
            check_bounded(
                &mut verdict,
                &card_location,
                &card.name,
                "card name",
                MAX_NAME,
            );
            check_bounded(
                &mut verdict,
                &card_location,
                &card.description,
                "card description",
                MAX_DESCRIPTION,
            );
            check_bounded(
                &mut verdict,
                &card_location,
                &card.when_to_use,
                "card when_to_use",
                MAX_WHEN_TO_USE,
            );
            for tag in &card.tags {
                check_identifier(&mut verdict, &card_location, tag, "card tag");
            }
            for import in &card.imports {
                check_identifier(&mut verdict, &card_location, import, "card import");
            }
            check_strictly_ascending(&mut verdict, &card_location, &card.tags, "card tags");
            check_strictly_ascending(&mut verdict, &card_location, &card.imports, "card imports");

            if !seen.insert(card.identity()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateSkillCard,
                    card_location.clone(),
                    "two cards name one (library_id, skill_id) identity",
                ));
            }

            // A global root is the shared tier. Publishing a non-shared skill
            // there would advertise it to every consumer of the root while its
            // own metadata says it is not shareable.
            if self.root_tier == RootTier::Global && !card.shared {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SkillCardNotShareable,
                    card_location,
                    "a global-tier root publishes only shared skills",
                ));
            }
        }

        // Card order is part of the document, not an accident of directory
        // traversal: a generated root has to be byte-reproducible across hosts.
        let ordering: Vec<String> = self
            .skill_cards
            .iter()
            .map(|card| format!("{}\u{0}{}", card.library_id, card.skill_id))
            .collect();
        check_strictly_ascending(&mut verdict, location, &ordering, "skill_cards");

        verdict.finish()
    }
}

/// Verify a discovery-root skill package presented as JSON, failing closed on
/// decode errors.
#[must_use]
pub fn verify_skill_package_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<DiscoveryRootSkill>(value.clone()) {
        Ok(skill) => skill.verify(),
        Err(error) => crate::diagnostic::schema_violation("skill_package", &error),
    }
}

/// Verify a package-local skill presented as JSON, failing closed on decode
/// errors.
#[must_use]
pub fn verify_package_local_skill_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<PackageLocalSkill>(value.clone()) {
        Ok(skill) => skill.verify(),
        Err(error) => crate::diagnostic::schema_violation("package_local_skill", &error),
    }
}

/// Verify a skill discovery root presented as JSON, failing closed on decode
/// errors.
#[must_use]
pub fn verify_skill_discovery_root_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<SkillDiscoveryRoot>(value.clone()) {
        Ok(root) => root.verify(),
        Err(error) => crate::diagnostic::schema_violation("skill_discovery_root", &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_version_grammar() {
        assert!(is_package_version("1.2.3"));
        assert!(is_package_version("0.0.0"));
        assert!(is_package_version("1.2.3-rc.1"));
        assert!(is_package_version("1.2.3+build.5"));
        assert!(!is_package_version("1.2"));
        assert!(!is_package_version("1.2.3.4"));
        assert!(!is_package_version("v1.2.3"));
        assert!(!is_package_version(""));
    }

    #[test]
    fn executable_resources_are_recognized_at_any_depth() {
        assert!(is_executable_skill_resource("skill.apxmobj"));
        assert!(is_executable_skill_resource("build/nested/module.APXMOBJ"));
        assert!(is_executable_skill_resource("out/program.air"));
        assert!(is_executable_skill_resource("skill.toml"));
        assert!(!is_executable_skill_resource("SKILL.md"));
        assert!(!is_executable_skill_resource("references/airflow.md"));
    }

    /// The constraint blocks are the contract's `const` assertions, so a
    /// producer that inverts one must be refused at decode, not merely flagged.
    #[test]
    fn constraint_constants_cannot_be_inverted() {
        serde_json::from_value::<DiscoveryRootSkillConstraints>(serde_json::json!({
            "instruction_only": true,
            "executable_package": false,
            "executable_artifacts": false,
            "ambient_context_mutation": false
        }))
        .expect("the published constraint block decodes");

        serde_json::from_value::<DiscoveryRootSkillConstraints>(serde_json::json!({
            "instruction_only": true,
            "executable_package": true,
            "executable_artifacts": false,
            "ambient_context_mutation": false
        }))
        .expect_err("a discovery-root skill may not claim to be an executable package");

        serde_json::from_value::<PackageLocalSkillConstraints>(serde_json::json!({
            "instruction_only": true,
            "carried_by_executable_package": false,
            "executable_artifacts": false,
            "ambient_context_mutation": false
        }))
        .expect_err("a package-local skill is carried by an executable package by definition");
    }

    /// The two instruction branches are mutually exclusive on the wire: each is
    /// closed, so a document cannot state a package path and a source-bundle
    /// digest at once and have a reader pick a winner.
    #[test]
    fn instruction_source_branches_are_mutually_exclusive() {
        let entry = serde_json::json!({
            "kind": "entry",
            "path": "SKILL.md",
            "digest": format!("sha256:{}", "1".repeat(64)),
            "media_type": "text/markdown",
            "bytes": 512
        });
        let inline = serde_json::json!({
            "kind": "inline",
            "media_type": "text/markdown",
            "bytes": 512,
            "source_bundle_digest": format!("sha256:{}", "4".repeat(64))
        });
        assert!(matches!(
            serde_json::from_value::<InstructionSource>(entry.clone()).expect("entry decodes"),
            InstructionSource::Entry(_)
        ));
        assert!(matches!(
            serde_json::from_value::<InstructionSource>(inline.clone()).expect("inline decodes"),
            InstructionSource::Inline(_)
        ));

        let mut mixed = entry.clone();
        mixed["source_bundle_digest"] = inline["source_bundle_digest"].clone();
        serde_json::from_value::<InstructionSource>(mixed)
            .expect_err("a document may state exactly one instruction branch");

        // The standalone carrier admits only the file-carried branch, so an
        // inline source there is a decode failure rather than a soft warning.
        serde_json::from_value::<InstructionEntrySource>(inline)
            .expect_err("a discovery-root skill has no source bundle to inline into");
    }
}
