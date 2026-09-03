//! Compilation Service Composition Root.
//!
//! Binds package snapshot validation, frontend selection, source-port compile,
//! package checks, and artifact commit. It does not depend on runtime execution.

mod stdio;

pub use stdio::{
    COMPILATION_CHANNEL, MAX_FRAME_BYTES, MAX_FRAMES_PER_CONNECTION, StdioFrame,
    UNIX_IO_TIMEOUT_MS, UnixEndpoint, decode_jsonl, encode_jsonl, handshake_cross_wired,
    serve_stdio, serve_unix,
};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use apxm_ais::permissions::{LayerDecisions, PermissionDecision, PermissionResolution};
use apxm_ais::{SLOT_CAPABILITY_REF, SemanticOpKind};
use apxm_compilation_protocol::{
    CompilationHandshake, CompilationRequest, CompilationResult, ProtocolError,
};
use apxm_core::types::host_capability::{
    ManifestCapabilities, host_capability_ref, minted_host_capability_refs,
};
use apxm_program::{ExecutableArtifact, air::AirModule};
use apxm_source_port::{
    Frontend, FrontendDrivers, FrontendRoots, PackageSnapshot, SnapshotError, SourceBundleRequest,
    compile_source_bundle, content_digest,
};

pub use apxm_source_port::{
    CAPTURE_SCRATCH_DIR_VARIABLE, CONFINEMENT_BOUNDARY, CONFINEMENT_MODE_VARIABLE,
    ConfinementMode, ConfinementReadiness, ConfinementStatus, capture_confinement_readiness,
};

/// In-memory artifact store used to prove commit versus crash reconciliation.
#[derive(Default)]
pub struct ArtifactStore {
    committed: BTreeMap<String, String>,
}

impl ArtifactStore {
    /// Record a committed digest. Uncertain commits never appear here.
    pub fn commit(&mut self, digest: String, bytes: String) {
        self.committed.insert(digest, bytes);
    }

    /// Look up a previously committed artifact.
    #[must_use]
    pub fn get(&self, digest: &str) -> Option<&str> {
        self.committed.get(digest).map(String::as_str)
    }

    /// Whether any artifact has been committed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.committed.is_empty()
    }
}

/// Compilation Service handler over the native protocol.
pub struct CompilationService {
    store: ArtifactStore,
    idempotency: BTreeMap<String, String>,
    roots: FrontendRoots,
    drivers: FrontendDrivers,
    artifact_dir: Option<PathBuf>,
}

impl Default for CompilationService {
    fn default() -> Self {
        Self::from_env()
    }
}

impl CompilationService {
    /// Bind exact frontend package roots and interpreter drivers.
    #[must_use]
    pub fn with_frontends(roots: FrontendRoots, drivers: FrontendDrivers) -> Self {
        Self {
            store: ArtifactStore::default(),
            idempotency: BTreeMap::new(),
            roots,
            drivers,
            artifact_dir: None,
        }
    }

    /// Product handler. Persists committed artifacts when `APXM_ARTIFACT_DIR` is set.
    #[must_use]
    pub fn from_env() -> Self {
        let mut service =
            Self::with_frontends(declared_frontend_roots(), declared_frontend_drivers());
        if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR")
            && !dir.trim().is_empty()
        {
            service.artifact_dir = Some(PathBuf::from(dir));
        }
        service
    }

    /// Persist committed executable artifacts under `dir` so a Runtime child
    /// can load and verify the exact canonical envelope.
    #[must_use]
    pub fn with_artifact_dir(mut self, dir: PathBuf) -> Self {
        self.artifact_dir = Some(dir);
        self
    }

    /// Admit a handshake and request. Runtime methods are unrepresentable.
    pub fn handle(
        &mut self,
        handshake: &CompilationHandshake,
        request: CompilationRequest,
    ) -> Result<CompilationResult, ProtocolError> {
        handshake.admit()?;
        match request {
            CompilationRequest::Compile {
                request_id,
                idempotency_key,
                snapshot,
            } => self.compile(request_id, idempotency_key, snapshot),
            CompilationRequest::Cancel {
                request_id,
                target_request_id,
            } => {
                if request_id.trim().is_empty() || target_request_id.trim().is_empty() {
                    return Err(ProtocolError::InvalidRequest);
                }
                Ok(CompilationResult::Cancelled { request_id })
            }
        }
    }

    /// Committed artifacts only. Failed compiles have no store entry.
    #[must_use]
    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }

    fn compile(
        &mut self,
        request_id: String,
        idempotency_key: String,
        snapshot: PackageSnapshot,
    ) -> Result<CompilationResult, ProtocolError> {
        if request_id.trim().is_empty() || idempotency_key.trim().is_empty() {
            return Err(ProtocolError::InvalidRequest);
        }
        if let Err(error) = snapshot.validate() {
            return Ok(failed(&request_id, snapshot_error_code(error)));
        }
        let fingerprint = snapshot.snapshot_digest.clone();
        if let Some(prior) = self.idempotency.get(&idempotency_key)
            && prior != &fingerprint
        {
            return Err(ProtocolError::ConflictingIdempotency);
        }
        self.idempotency.insert(idempotency_key, fingerprint);

        match compile_snapshot(&snapshot, &self.roots, &self.drivers) {
            Ok(artifact_json) => {
                let artifact = ExecutableArtifact::decode(artifact_json.as_bytes())
                    .map_err(|_| ProtocolError::InvalidRequest)?;
                let artifact_digest = artifact.artifact_digest.clone();
                let execution_lineage_ref = artifact
                    .execution_lineage_ref
                    .clone()
                    .ok_or(ProtocolError::InvalidRequest)?;
                if let Some(dir) = &self.artifact_dir
                    && persist_artifact(dir, &artifact_digest, artifact_json.as_bytes()).is_err()
                {
                    return Ok(failed(&request_id, "artifact_persist"));
                }
                self.store.commit(artifact_digest.clone(), artifact_json);
                Ok(CompilationResult::ArtifactCommitted {
                    request_id,
                    artifact_digest,
                    artifact: Box::new(artifact),
                    execution_lineage_ref,
                    build_key: format!("{}:{}", snapshot.frontend.wire(), snapshot.snapshot_digest),
                })
            }
            Err(code) => Ok(failed(&request_id, &code)),
        }
    }
}

fn failed(request_id: &str, code: &str) -> CompilationResult {
    CompilationResult::Failed {
        request_id: request_id.to_owned(),
        code: code.to_owned(),
    }
}

/// File name for a digest in a shared artifact directory (`:` is not portable).
#[must_use]
pub fn artifact_file_name(digest: &str) -> String {
    digest.replace(':', "-")
}

fn persist_artifact(dir: &Path, digest: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let directory = std::fs::symlink_metadata(dir).map_err(|error| error.to_string())?;
    if directory.file_type().is_symlink() || !directory.is_dir() {
        return Err(format!(
            "artifact directory '{}' is not a real directory",
            dir.display()
        ));
    }
    let path = dir.join(artifact_file_name(digest));
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "artifact path '{}' is not a regular file",
                    path.display()
                ));
            }
            let existing = std::fs::read(&path).map_err(|error| error.to_string())?;
            if existing == bytes {
                return Ok(());
            }
            return Err(format!(
                "artifact path '{}' already contains different bytes",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }

    use std::io::Write as _;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another compiler worker won the create race. Reconcile only
            // against the digest-bound bytes; never truncate or follow a
            // path that appeared after the initial check.
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "artifact path '{}' is not a regular file",
                    path.display()
                ));
            }
            let existing = std::fs::read(&path).map_err(|e| e.to_string())?;
            return if existing == bytes {
                Ok(())
            } else {
                Err(format!(
                    "artifact path '{}' already contains different bytes",
                    path.display()
                ))
            };
        }
        Err(error) => return Err(error.to_string()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

fn snapshot_error_code(error: SnapshotError) -> &'static str {
    match error {
        SnapshotError::UnsupportedContract => "unsupported_contract",
        SnapshotError::Incomplete => "incomplete_snapshot",
        SnapshotError::UnsafePath => "unsafe_path",
        SnapshotError::DigestMismatch => "digest_mismatch",
        SnapshotError::DuplicatePath => "duplicate_path",
        SnapshotError::LockDrift => "lock_drift",
        SnapshotError::MissingEntrypoint => "missing_entrypoint",
    }
}

fn compile_snapshot(
    snapshot: &PackageSnapshot,
    roots: &FrontendRoots,
    drivers: &FrontendDrivers,
) -> Result<String, String> {
    let manifest = declared_manifest(snapshot)?;
    if manifest.frontend != snapshot.frontend {
        return Err("frontend_mismatch".to_owned());
    }
    if manifest.entry != snapshot.entrypoint {
        return Err("entrypoint_mismatch".to_owned());
    }
    verify_integrity(snapshot)?;

    let entry = snapshot
        .file(&snapshot.entrypoint)
        .ok_or_else(|| "missing_entrypoint".to_owned())?;
    let source =
        String::from_utf8(entry.bytes.clone()).map_err(|_| "entrypoint_not_utf8".to_owned())?;
    let program = authored_program_name(snapshot.frontend, &source)?;
    let compiled = compile_source_bundle(
        &SourceBundleRequest::new(snapshot.frontend, program, source)
            .with_host_capabilities(manifest.host_capabilities.clone()),
        roots,
        drivers,
    )
    .map_err(|diagnostics| {
        diagnostics.first().map_or_else(
            || "compile_failed".to_owned(),
            |diagnostic| {
                if diagnostic.message.is_empty() {
                    diagnostic.code.slug().to_owned()
                } else {
                    format!("{}: {}", diagnostic.code.slug(), diagnostic.message)
                }
            },
        )
    })?;
    check_capability_references(snapshot, &manifest, &compiled.air)?;
    check_package_permissions(snapshot, &compiled.air, &manifest)?;
    let artifact = ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&artifact).map_err(|error| error.to_string())
}

#[derive(Debug, serde::Deserialize)]
struct AgentManifest {
    #[serde(default)]
    compile: Option<CompileManifest>,
    #[serde(default)]
    capabilities: ManifestCapabilities,
    #[serde(default)]
    permissions: BTreeMap<String, PermissionDecision>,
}

#[derive(Debug, serde::Deserialize)]
struct CompileManifest {
    entry: Option<String>,
    frontend: Option<Frontend>,
}

struct DeclaredManifest {
    frontend: Frontend,
    entry: String,
    /// The host capability ids `[[capabilities.host]]` declares, without the
    /// reserved prefix. Passed into capture so the frontend's minted set is the
    /// catalogue united with these, and united into the granted set so the
    /// admission below admits the references they mint.
    host_capabilities: Vec<String>,
    permissions: BTreeMap<String, PermissionDecision>,
}

fn declared_manifest(snapshot: &PackageSnapshot) -> Result<DeclaredManifest, String> {
    let Some(agent) = snapshot.file("agent.toml") else {
        return Err("missing_frontend".to_owned());
    };
    let text = String::from_utf8(agent.bytes.clone()).map_err(|_| "invalid_manifest".to_owned())?;
    let parsed: AgentManifest = toml::from_str(&text).map_err(|_| "invalid_manifest".to_owned())?;
    let compile = parsed
        .compile
        .ok_or_else(|| "missing_frontend".to_owned())?;
    let frontend = compile
        .frontend
        .ok_or_else(|| "missing_frontend".to_owned())?;
    let entry = compile
        .entry
        .ok_or_else(|| "missing_entrypoint".to_owned())?;
    let host_capabilities = parsed
        .capabilities
        .host
        .iter()
        .map(|declaration| declaration.id.clone())
        .collect::<Vec<_>>();
    minted_host_capability_refs(&parsed.capabilities.host)
        .map_err(|_| "invalid_manifest".to_owned())?;
    Ok(DeclaredManifest {
        frontend,
        entry,
        host_capabilities,
        permissions: parsed.permissions,
    })
}

fn authored_program_name(frontend: Frontend, source: &str) -> Result<String, String> {
    let mut found = None;
    match frontend {
        Frontend::Python => {
            // Keep the source scan linear. A package may contain many child
            // Agents, and rescanning the suffix after every decorator makes
            // authored-program discovery quadratic in source size.
            let mut awaiting_agent = false;
            let mut decorator_depth = 0usize;
            let mut python_string = None;
            for line in source.lines() {
                let code = strip_python_non_code(line, &mut python_string);
                let trimmed = code.trim_start();
                // A source comment or string is data, not an Agent
                // declaration. Only a decorator at the beginning of a Python
                // logical line may arm discovery.
                if trimmed.starts_with('#') {
                    continue;
                }
                if is_python_agent_decorator(trimmed) {
                    awaiting_agent = true;
                    decorator_depth = parenthesis_depth(trimmed);
                    continue;
                }
                if !awaiting_agent {
                    continue;
                }
                if decorator_depth > 0 {
                    let closes = trimmed.bytes().filter(|byte| *byte == b')').count();
                    let opens = trimmed.bytes().filter(|byte| *byte == b'(').count();
                    decorator_depth = decorator_depth.saturating_add(opens).saturating_sub(closes);
                    continue;
                }
                if trimmed.is_empty() || trimmed.starts_with('@') {
                    continue;
                }
                let rest = trimmed
                    .strip_prefix("async def ")
                    .or_else(|| trimmed.strip_prefix("def "));
                let Some(rest) = rest else {
                    // Do not let an arbitrary statement after a decorator
                    // bind a later function name.
                    awaiting_agent = false;
                    continue;
                };
                let name = rest.split('(').next().unwrap_or("").trim();
                awaiting_agent = false;
                if !is_identifier(name) {
                    continue;
                }
                // A composition root may declare child Agent Programs in
                // the same file. The last `@Agent` is the package entry.
                found = Some(name.to_owned());
            }
        }
        Frontend::Typescript => {
            let mut in_block_comment = false;
            for line in source.lines() {
                let line = strip_typescript_comments(line, &mut in_block_comment);
                let trimmed = line.trim_start();
                let Some((before, after)) = trimmed.split_once('=') else {
                    continue;
                };
                let mut tokens = before.split_whitespace();
                let Some(name) = tokens.next_back() else {
                    continue;
                };
                if tokens.next_back() != Some("const")
                    || !tokens.all(|token| token == "export" || token == "declare")
                    || !is_identifier(name)
                {
                    continue;
                }
                let after = after.trim_start();
                if !after.starts_with("Agent")
                    || !after["Agent".len()..]
                        .chars()
                        .next()
                        .is_some_and(|character| {
                            character == '<' || character == '(' || character.is_whitespace()
                        })
                {
                    continue;
                }
                found = Some(name.to_owned());
            }
        }
    }
    found.ok_or_else(|| "missing_program".to_owned())
}

fn is_python_agent_decorator(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("@Agent") else {
        return false;
    };
    rest.is_empty() || rest.trim_start().starts_with('(')
}

fn parenthesis_depth(line: &str) -> usize {
    line.bytes().fold(0usize, |depth, byte| match byte {
        b'(' => depth.saturating_add(1),
        b')' => depth.saturating_sub(1),
        _ => depth,
    })
}

/// Remove Python strings and comments before looking for decorators. The
/// service only needs a tiny lexical view here; the frontend remains the
/// authority for syntax and Agent semantics. Keeping quoted text out of this
/// scan prevents prompt-like examples in docstrings from selecting a fake
/// package root.
fn strip_python_non_code(line: &str, string: &mut Option<(char, bool)>) -> String {
    let mut output = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some((quote, triple)) = *string {
            if triple && bytes[index..].starts_with(&[quote as u8; 3]) {
                *string = None;
                index += 3;
                continue;
            }
            if !triple && bytes[index] == quote as u8 {
                *string = None;
                index += 1;
                continue;
            }
            if bytes[index] == b'\\' {
                index = index.saturating_add(2);
            } else {
                index += 1;
            }
            continue;
        }

        match bytes[index] {
            b'#' => break,
            b'\'' | b'"' => {
                let quote = bytes[index] as char;
                let triple = bytes[index..].starts_with(&[bytes[index]; 3]);
                *string = Some((quote, triple));
                index += if triple { 3 } else { 1 };
            }
            byte => {
                output.push(byte as char);
                index += 1;
            }
        }
    }
    output
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn strip_typescript_comments(line: &str, in_block_comment: &mut bool) -> String {
    let mut output = String::with_capacity(line.len());
    let mut cursor = 0;
    while cursor < line.len() {
        if *in_block_comment {
            let Some(end) = line[cursor..].find("*/") else {
                return output;
            };
            cursor += end + 2;
            *in_block_comment = false;
            continue;
        }
        let remainder = &line[cursor..];
        let line_comment = remainder.find("//");
        let block_comment = remainder.find("/*");
        match (line_comment, block_comment) {
            (Some(line_index), Some(block_index)) if line_index < block_index => {
                output.push_str(&remainder[..line_index]);
                break;
            }
            (Some(line_index), None) => {
                output.push_str(&remainder[..line_index]);
                break;
            }
            (_, Some(block_index)) => {
                output.push_str(&remainder[..block_index]);
                cursor += block_index + 2;
                *in_block_comment = true;
            }
            (None, None) => {
                output.push_str(remainder);
                break;
            }
        }
    }
    output
}

fn verify_integrity(snapshot: &PackageSnapshot) -> Result<(), String> {
    let Some(integrity) = snapshot.file("integrity.toml") else {
        return Ok(());
    };
    let text =
        String::from_utf8(integrity.bytes.clone()).map_err(|_| "integrity_invalid".to_owned())?;
    let recorded: IntegrityToml =
        toml::from_str(&text).map_err(|_| "integrity_invalid".to_owned())?;
    if recorded.algorithm != "sha256" {
        return Err("integrity_invalid".to_owned());
    }
    let mut files = BTreeMap::new();
    for link in &recorded.chain {
        let content = snapshot
            .file(&link.path)
            .ok_or_else(|| "integrity_mismatch".to_owned())?;
        files.insert(link.path.clone(), content.digest.clone());
    }
    let expected = compute_integrity(&files);
    if recorded != expected {
        return Err("integrity_mismatch".to_owned());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct IntegrityToml {
    algorithm: String,
    hash: String,
    chain: Vec<ChainLinkToml>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct ChainLinkToml {
    path: String,
    prev_hash: String,
    hash: String,
}

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn compute_integrity(files: &BTreeMap<String, String>) -> IntegrityToml {
    let mut prev = GENESIS_HASH.to_owned();
    let mut chain = Vec::with_capacity(files.len());
    for (path, digest) in files {
        let mut preimage = String::with_capacity(prev.len() + path.len() + digest.len());
        preimage.push_str(&prev);
        preimage.push_str(path);
        preimage.push_str(digest);
        let hash = content_digest(preimage.as_bytes());
        chain.push(ChainLinkToml {
            path: path.clone(),
            prev_hash: prev.clone(),
            hash: hash.clone(),
        });
        prev = hash;
    }
    let hash = chain
        .last()
        .map_or_else(|| GENESIS_HASH.to_owned(), |link| link.hash.clone());
    IntegrityToml {
        algorithm: "sha256".to_owned(),
        hash,
        chain,
    }
}

fn granted_capability_ids(
    snapshot: &PackageSnapshot,
    manifest: &DeclaredManifest,
) -> BTreeSet<String> {
    let mut granted = apxm_ais::capabilities::BUILTINS
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<BTreeSet<_>>();
    granted.extend(
        manifest
            .host_capabilities
            .iter()
            .map(|id| host_capability_ref(id)),
    );
    for content in &snapshot.contents {
        let Some((id, file)) = content.path.strip_prefix("capabilities/").and_then(|rest| {
            let (id, file) = rest.split_once('/')?;
            Some((id, file))
        }) else {
            continue;
        };
        if file == "handler.py" || file == "handler.ts" {
            granted.insert(id.to_owned());
        }
    }
    granted
}

fn check_capability_references(
    snapshot: &PackageSnapshot,
    manifest: &DeclaredManifest,
    module: &AirModule,
) -> Result<(), String> {
    let granted = granted_capability_ids(snapshot, manifest);
    let mut ungranted: Vec<&str> = module
        .semantic_operations
        .iter()
        .filter(|operation| operation.op == SemanticOpKind::CapabilityInvoke)
        .filter_map(|operation| {
            operation
                .operands
                .iter()
                .find(|operand| operand.slot == SLOT_CAPABILITY_REF)
                .map(|operand| operand.value_id.as_str())
        })
        .filter(|capability_ref| !granted.contains(*capability_ref))
        .collect();
    ungranted.sort_unstable();
    ungranted.dedup();
    if ungranted.is_empty() {
        Ok(())
    } else {
        Err("ungranted_capability".to_owned())
    }
}

fn check_package_permissions(
    snapshot: &PackageSnapshot,
    module: &AirModule,
    manifest: &DeclaredManifest,
) -> Result<(), String> {
    let grantable = granted_capability_ids(snapshot, manifest);
    let requested: LayerDecisions = grantable
        .iter()
        .map(|id| {
            (
                id.clone(),
                module
                    .capability_permission_requests
                    .get(id)
                    .cloned()
                    .unwrap_or_else(PermissionDecision::allow),
            )
        })
        .collect();
    PermissionResolution::resolve_code_over_package(
        requested,
        manifest.permissions.clone().into_iter().collect(),
    )
    .map(|_| ())
    .map_err(|_| "permission_widening".to_owned())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the compilation-service crate sits three levels under the repository root")
        .to_path_buf()
}

fn declared_frontend_roots() -> FrontendRoots {
    let root = workspace_root();
    FrontendRoots::new(
        root.join("crates/compiler/frontend/python"),
        root.join("crates/compiler/frontend/typescript"),
    )
}

fn declared_frontend_drivers() -> FrontendDrivers {
    let root = workspace_root();
    FrontendDrivers::new(
        root.join(".dekk/env/bin/python"),
        root.join(".dekk/env/bin/node"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_compilation_protocol::{COMPILATION_PROTOCOL_VERSION, CompilationRequest};
    use apxm_source_port::{Frontend, SnapshotContent};

    const PYTHON_PROGRAM: &str = r#"from apxm_program import Agent, Model, Tool


class ReviewRequest:
    pass


class Review:
    pass


ReviewModel = Model[ReviewRequest, Review]("review.model")
SearchWeb = Tool[ReviewRequest, Review]("search_web")


@Agent(input=ReviewRequest, output=Review)
async def Reviewer(agent, request):
    evidence = await SearchWeb(request)
    return await ReviewModel(evidence)
"#;

    const TYPESCRIPT_PROGRAM: &str = r#"import { Agent, Model, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type ReviewRequest = object;
type Review = object;

const ReviewModel = Model<ReviewRequest, Review>("review.model");
const SearchWeb = Tool<ReviewRequest, Review>("search_web");

export const Reviewer = Agent<ReviewRequest, Review>({
  name: "Reviewer",
  async run(agent, request) {
    const evidence = await SearchWeb(request);
    return await ReviewModel(evidence);
  },
});
"#;

    fn handshake() -> CompilationHandshake {
        CompilationHandshake {
            protocol_version: COMPILATION_PROTOCOL_VERSION.to_owned(),
        }
    }

    fn frontend_present(frontend: Frontend) -> bool {
        let root = declared_frontend_roots().root(frontend).to_path_buf();
        let driver = declared_frontend_drivers().driver(frontend).to_path_buf();
        if !driver.is_file() {
            return false;
        }
        match frontend {
            Frontend::Python => root.join("apxm_program/_native.so").is_file(),
            Frontend::Typescript => {
                root.join("dist/index.js").is_file()
                    && root.join("dist/node.js").is_file()
                    && root.join("dist/_native.node").is_file()
                    && root
                        .join("node_modules/typescript/lib/typescript.js")
                        .is_file()
            }
        }
    }

    fn agent_toml(frontend: Frontend, entry: &str) -> String {
        format!(
            "id = \"tiny\"\nversion = \"0.1.0\"\nschema_version = \"apxm.agent\"\n\n[compile]\nentry = \"{entry}\"\nfrontend = \"{}\"\n",
            frontend.wire()
        )
    }

    fn package_snapshot(frontend: Frontend, entry: &str, source: &str) -> PackageSnapshot {
        let mut contents = vec![
            SnapshotContent::from_bytes("agent.toml", agent_toml(frontend, entry).into_bytes()),
            SnapshotContent::from_bytes(entry, source.as_bytes().to_vec()),
        ];
        let files = contents
            .iter()
            .map(|content| (content.path.clone(), content.digest.clone()))
            .collect();
        let integrity = compute_integrity(&files);
        contents.push(SnapshotContent::from_bytes(
            "integrity.toml",
            toml::to_string(&integrity).expect("integrity").into_bytes(),
        ));
        PackageSnapshot::assemble(
            frontend,
            entry,
            contents,
            None,
            "apxm.compatibility-set/test",
        )
        .expect("fixture snapshot")
    }

    #[test]
    fn python_and_typescript_commit_through_one_handler() {
        let mut service = CompilationService::default();
        for (frontend, entry, source) in [
            (Frontend::Python, "src/agent.py", PYTHON_PROGRAM),
            (Frontend::Typescript, "src/agent.ts", TYPESCRIPT_PROGRAM),
        ] {
            if !frontend_present(frontend) {
                continue;
            }
            let result = service
                .handle(
                    &handshake(),
                    CompilationRequest::Compile {
                        request_id: frontend.wire().to_owned(),
                        idempotency_key: frontend.wire().to_owned(),
                        snapshot: package_snapshot(frontend, entry, source),
                    },
                )
                .unwrap();
            let CompilationResult::ArtifactCommitted {
                artifact_digest,
                execution_lineage_ref,
                ..
            } = result
            else {
                panic!("commit for {}", frontend.wire());
            };
            assert!(execution_lineage_ref.starts_with("sha256:"));
            let bytes = service
                .store()
                .get(&artifact_digest)
                .expect("store holds committed artifact");
            let artifact = ExecutableArtifact::decode(bytes.as_bytes()).expect("artifact JSON");
            assert_eq!(
                artifact.artifact_digest,
                artifact.canonical_digest().unwrap()
            );
            assert!(artifact.execution_lineage_ref.is_some());
            assert_eq!(artifact.source_map, artifact.air.source_map);
            let air = artifact.air;
            assert!(
                !air.semantic_operations.is_empty(),
                "{} artifact must contain compiled AIR",
                frontend.wire()
            );
        }
    }

    #[test]
    fn last_agent_in_the_entry_file_is_the_package_root() {
        if !frontend_present(Frontend::Python) {
            return;
        }
        let source = r#"
from typing import TypedDict
from apxm_program import Agent, Event, Model

class In(TypedDict):
    message: str

class Out(TypedDict):
    message: str

ChildModel = Model[In, Out]("model.child")
Approval = Event[In]("event.harness.approval")

@Agent(input=In, output=Out)
async def Child(agent, request):
    return await ChildModel(request)

@Agent(input=In, output=Out)
async def Harness(agent, request):
    while True:
        child = Child.new()
        review = await child.invoke(request)
        approved = await Approval.wait()
        request = await agent.yield_(review)
"#;
        let mut service = CompilationService::default();
        let result = service
            .handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: "harness".to_owned(),
                    idempotency_key: "harness".to_owned(),
                    snapshot: package_snapshot(Frontend::Python, "src/agent.py", source),
                },
            )
            .unwrap();
        let CompilationResult::ArtifactCommitted {
            artifact_digest, ..
        } = result
        else {
            panic!("composed package must commit: {result:?}");
        };
        let air = service
            .store()
            .get(&artifact_digest)
            .expect("artifact envelope");
        assert!(air.contains("\"op\":\"program.new\""), "{air}");
        assert!(air.contains("\"op\":\"program.invoke\""), "{air}");
        assert!(air.contains("\"op\":\"await.event\""), "{air}");
    }

    #[test]
    fn failed_compile_has_no_store_entry() {
        let mut service = CompilationService::default();
        if !frontend_present(Frontend::Python) {
            return;
        }
        let result = service
            .handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: "bad".to_owned(),
                    idempotency_key: "bad".to_owned(),
                    snapshot: package_snapshot(
                        Frontend::Python,
                        "src/agent.py",
                        "this is not valid python for an Agent\n",
                    ),
                },
            )
            .unwrap();
        assert!(matches!(result, CompilationResult::Failed { .. }));
        assert!(service.store().is_empty());
    }

    #[test]
    fn missing_frontend_fails_closed() {
        let mut service = CompilationService::default();
        let snapshot = PackageSnapshot::assemble(
            Frontend::Python,
            "src/agent.py",
            vec![SnapshotContent::from_bytes(
                "src/agent.py",
                b"print('no manifest')",
            )],
            None,
            "apxm.compatibility-set/test",
        )
        .unwrap();
        let result = service
            .handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: "missing".to_owned(),
                    idempotency_key: "missing".to_owned(),
                    snapshot,
                },
            )
            .unwrap();
        match result {
            CompilationResult::Failed { code, .. } => assert_eq!(code, "missing_frontend"),
            other => panic!("expected missing frontend, got {other:?}"),
        }
        assert!(service.store().is_empty());
    }

    #[test]
    fn lock_drift_fails_closed() {
        let err = PackageSnapshot::assemble(
            Frontend::Python,
            "src/agent.py",
            vec![
                SnapshotContent::from_bytes("src/agent.py", b"print('ok')"),
                SnapshotContent::from_bytes("uv.lock", b"stale"),
            ],
            Some("not-the-lock-digest".to_owned()),
            "apxm.compatibility-set/test",
        )
        .unwrap_err();
        assert_eq!(err, SnapshotError::LockDrift);
    }

    #[test]
    fn failed_handshake_never_contacts_the_store() {
        let mut service = CompilationService::default();
        let err = service
            .handle(
                &CompilationHandshake {
                    protocol_version: "nope".to_owned(),
                },
                CompilationRequest::Cancel {
                    request_id: "r".to_owned(),
                    target_request_id: "t".to_owned(),
                },
            )
            .unwrap_err();
        assert_eq!(err, ProtocolError::IncompatibleVersion);
        assert!(service.store().get("artifact:x").is_none());
    }

    #[test]
    fn authored_program_discovery_ignores_prompt_like_comments_strings_and_prefixes() {
        let python = r#"
# @Agent(input=Fake, output=Fake)
# async def CommentOnly(agent, request): pass
label = "@Agent"
@Agent(input=In, output=Out)
async def Real(agent, request):
    return request
@AgentFacade(input=In, output=Out)
async def NotAnAgentFacade(agent, request):
    return request
"#;
        assert_eq!(
            authored_program_name(Frontend::Python, python).unwrap(),
            "Real"
        );

        let typescript = r#"
// export const CommentOnly = Agent<In, Out>({});
const label = "= Agent";
/*
const BlockComment = Agent<In, Out>({});
*/
export const Real = Agent<In, Out>({});
const AgentFacade = AgentFacade<In, Out>({});
"#;
        assert_eq!(
            authored_program_name(Frontend::Typescript, typescript).unwrap(),
            "Real"
        );
    }

    #[test]
    fn authored_program_discovery_accepts_multiline_python_agent_decorators() {
        let source = r#"
@Agent(
    input=Input,
    output=Output,
    # A decorator may carry comments while it spans lines.
    context=Context,
)
async def ConversationalExample(agent, request):
    return request
"#;
        assert_eq!(
            authored_program_name(Frontend::Python, source).unwrap(),
            "ConversationalExample"
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifact_persistence_refuses_to_follow_a_symlink() {
        use std::os::unix::fs::symlink;

        let directory = std::env::temp_dir().join(format!(
            "apxm-artifact-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("test directory");
        let target = directory.join("target");
        std::fs::write(&target, b"must survive").expect("target");
        let artifact = directory.join(artifact_file_name("sha256:deadbeef"));
        symlink(&target, &artifact).expect("artifact symlink");

        let error = persist_artifact(&directory, "sha256:deadbeef", b"replacement")
            .expect_err("artifact persistence must not follow a pre-existing symlink");
        assert!(error.contains("not a regular file"));
        assert_eq!(
            std::fs::read(&target).expect("target survives"),
            b"must survive"
        );
        std::fs::remove_file(&artifact).expect("symlink cleanup");
        std::fs::remove_file(&target).expect("target cleanup");
        std::fs::remove_dir(&directory).expect("directory cleanup");
    }

    #[test]
    fn jsonl_round_trip_compiles_python_when_frontend_present() {
        if !frontend_present(Frontend::Python) {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "apxm-compilation-jsonl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let service = CompilationService::default().with_artifact_dir(dir.clone());
        let envelope = serde_json::json!({
            "handshake": handshake(),
            "request": CompilationRequest::Compile {
                request_id: "c".to_owned(),
                idempotency_key: "k".to_owned(),
                snapshot: package_snapshot(Frontend::Python, "src/agent.py", PYTHON_PROGRAM),
            }
        });
        let frame = crate::StdioFrame {
            channel: "compilation".to_owned(),
            payload: envelope.to_string(),
        };
        let mut out = Vec::new();
        crate::serve_stdio(crate::encode_jsonl(&frame).as_bytes(), &mut out, service).unwrap();
        let reply = crate::decode_jsonl(std::str::from_utf8(&out).unwrap()).unwrap();
        let result: CompilationResult = serde_json::from_str(&reply.payload).unwrap();
        let CompilationResult::ArtifactCommitted {
            artifact_digest, ..
        } = result
        else {
            panic!("jsonl compile: {result:?}");
        };
        let persisted = dir.join(crate::artifact_file_name(&artifact_digest));
        assert!(persisted.is_file(), "artifact bytes persisted for runtime");
        let air = std::fs::read_to_string(persisted).unwrap();
        assert!(air.contains("apxm.air"));
    }
}
