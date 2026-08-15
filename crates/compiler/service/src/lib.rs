//! Compilation Service Composition Root.
//!
//! Binds package snapshot validation, frontend selection, source-port compile,
//! package checks, and artifact commit. It does not depend on runtime execution.

mod stdio;

pub use stdio::{
    StdioFrame, UnixEndpoint, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio,
    serve_unix,
};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use apxm_ais::permissions::{LayerDecisions, PermissionDecision, PermissionResolution};
use apxm_ais::{SLOT_CAPABILITY_REF, SemanticOpKind};
use apxm_compilation_protocol::{
    CompilationHandshake, CompilationRequest, CompilationResult, ProtocolError,
};
use apxm_program::air::AirModule;
use apxm_source_port::{
    Frontend, FrontendDrivers, FrontendRoots, PackageSnapshot, SnapshotError, SourceBundleRequest,
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
            last_idempotency: None,
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

    /// Persist committed AIR under `dir` so a Runtime child can load the digest.
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
        if let Some((key, prior)) = &self.last_idempotency
            && key == &idempotency_key
            && prior != &fingerprint
        {
            return Err(ProtocolError::ConflictingIdempotency);
        }
        self.last_idempotency = Some((idempotency_key, fingerprint));

        match compile_snapshot(&snapshot, &self.roots, &self.drivers) {
            Ok(air_json) => {
                let artifact_digest = format!("sha256:{:x}", Sha256::digest(air_json.as_bytes()));
                if let Some(dir) = &self.artifact_dir
                    && persist_artifact(dir, &artifact_digest, air_json.as_bytes()).is_err()
                {
                    return Ok(failed(&request_id, "artifact_persist"));
                }
                self.store.commit(artifact_digest.clone(), air_json);
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

/// File name for a digest in a shared artifact directory (`:` is not portable).
#[must_use]
pub fn artifact_file_name(digest: &str) -> String {
    digest.replace(':', "-")
}

fn persist_artifact(dir: &Path, digest: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    std::fs::write(dir.join(artifact_file_name(digest)), bytes).map_err(|error| error.to_string())
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
    let source =
        String::from_utf8(entry.bytes.clone()).map_err(|_| "entrypoint_not_utf8".to_owned())?;
    let program = authored_program_name(snapshot.frontend, &source)?;
    let compiled = compile_source_bundle(
        &SourceBundleRequest::new(snapshot.frontend, program, source),
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
                        continue;
                    };
                    let name = rest.split('(').next().unwrap_or("").trim();
                    if name.is_empty() {
                        break;
                    }
                    // A composition root may declare child Agent Programs in
                    // the same file. The last `@Agent` is the package entry.
                    found = Some(name.to_owned());
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
                found = Some(name);
            }
        }
    }
    found.ok_or_else(|| "missing_program".to_owned())
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
        let air = service.store().get(&artifact_digest).expect("AIR");
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
