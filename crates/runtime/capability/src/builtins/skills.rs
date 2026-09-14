//! `list_skills`, `search_skills`, and `read_skill` — the skill discovery
//! capabilities, with handlers.
//!
//! These three ids were once on the compiler's builtin allowlist with nothing
//! behind them, so `agent lint` accepted a package declaring a capability that
//! could never execute. They were removed for that reason. This module is why
//! they are back: the handler exists first, and the id is re-added to
//! `apxm_ais::capabilities::BUILTINS` only because of it.
//!
//! # What discovery is, and is not
//!
//! A discovery root publishes *cards*: identity, digests, and routing metadata.
//! Listing or searching never loads an instruction body and never mints
//! authority — that is `apxm.skill-discovery-root`'s `metadata_only` mode and
//! its `activates_on_listing: false` / `mutates_authority: false` constraints.
//! Only [`ReadSkillCapability`] loads a body, and it loads exactly one.
//!
//! The index is not trusted merely because it was built here. Every root this
//! module scans is assembled into an `apxm.skill-discovery-root` document and
//! run through [`verify_skill_discovery_root_json`]; a root that does not
//! verify is refused rather than served. That verifier is the reader the
//! discovery-root contract previously lacked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use apxm_core::{constants::capabilities, error::RuntimeError, types::Value};
use apxm_program::frontend_graph::MAX_INLINE_SKILL_BYTES;
use apxm_program::skill::{RootTier, verify_skill_discovery_root_json};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::{fs_boundary::secure_read_under_root, require_string_arg, untrusted_content_value};
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};

/// The one instruction document a skill directory carries.
const INSTRUCTIONS: &str = "SKILL.md";

/// Ceiling on one instruction body. A skill is trusted context that lands in a
/// model's window, so an oversized one is a context-budget failure, not a file
/// the reader should quietly truncate.
const MAX_INSTRUCTION_BYTES: usize = 128 * 1024;

/// One configured discovery root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRootConfig {
    /// Stable identifier for the root, also used as the cards' `library_id`.
    pub root_id: String,
    /// Which tier this root publishes at.
    pub tier: RootTier,
    /// Directory holding one `<skill_id>/SKILL.md` per skill.
    pub path: PathBuf,
}

/// Configuration for the skill discovery capabilities.
///
/// `roots` is empty by default: a capability that scanned the filesystem on its
/// own initiative would be reading whatever happened to be near the process.
/// The host states its roots, exactly as it states `read`'s allowed paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub roots: Vec<SkillRootConfig>,
}

/// One inline Agent Skill body carried by an executable artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSkill {
    pub skill_id: String,
    pub text: String,
}

impl InlineSkill {
    #[must_use]
    pub fn new(skill_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            skill_id: skill_id.into(),
            text: text.into(),
        }
    }
}

const INLINE_ROOT_ID: &str = "inline";

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: Vec::new(),
        }
    }
}

/// One indexed skill: its published card plus the on-disk path of its body.
#[derive(Clone, Debug)]
struct IndexedSkill {
    root_id: String,
    root_path: PathBuf,
    skill_id: String,
    card: BTreeMap<String, serde_json::Value>,
    instructions: PathBuf,
}

/// YAML-subset frontmatter: a leading `---` block of `key: value` lines.
///
/// This is deliberately the same shape `tools/scripts/validate_agent_skill.py`
/// accepts, so a directory that passes the authoring gate is a directory this
/// index can read.
fn frontmatter(text: &str) -> Option<BTreeMap<String, String>> {
    let body = text.strip_prefix("---\n")?;
    let end = body.find("\n---\n")?;
    let mut values = BTreeMap::new();
    for line in body[..end].lines() {
        if line.is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(':')?;
        values.insert(key.trim().to_string(), value.trim().to_string());
    }
    Some(values)
}

/// A comma-separated frontmatter list, sorted and deduplicated.
///
/// Ordering is normalized on read rather than demanded of the author: the
/// discovery document has to be byte-reproducible, and that is the index's
/// obligation, not the skill author's.
fn identifier_list(raw: Option<&String>) -> Vec<String> {
    let mut items: Vec<String> = raw
        .map(|value| value.trim_matches(['[', ']']))
        .unwrap_or_default()
        .split(',')
        .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
        .filter(|item| !item.is_empty())
        .collect();
    items.sort();
    items.dedup();
    items
}

fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn capability_error(capability: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Capability {
        capability: capability.to_string(),
        message: message.into(),
    }
}

/// Read one skill directory into a card, or explain why it is not one.
fn index_one(
    root: &SkillRootConfig,
    root_canonical: &Path,
    directory: &Path,
    capability: &str,
) -> CapabilityResult<Option<IndexedSkill>> {
    let directory_metadata = std::fs::symlink_metadata(directory).map_err(|error| {
        capability_error(capability, format!("stat {}: {error}", directory.display()))
    })?;
    if directory_metadata.file_type().is_symlink() {
        return Err(capability_error(
            capability,
            format!("skill directory '{}' is a symlink", directory.display()),
        ));
    }
    if !directory_metadata.is_dir() {
        return Ok(None);
    }
    let directory_canonical = std::fs::canonicalize(directory).map_err(|error| {
        capability_error(
            capability,
            format!("canonicalize {}: {error}", directory.display()),
        )
    })?;
    if !directory_canonical.starts_with(root_canonical) {
        return Err(capability_error(
            capability,
            format!(
                "skill directory '{}' escapes its configured root",
                directory.display()
            ),
        ));
    }
    let instructions = directory.join(INSTRUCTIONS);
    if !instructions.is_file() {
        return Ok(None);
    }
    // A symlinked body would let a directory inside the root serve a file
    // outside it, which is the one way a metadata-only root could leak.
    if std::fs::symlink_metadata(&instructions)
        .map_err(|error| {
            capability_error(
                capability,
                format!("stat {}: {error}", instructions.display()),
            )
        })?
        .file_type()
        .is_symlink()
    {
        return Err(capability_error(
            capability,
            format!(
                "'{}' is a symlink; a skill body must be a regular file inside its root",
                instructions.display()
            ),
        ));
    }

    let Some(skill_id) = directory.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };

    let bytes = secure_read_under_root(&root.path, &instructions, MAX_INSTRUCTION_BYTES).map_err(
        |error| {
            capability_error(
                capability,
                format!("securely read {}: {error}", instructions.display()),
            )
        },
    )?;
    let size = bytes.len();
    if size > MAX_INSTRUCTION_BYTES {
        return Err(capability_error(
            capability,
            format!(
                "'{}' is {size} bytes, over the {MAX_INSTRUCTION_BYTES} byte instruction limit",
                instructions.display()
            ),
        ));
    }
    let text = String::from_utf8(bytes.clone()).map_err(|error| {
        capability_error(
            capability,
            format!("{} is not valid UTF-8: {error}", instructions.display()),
        )
    })?;
    let Some(front) = frontmatter(&text) else {
        return Err(capability_error(
            capability,
            format!(
                "'{}' has no readable frontmatter; a skill states at least a name and a description",
                instructions.display()
            ),
        ));
    };

    // The directory name is the discovery path, so it is the id. A frontmatter
    // name that disagrees with it would make the same skill answer to two
    // names, so it is refused rather than silently preferred either way.
    let declared = front.get("name").map_or(skill_id, String::as_str);
    if declared != skill_id {
        return Err(capability_error(
            capability,
            format!(
                "'{}' declares name '{declared}' but lives in directory '{skill_id}'",
                instructions.display()
            ),
        ));
    }
    let Some(description) = front.get("description").filter(|value| !value.is_empty()) else {
        return Err(capability_error(
            capability,
            format!("'{}' states no description", instructions.display()),
        ));
    };

    // `when_to_use` is the routing hint the contract requires on every card. A
    // skill that does not state one separately is routed on its description,
    // which is what a one-line skill description already is.
    let when_to_use = front
        .get("when_to_use")
        .filter(|value| !value.is_empty())
        .unwrap_or(description);

    let mut card = BTreeMap::new();
    card.insert("skill_id".into(), serde_json::json!(skill_id));
    card.insert("library_id".into(), serde_json::json!(root.root_id));
    // A standalone skill directory's package inventory is its instruction file,
    // so both digests are taken over the same bytes today. They stay separate
    // fields because a skill that carries resources has a package digest over
    // more than its body.
    card.insert(
        "package_digest".into(),
        serde_json::json!(digest_of(&bytes)),
    );
    card.insert(
        "instruction_digest".into(),
        serde_json::json!(digest_of(&bytes)),
    );
    card.insert("name".into(), serde_json::json!(skill_id));
    card.insert("description".into(), serde_json::json!(description));
    card.insert("when_to_use".into(), serde_json::json!(when_to_use));
    card.insert(
        "tags".into(),
        serde_json::json!(identifier_list(front.get("tags"))),
    );
    card.insert(
        "imports".into(),
        serde_json::json!(identifier_list(front.get("imports"))),
    );
    card.insert(
        "shared".into(),
        serde_json::json!(
            front.get("shared").map(String::as_str) == Some("true")
                || root.tier == RootTier::Global
        ),
    );

    Ok(Some(IndexedSkill {
        root_id: root.root_id.clone(),
        root_path: root.path.clone(),
        skill_id: skill_id.to_string(),
        card,
        instructions,
    }))
}

/// Scan one configured root, verify the discovery document it produces, and
/// return its indexed skills.
fn index_root(root: &SkillRootConfig, capability: &str) -> CapabilityResult<Vec<IndexedSkill>> {
    let root_metadata = match std::fs::symlink_metadata(&root.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(capability_error(
                capability,
                format!("stat {}: {error}", root.path.display()),
            ));
        }
    };
    if root_metadata.file_type().is_symlink() {
        return Err(capability_error(
            capability,
            format!(
                "configured skill root '{}' is a symlink",
                root.path.display()
            ),
        ));
    }
    if !root_metadata.is_dir() {
        return Ok(Vec::new());
    }
    let root_canonical = std::fs::canonicalize(&root.path).map_err(|error| {
        capability_error(
            capability,
            format!("canonicalize {}: {error}", root.path.display()),
        )
    })?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&root.path)
        .map_err(|error| {
            capability_error(capability, format!("read {}: {error}", root.path.display()))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            capability_error(capability, format!("read {}: {error}", root.path.display()))
        })?
        .into_iter()
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    let mut skills = Vec::new();
    for directory in entries {
        if let Some(skill) = index_one(root, &root_canonical, &directory, capability)? {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| (&a.root_id, &a.skill_id).cmp(&(&b.root_id, &b.skill_id)));

    if skills.is_empty() {
        return Ok(skills);
    }

    // The index serves nothing it has not first held against the published
    // discovery-root contract. This is the wiring that gives
    // `apxm.skill-discovery-root` a reader instead of a description.
    let document = serde_json::json!({
        "schema_version": "apxm.skill-discovery-root",
        "root_id": root.root_id,
        "root_tier": root.tier,
        "root_ref": root.path.display().to_string(),
        "discovery_mode": "metadata_only",
        "skill_cards": skills.iter().map(|skill| &skill.card).collect::<Vec<_>>(),
        "constraints": {
            "body_loaded_on_call": true,
            "activates_on_listing": false,
            "mutates_authority": false
        }
    });
    let verdict = verify_skill_discovery_root_json(&document);
    if !verdict.is_accepted() {
        let reasons = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code.slug(), diagnostic.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(capability_error(
            capability,
            format!(
                "discovery root '{}' does not satisfy apxm.skill-discovery-root: {reasons}",
                root.root_id
            ),
        ));
    }
    Ok(skills)
}

/// Every indexed skill across every configured root, in a stable order.
fn index_all(roots: &[SkillRootConfig], capability: &str) -> CapabilityResult<Vec<IndexedSkill>> {
    let mut all = Vec::new();
    for root in roots {
        all.extend(index_root(root, capability)?);
    }
    all.sort_by(|a, b| (&a.root_id, &a.skill_id).cmp(&(&b.root_id, &b.skill_id)));
    Ok(all)
}

/// Render cards as deterministic JSON text.
fn render_cards(capability: &str, skills: &[IndexedSkill]) -> CapabilityResult<Value> {
    let cards: Vec<&BTreeMap<String, serde_json::Value>> =
        skills.iter().map(|skill| &skill.card).collect();
    let content = serde_json::to_string_pretty(&cards).unwrap_or_else(|_| "[]".to_string());
    untrusted_content_value(capability, "skill://discovery", content)
}

/// Re-check the indexed body immediately before loading it. Indexing is a
/// discovery pass, not a capability grant: a root or body replaced after that
/// pass must not turn `read_skill` into a path traversal primitive.
fn read_instruction(
    root: &Path,
    instructions: &Path,
    capability: &str,
) -> CapabilityResult<String> {
    // The descriptor boundary opens the body without following any component
    // and caps the read itself. The index is only discovery; the body is
    // intentionally opened again by read_skill.
    let bytes =
        secure_read_under_root(root, instructions, MAX_INSTRUCTION_BYTES).map_err(|error| {
            capability_error(
                capability,
                format!("securely read {}: {error}", instructions.display()),
            )
        })?;
    String::from_utf8(bytes).map_err(|error| {
        capability_error(
            capability,
            format!("{} is not valid UTF-8: {error}", instructions.display()),
        )
    })
}

// ---------------------------------------------------------------------------
// list_skills
// ---------------------------------------------------------------------------

/// List every discoverable skill as a metadata-only card.
pub struct ListSkillsCapability {
    metadata: RuntimeCapability,
    config: SkillsConfig,
}

impl ListSkillsCapability {
    pub fn new() -> Self {
        Self::with_config(SkillsConfig::default())
    }

    pub fn with_config(config: SkillsConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                capabilities::LIST_SKILLS,
                "List the available Agent Skills as metadata-only discovery cards. \
                 Listing never loads a skill body; call read_skill for that.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "root_id": { "type": "string" }
                    }
                }),
            )
            .with_returns("object (untrusted quoted JSON array of skill cards)")
            .with_groups(vec![
                capabilities::groups::SKILLS.to_string(),
                capabilities::groups::DISCOVERY.to_string(),
                capabilities::groups::READ.to_string(),
            ])
            .with_read_only()
            .with_latency(15),
            config,
        }
    }
}

impl Default for ListSkillsCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for ListSkillsCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let skills = index_all(&self.config.roots, &self.metadata.name)?;
        let filtered = filter_by_root(skills, &args);
        render_cards(&self.metadata.name, &filtered)
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

fn filter_by_root(skills: Vec<IndexedSkill>, args: &HashMap<String, Value>) -> Vec<IndexedSkill> {
    match args.get("root_id").and_then(|value| value.as_str()) {
        Some(root_id) => skills
            .into_iter()
            .filter(|skill| skill.root_id == root_id)
            .collect(),
        None => skills,
    }
}

// ---------------------------------------------------------------------------
// search_skills
// ---------------------------------------------------------------------------

/// Search discovery cards by a case-insensitive term over their routing text.
pub struct SearchSkillsCapability {
    metadata: RuntimeCapability,
    config: SkillsConfig,
}

impl SearchSkillsCapability {
    pub fn new() -> Self {
        Self::with_config(SkillsConfig::default())
    }

    pub fn with_config(config: SkillsConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                capabilities::SEARCH_SKILLS,
                "Search the available Agent Skills by name, description, when-to-use \
                 text, or tag. Returns metadata-only cards; call read_skill to load one.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "root_id": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            )
            .with_returns("object (untrusted quoted JSON array of matching skill cards)")
            .with_groups(vec![
                capabilities::groups::SKILLS.to_string(),
                capabilities::groups::DISCOVERY.to_string(),
                capabilities::groups::SEARCH.to_string(),
            ])
            .with_read_only()
            .with_latency(20),
            config,
        }
    }
}

impl Default for SearchSkillsCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for SearchSkillsCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let query = require_string_arg(&args, "query", &self.metadata.name)?
            .trim()
            .to_lowercase();
        let skills = index_all(&self.config.roots, &self.metadata.name)?;
        let matched = filter_by_root(skills, &args)
            .into_iter()
            .filter(|skill| {
                if query.is_empty() {
                    return true;
                }
                ["skill_id", "name", "description", "when_to_use", "tags"]
                    .iter()
                    .filter_map(|field| skill.card.get(*field))
                    .any(|value| value.to_string().to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        render_cards(&self.metadata.name, &matched)
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

// ---------------------------------------------------------------------------
// read_skill
// ---------------------------------------------------------------------------

/// Load exactly one skill's instruction body.
///
/// This is the only capability of the three that loads a body, which is what
/// `body_loaded_on_call: true` means: discovery advertises, a call activates.
pub struct ReadSkillCapability {
    metadata: RuntimeCapability,
    config: SkillsConfig,
    inline_skills: Vec<InlineSkill>,
}

impl ReadSkillCapability {
    pub fn new() -> Self {
        Self::with_config(SkillsConfig::default())
    }

    pub fn with_config(config: SkillsConfig) -> Self {
        Self::with_config_and_inline_skills(config, Vec::new())
    }

    pub fn with_config_and_inline_skills(
        config: SkillsConfig,
        inline_skills: Vec<InlineSkill>,
    ) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                capabilities::READ_SKILL,
                "Load one Agent Skill's instructions by id. Use list_skills or \
                 search_skills first to find the id.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "skill_id": { "type": "string" },
                        "root_id": { "type": "string" }
                    },
                    "required": ["skill_id"]
                }),
            )
            .with_returns("object (untrusted quoted skill instruction document)")
            .with_groups(vec![
                capabilities::groups::SKILLS.to_string(),
                capabilities::groups::READ.to_string(),
            ])
            .with_read_only()
            .with_latency(20),
            config,
            inline_skills,
        }
    }
}

impl Default for ReadSkillCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for ReadSkillCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let skill_id = require_string_arg(&args, "skill_id", &self.metadata.name)?.to_string();
        let skills = index_all(&self.config.roots, &self.metadata.name)?;
        let candidates = filter_by_root(skills, &args);

        let file_matches: Vec<&IndexedSkill> = candidates
            .iter()
            .filter(|skill| skill.skill_id == skill_id)
            .collect();
        let inline_matches: Vec<&InlineSkill> = self
            .inline_skills
            .iter()
            .filter(|skill| skill.skill_id == skill_id)
            .filter(|_| match args.get("root_id").and_then(Value::as_str) {
                Some(root_id) => root_id == INLINE_ROOT_ID,
                None => true,
            })
            .collect();
        let inline_bytes = self.inline_skills.iter().fold(0usize, |total, skill| {
            total.saturating_add(skill.text.len())
        });
        if inline_bytes > MAX_INLINE_SKILL_BYTES {
            return Err(capability_error(
                &self.metadata.name,
                format!(
                    "inline skill bodies exceed the {MAX_INLINE_SKILL_BYTES} byte aggregate \
                     instruction limit"
                ),
            ));
        }
        match (file_matches.as_slice(), inline_matches.as_slice()) {
            ([], []) => Err(capability_error(
                &self.metadata.name,
                format!(
                    "no skill '{skill_id}' in the configured discovery roots or inline source; \
                     call list_skills to see what is available"
                ),
            )),
            ([skill], []) => {
                let body =
                    read_instruction(&skill.root_path, &skill.instructions, &self.metadata.name)?;
                untrusted_content_value(
                    &self.metadata.name,
                    format!("skill://{}/{}", skill.root_id, skill.skill_id),
                    body,
                )
            }
            ([], [skill]) => {
                if skill.text.is_empty() || skill.text.len() > MAX_INSTRUCTION_BYTES {
                    return Err(capability_error(
                        &self.metadata.name,
                        format!(
                            "inline skill '{}' is empty or exceeds the {MAX_INSTRUCTION_BYTES} \
                             byte instruction limit",
                            skill.skill_id
                        ),
                    ));
                }
                untrusted_content_value(
                    &self.metadata.name,
                    format!("skill://{INLINE_ROOT_ID}/{}", skill.skill_id),
                    skill.text.clone(),
                )
            }
            _ => {
                let mut roots = file_matches
                    .iter()
                    .map(|skill| skill.root_id.as_str())
                    .collect::<Vec<_>>();
                if !inline_matches.is_empty() {
                    roots.push(INLINE_ROOT_ID);
                }
                Err(capability_error(
                    &self.metadata.name,
                    format!(
                        "skill '{skill_id}' is published by more than one root ({}); name one \
                         with root_id",
                        roots.join(", ")
                    ),
                ))
            }
        }
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_skill_read_enforces_instruction_bound() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let directory = temporary.path().join("oversized");
        std::fs::create_dir(&directory).expect("create skill directory");
        let instructions = directory.join(INSTRUCTIONS);
        std::fs::write(&instructions, vec![b'x'; MAX_INSTRUCTION_BYTES + 1])
            .expect("write oversized instructions");

        let error = read_instruction(temporary.path(), &instructions, "read_skill")
            .expect_err("an oversized body must fail closed");
        let error = format!("{error}");
        assert!(
            error.contains("byte instruction limit") || error.contains("exceeds configured size"),
            "unexpected bounded-read refusal: {error}"
        );
    }

    fn write_skill(root: &Path, id: &str, front: &str, body: &str) {
        let directory = root.join(id);
        std::fs::create_dir_all(&directory).expect("create skill directory");
        std::fs::write(
            directory.join(INSTRUCTIONS),
            format!("---\n{front}\n---\n\n{body}\n"),
        )
        .expect("write instructions");
    }

    fn root_of(path: &Path) -> SkillsConfig {
        SkillsConfig {
            enabled: true,
            roots: vec![SkillRootConfig {
                root_id: "project".to_string(),
                tier: RootTier::Project,
                path: path.to_path_buf(),
            }],
        }
    }

    fn envelope_content(value: &Value) -> String {
        let wire = serde_json::to_value(value).expect("serialize untrusted skill envelope");
        assert_eq!(wire["kind"], "untrusted_content");
        assert_eq!(wire["trust"], "untrusted");
        wire["items"][0]["content"]
            .as_str()
            .expect("untrusted skill envelope content")
            .to_owned()
    }

    #[tokio::test]
    async fn listing_publishes_cards_and_reading_loads_exactly_one_body() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: review\ndescription: Review a change set.",
            "# Review\nDo the review.",
        );
        write_skill(
            temporary.path(),
            "triage",
            "name: triage\ndescription: Sort an incoming issue.",
            "# Triage\nSort it.",
        );

        let config = root_of(temporary.path());
        let listed = ListSkillsCapability::with_config(config.clone())
            .execute(HashMap::new())
            .await
            .expect("listing succeeds");
        let listed = envelope_content(&listed);
        assert!(listed.contains("\"skill_id\": \"review\""), "{listed}");
        assert!(listed.contains("\"skill_id\": \"triage\""), "{listed}");
        // Discovery is metadata-only: a listing advertises, it does not activate.
        assert!(!listed.contains("Do the review."), "{listed}");

        let body = ReadSkillCapability::with_config(config)
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("review".into()),
            )]))
            .await
            .expect("reading succeeds");
        let body = envelope_content(&body);
        assert!(body.contains("Do the review."), "{body}");
        assert!(
            !body.contains("Sort it."),
            "one call loads one body: {body}"
        );
    }

    #[tokio::test]
    async fn searching_matches_routing_text_and_never_returns_a_body() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: review\ndescription: Review a change set.\ntags: quality, review",
            "# Review\nsecret body text",
        );
        write_skill(
            temporary.path(),
            "triage",
            "name: triage\ndescription: Sort an incoming issue.",
            "# Triage",
        );

        let search = SearchSkillsCapability::with_config(root_of(temporary.path()));
        let hits = search
            .execute(HashMap::from([(
                "query".to_string(),
                Value::String("quality".into()),
            )]))
            .await
            .expect("search succeeds");
        let hits = envelope_content(&hits);
        assert!(hits.contains("\"skill_id\": \"review\""), "{hits}");
        assert!(!hits.contains("triage"), "{hits}");
        assert!(!hits.contains("secret body text"), "{hits}");
    }

    #[tokio::test]
    async fn an_unknown_skill_is_refused_rather_than_guessed_at() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: review\ndescription: Review a change set.",
            "# Review",
        );
        let error = ReadSkillCapability::with_config(root_of(temporary.path()))
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("absent".into()),
            )]))
            .await
            .expect_err("an unknown skill is an error");
        assert!(format!("{error}").contains("no skill 'absent'"), "{error}");
    }

    #[tokio::test]
    async fn an_inline_skill_is_read_without_a_filesystem_root() {
        let port = ReadSkillCapability::with_config_and_inline_skills(
            SkillsConfig::default(),
            vec![InlineSkill::new(
                "review",
                "# Review\nUse the submitted change.",
            )],
        );
        let body = port
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("review".into()),
            )]))
            .await
            .expect("inline skill read");
        let body = envelope_content(&body);
        assert_eq!(body, "# Review\nUse the submitted change.");
    }

    #[tokio::test]
    async fn mounted_and_inline_skills_require_an_explicit_root_when_ids_overlap() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: review\ndescription: Mounted review instructions.",
            "Mounted review body.",
        );
        let port = ReadSkillCapability::with_config_and_inline_skills(
            root_of(temporary.path()),
            vec![InlineSkill::new("review", "Inline review body.")],
        );

        let error = port
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("review".into()),
            )]))
            .await
            .expect_err("an overlapping id must not choose a source implicitly");
        assert!(format!("{error}").contains("more than one root"), "{error}");

        let inline = port
            .execute(HashMap::from([
                ("skill_id".to_string(), Value::String("review".into())),
                ("root_id".to_string(), Value::String("inline".into())),
            ]))
            .await
            .expect("explicit inline root");
        assert_eq!(envelope_content(&inline), "Inline review body.");

        let mounted = port
            .execute(HashMap::from([
                ("skill_id".to_string(), Value::String("review".into())),
                ("root_id".to_string(), Value::String("project".into())),
            ]))
            .await
            .expect("explicit mounted root");
        assert_eq!(envelope_content(&mounted), "Mounted review body.\n");
    }

    #[tokio::test]
    async fn an_inline_skill_respects_the_instruction_ceiling() {
        let port = ReadSkillCapability::with_config_and_inline_skills(
            SkillsConfig::default(),
            vec![InlineSkill::new(
                "oversized",
                "x".repeat(MAX_INSTRUCTION_BYTES + 1),
            )],
        );
        let error = port
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("oversized".into()),
            )]))
            .await
            .expect_err("oversized inline body");
        assert!(format!("{error}").contains("instruction limit"), "{error}");
    }

    #[tokio::test]
    async fn inline_skill_bodies_have_an_aggregate_ceiling() {
        let body = "x".repeat(MAX_INLINE_SKILL_BYTES / 2 + 1);
        let port = ReadSkillCapability::with_config_and_inline_skills(
            SkillsConfig::default(),
            vec![
                InlineSkill::new("first", body.clone()),
                InlineSkill::new("second", body),
            ],
        );
        let error = port
            .execute(HashMap::from([(
                "skill_id".to_string(),
                Value::String("first".into()),
            )]))
            .await
            .expect_err("aggregate inline body limit");
        assert!(
            format!("{error}").contains("aggregate instruction limit"),
            "{error}"
        );
    }

    /// A directory whose frontmatter name disagrees with its directory name
    /// would answer to two ids. The index refuses the whole root rather than
    /// publishing a card whose id depends on which field a reader consulted.
    #[tokio::test]
    async fn a_name_that_disagrees_with_its_directory_is_refused() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: reviewer\ndescription: Review a change set.",
            "# Review",
        );
        let error = ListSkillsCapability::with_config(root_of(temporary.path()))
            .execute(HashMap::new())
            .await
            .expect_err("an ambiguous id is an error");
        assert!(
            format!("{error}").contains("declares name 'reviewer'"),
            "{error}"
        );
    }

    /// The index serves nothing it has not first held against the published
    /// discovery contract. A directory name that is not a contract identifier
    /// produces a card the verifier rejects, and the whole root is refused —
    /// the rule lives in `apxm.skill-discovery-root`'s reader, not restated here.
    #[tokio::test]
    async fn a_root_that_fails_the_published_contract_is_refused() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "not an identifier",
            "name: not an identifier\ndescription: Review a change set.",
            "# Review",
        );
        let error = ListSkillsCapability::with_config(root_of(temporary.path()))
            .execute(HashMap::new())
            .await
            .expect_err("a root that fails its own contract is not served");
        let error = format!("{error}");
        assert!(
            error.contains("apxm.skill-discovery-root") && error.contains("invalid_identifier"),
            "{error}"
        );
    }

    /// A global root publishes only shared skills. The index marks its cards
    /// shared so the document it builds satisfies the contract it is checked
    /// against, rather than being refused for a tier decision the host made.
    #[tokio::test]
    async fn a_global_root_publishes_shared_cards() {
        let temporary = tempfile::tempdir().expect("temporary root");
        write_skill(
            temporary.path(),
            "review",
            "name: review\ndescription: Review a change set.",
            "# Review",
        );
        let mut config = root_of(temporary.path());
        config.roots[0].tier = RootTier::Global;
        let listed = ListSkillsCapability::with_config(config)
            .execute(HashMap::new())
            .await
            .expect("a global root of shared skills lists");
        assert!(envelope_content(&listed).contains("\"shared\": true"));
    }

    #[tokio::test]
    async fn an_empty_or_absent_root_lists_nothing_rather_than_failing() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let mut config = root_of(temporary.path());
        config.roots.push(SkillRootConfig {
            root_id: "absent".into(),
            tier: RootTier::Local,
            path: temporary.path().join("does-not-exist"),
        });
        let listed = ListSkillsCapability::with_config(config)
            .execute(HashMap::new())
            .await
            .expect("an absent root is not an error");
        assert_eq!(envelope_content(&listed), "[]");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_root_and_skill_directory_are_refused() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary root");
        let outside = tempfile::tempdir().expect("outside root");
        write_skill(
            outside.path(),
            "secret",
            "name: secret\ndescription: outside",
            "do not expose",
        );

        let root_link = temporary.path().join("root-link");
        symlink(outside.path(), &root_link).expect("root symlink");
        let root_error = ListSkillsCapability::with_config(root_of(&root_link))
            .execute(HashMap::new())
            .await
            .expect_err("a configured root symlink is not trusted");
        assert!(
            format!("{root_error}").contains("root") && format!("{root_error}").contains("symlink")
        );

        let root = temporary.path().join("skills");
        std::fs::create_dir(&root).expect("real root");
        symlink(outside.path().join("secret"), root.join("secret"))
            .expect("skill directory symlink");
        let skill_error = ListSkillsCapability::with_config(root_of(&root))
            .execute(HashMap::new())
            .await
            .expect_err("a symlinked skill directory is not trusted");
        assert!(
            format!("{skill_error}").contains("skill directory")
                && format!("{skill_error}").contains("symlink")
        );
    }

    #[test]
    fn frontmatter_lists_are_normalized_on_read() {
        assert_eq!(
            identifier_list(Some(&"review, quality, review".to_string())),
            vec!["quality".to_string(), "review".to_string()]
        );
        assert_eq!(
            identifier_list(Some(&"[\"b\", \"a\"]".to_string())),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(identifier_list(None).is_empty());
    }
}
