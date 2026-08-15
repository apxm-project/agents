//! Compilation Service Composition Root.
//!
//! Binds package snapshot validation, frontend selection, source-port compile,
//! package checks, and artifact commit. It does not depend on runtime execution.

mod stdio;

pub use stdio::{StdioFrame, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use apxm_ais::permissions::{LayerDecisions, PermissionDecision, PermissionResolution};
use apxm_ais::{SLOT_CAPABILITY_REF, SemanticOpKind};
use apxm_compilation_protocol::{
    CompilationHandshake, CompilationRequest, CompilationResult, ProtocolError,
};
use apxm_program::air::AirModule;
use apxm_source_port::{
    Frontend, FrontendDrivers, FrontendRoots, PackageSnapshot, SourceBundleRequest, SnapshotError,
    compile_source_bundle, content_digest,
};
use sha2::{Digest, Sha256};

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
    last_idempotency: Option<(String, String)>,
    roots: FrontendRoots,
    drivers: FrontendDrivers,
}

impl Default for CompilationService {
    fn default() -> Self {
        Self::with_frontends(declared_frontend_roots(), declared_frontend_drivers())
    }
}

impl CompilationService {
    /// Bind exact frontend package roots and interpreter drivers.
    #[must_use]
    pub fn with_frontends(roots: FrontendRoots, drivers: FrontendDrivers) -> Self {
        Self {
            store: ArtifactStore::default(),
            last_idempotency: None,
            roots,
            drivers,
        }
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
                target_request_id: _,
            } => Ok(CompilationResult::Cancelled { request_id }),
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
        if let Err(error) = snapshot.validate() {
            return Ok(failed(&request_id, snapshot_error_code(error)));
        }
        let fingerprint = snapshot.snapshot_digest.clone();
        if let Some((key, prior)) = &self.last_idempotency {
            if key == &idempotency_key && prior != &fingerprint {
                return Err(ProtocolError::ConflictingIdempotency);
            }
        }
        self.last_idempotency = Some((idempotency_key, fingerprint));

        match compile_snapshot(&snapshot, &self.roots, &self.drivers) {
            Ok(air_json) => {
                let artifact_digest = format!("sha256:{:x}", Sha256::digest(air_json.as_bytes()));
                self.store
                    .commit(artifact_digest.clone(), air_json);
                Ok(CompilationResult::ArtifactCommitted {
                    request_id,
                    artifact_digest,
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

fn snapshot_error_code(error: SnapshotError) -> &'static str {
    match error {
        SnapshotError::UnsupportedContract => "unsupported_contract",
        SnapshotError::Incomplete => "incomplete_snapshot",
        SnapshotError::UnsafePath => "unsafe_path",
        SnapshotError::DigestMismatch => "digest_mismatch",
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
    let source = String::from_utf8(entry.bytes.clone())
        .map_err(|_| "entrypoint_not_utf8".to_owned())?;
    let program = authored_program_name(snapshot.frontend, &source)?;
    let compiled = compile_source_bundle(
        &SourceBundleRequest::new(snapshot.frontend, program, source),
        roots,
        drivers,
    )
    .map_err(|diagnostics| {
        diagnostics
            .first()
            .map(|diagnostic| diagnostic.code.slug().to_owned())
            .unwrap_or_else(|| "compile_failed".to_owned())
    })?;
    check_capability_references(snapshot, &compiled.air)?;
    check_package_permissions(snapshot, &compiled.air, &manifest)?;
    serde_json::to_string(&compiled.air).map_err(|error| error.to_string())
}

#[derive(Debug, serde::Deserialize)]
struct AgentManifest {
    #[serde(default)]
    compile: Option<CompileManifest>,
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
    permissions: BTreeMap<String, PermissionDecision>,
}

fn declared_manifest(snapshot: &PackageSnapshot) -> Result<DeclaredManifest, String> {
    let Some(agent) = snapshot.file("agent.toml") else {
        return Err("missing_frontend".to_owned());
    };
    let text =
        String::from_utf8(agent.bytes.clone()).map_err(|_| "invalid_manifest".to_owned())?;
    let parsed: AgentManifest =
        toml::from_str(&text).map_err(|_| "invalid_manifest".to_owned())?;
    let compile = parsed.compile.ok_or_else(|| "missing_frontend".to_owned())?;
    let frontend = compile.frontend.ok_or_else(|| "missing_frontend".to_owned())?;
    let entry = compile.entry.ok_or_else(|| "missing_entrypoint".to_owned())?;
    Ok(DeclaredManifest {
        frontend,
        entry,
        permissions: parsed.permissions,
    })
}

fn authored_program_name(frontend: Frontend, source: &str) -> Result<String, String> {
    let mut found = None;
    match frontend {
        Frontend::Python => {
            let lines: Vec<&str> = source.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if !line.contains("@Agent") {
                    continue;
                }
                for next in &lines[index + 1..] {
                    let trimmed = next.trim_start();
                    let rest = trimmed
                        .strip_prefix("async def ")
                        .or_else(|| trimmed.strip_prefix("def "));
                    let Some(rest) = rest else {
                        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@')
                        {
                            continue;
                        }
                        break;
                    };
                    let name = rest.split('(').next().unwrap_or("").trim();
                    if name.is_empty() {
                        break;
                    }
                    if found.replace(name.to_owned()).is_some() {
                        return Err("ambiguous_entrypoint".to_owned());
                    }
                    break;
                }
            }
        }
        Frontend::Typescript => {
            for line in source.lines() {
                let Some((before, _)) = line.split_once("= Agent") else {
                    continue;
                };
                let name = before
                    .replace("export const", "")
                    .replace("const", "")
                    .trim()
                    .to_owned();
                if name.is_empty() {
                    continue;
                }
                if found.replace(name).is_some() {
                    return Err("ambiguous_entrypoint".to_owned());
                }
            }
        }
    }
    found.ok_or_else(|| "missing_program".to_owned())
}

fn verify_integrity(snapshot: &PackageSnapshot) -> Result<(), String> {
    let Some(integrity) = snapshot.file("integrity.toml") else {
        return Ok(());
    };
    let text = String::from_utf8(integrity.bytes.clone())
        .map_err(|_| "integrity_invalid".to_owned())?;
    let recorded: IntegrityToml =
        toml::from_str(&text).map_err(|_| "integrity_invalid".to_owned())?;
    if recorded.algorithm != "sha256" {
        return Err("integrity_invalid".to_owned());
    }
    let mut files = BTreeMap::new();
    for content in &snapshot.contents {
        if content.path != "integrity.toml" {
            files.insert(content.path.clone(), content.digest.clone());
        }
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

fn granted_capability_ids(snapshot: &PackageSnapshot) -> BTreeSet<String> {
    let mut granted = apxm_ais::capabilities::BUILTINS
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<BTreeSet<_>>();
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
    module: &AirModule,
) -> Result<(), String> {
    let granted = granted_capability_ids(snapshot);
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
    let grantable = granted_capability_ids(snapshot);
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
                artifact_digest, ..
            } = result
            else {
                panic!("commit for {}", frontend.wire());
            };
            let bytes = service
                .store()
                .get(&artifact_digest)
                .expect("store holds committed AIR");
            let air: AirModule = serde_json::from_str(bytes).expect("AIR JSON");
            assert!(
                !air.semantic_operations.is_empty(),
                "{} artifact must be compiled AIR",
                frontend.wire()
            );
        }
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
}
