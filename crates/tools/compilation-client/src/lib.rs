//! Compilation Client: snapshot, submit, render diagnostics, return artifact refs.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use apxm_compilation_protocol::{
    COMPILATION_PROTOCOL_VERSION, CompilationHandshake, CompilationRequest, CompilationResult,
};
use apxm_compilation_service::{CompilationService, StdioFrame, decode_jsonl, encode_jsonl};
use apxm_source_port::{Frontend, PackageSnapshot, SnapshotContent};
use serde::Deserialize;

/// Headless build client. Contains no frontend or compiler implementation.
pub struct CompilationClient {
    inner: CompilationInner,
}

enum CompilationInner {
    InProcess(CompilationService),
    Stdio(StdioCompilation),
}

struct StdioCompilation {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    artifact_dir: PathBuf,
    last_bytes: Option<(String, String)>,
}

impl Default for CompilationClient {
    fn default() -> Self {
        Self {
            inner: CompilationInner::InProcess(CompilationService::default()),
        }
    }
}

impl Drop for CompilationClient {
    fn drop(&mut self) {
        if let CompilationInner::Stdio(stdio) = &mut self.inner {
            let _ = stdio.child.kill();
            let _ = stdio.child.wait();
        }
    }
}

impl CompilationClient {
    /// Speak JSONL to a Compilation Service child. The parent does not construct
    /// the service handler.
    pub fn spawn_stdio(
        program: impl AsRef<Path>,
        args: &[&str],
        artifact_dir: impl AsRef<Path>,
    ) -> Result<Self, String> {
        let artifact_dir = artifact_dir.as_ref().to_path_buf();
        fs::create_dir_all(&artifact_dir).map_err(|error| error.to_string())?;
        let mut child = Command::new(program.as_ref())
            .args(args)
            .env("APXM_ARTIFACT_DIR", &artifact_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| error.to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "compilation stdin".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "compilation stdout".to_owned())?;
        Ok(Self {
            inner: CompilationInner::Stdio(StdioCompilation {
                child,
                stdin,
                stdout: BufReader::new(stdout),
                artifact_dir,
                last_bytes: None,
            }),
        })
    }

    /// Snapshot a local package directory and compile it. The CLI never
    /// constructs a `PackageSnapshot` itself.
    pub fn build_package(&mut self, package_root: &Path) -> Result<String, String> {
        self.build(snapshot_package(package_root)?)
    }

    /// Submit one exact snapshot. Failed or uncertain compiles return no digest.
    pub fn build(&mut self, snapshot: PackageSnapshot) -> Result<String, String> {
        let result = self.request(CompilationRequest::Compile {
            request_id: "build".to_owned(),
            idempotency_key: snapshot.snapshot_digest.clone(),
            snapshot,
        })?;
        match result {
            CompilationResult::ArtifactCommitted {
                artifact_digest, ..
            } => Ok(artifact_digest),
            CompilationResult::Failed { code, .. } => Err(code),
            CompilationResult::Cancelled { .. } => Err("cancelled".to_owned()),
        }
    }

    /// Committed artifact bytes for a digest this client produced.
    #[must_use]
    pub fn artifact_bytes(&self, digest: &str) -> Option<&str> {
        match &self.inner {
            CompilationInner::InProcess(service) => service.store().get(digest),
            CompilationInner::Stdio(stdio) => stdio
                .last_bytes
                .as_ref()
                .and_then(|(id, bytes)| (id == digest).then_some(bytes.as_str())),
        }
    }

    fn request(&mut self, request: CompilationRequest) -> Result<CompilationResult, String> {
        match &mut self.inner {
            CompilationInner::InProcess(service) => service
                .handle(
                    &CompilationHandshake {
                        protocol_version: COMPILATION_PROTOCOL_VERSION.to_owned(),
                    },
                    request,
                )
                .map_err(|error| format!("{error:?}")),
            CompilationInner::Stdio(stdio) => {
                let envelope = serde_json::json!({
                    "handshake": {
                        "protocol_version": COMPILATION_PROTOCOL_VERSION,
                    },
                    "request": request,
                });
                let frame = StdioFrame {
                    channel: "compilation".to_owned(),
                    payload: envelope.to_string(),
                };
                stdio
                    .stdin
                    .write_all(encode_jsonl(&frame).as_bytes())
                    .map_err(|error| error.to_string())?;
                stdio.stdin.flush().map_err(|error| error.to_string())?;
                let mut line = String::new();
                stdio
                    .stdout
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                let reply = decode_jsonl(&line)?;
                let result: CompilationResult =
                    serde_json::from_str(&reply.payload).map_err(|error| error.to_string())?;
                if let CompilationResult::ArtifactCommitted {
                    ref artifact_digest,
                    ..
                } = result
                {
                    if let Ok(bytes) = fs::read_to_string(
                        stdio.artifact_dir.join(artifact_digest.replace(':', "-")),
                    ) {
                        stdio.last_bytes = Some((artifact_digest.clone(), bytes));
                    }
                }
                Ok(result)
            }
        }
    }
}

/// Walk a package root and bind every file's bytes into a validated snapshot.
pub fn snapshot_package(package_root: &Path) -> Result<PackageSnapshot, String> {
    if !package_root.is_dir() {
        return Err(format!("'{}' is not a directory", package_root.display()));
    }
    let manifest = read_manifest(package_root)?;
    let mut contents = Vec::new();
    collect_files(package_root, package_root, &mut contents)?;
    if contents.is_empty() {
        return Err("package snapshot is empty".to_owned());
    }
    let lock_digest = contents
        .iter()
        .find(|content| is_lock_name(&content.path))
        .map(|content| content.digest.clone());
    PackageSnapshot::assemble(
        manifest.frontend,
        manifest.entry,
        contents,
        lock_digest,
        "apxm.compatibility-set/local",
    )
    .map_err(|error| error.to_string())
}

#[derive(Debug, Deserialize)]
struct AgentToml {
    compile: Option<CompileToml>,
}

#[derive(Debug, Deserialize)]
struct CompileToml {
    entry: Option<String>,
    frontend: Option<Frontend>,
}

struct DeclaredCompile {
    frontend: Frontend,
    entry: String,
}

fn read_manifest(package_root: &Path) -> Result<DeclaredCompile, String> {
    let path = package_root.join("agent.toml");
    let text = fs::read_to_string(&path).map_err(|_| "missing_frontend".to_owned())?;
    let parsed: AgentToml = toml::from_str(&text).map_err(|error| error.to_string())?;
    let compile = parsed
        .compile
        .ok_or_else(|| "missing_frontend".to_owned())?;
    let frontend = compile
        .frontend
        .ok_or_else(|| "missing_frontend".to_owned())?;
    let entry = compile
        .entry
        .ok_or_else(|| "missing_entrypoint".to_owned())?;
    Ok(DeclaredCompile { frontend, entry })
}

fn collect_files(
    root: &Path,
    current: &Path,
    contents: &mut Vec<SnapshotContent>,
) -> Result<(), String> {
    let entries = fs::read_dir(current).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if should_skip(&name) {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, contents)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        contents.push(SnapshotContent::from_bytes(relative, bytes));
    }
    Ok(())
}

fn should_skip(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | "__pycache__" | ".apxm" | ".dekk" | ".DS_Store"
    ) || name.ends_with(".pyc")
}

fn is_lock_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "uv.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "Cargo.lock"
            | "poetry.lock"
    ) || name.ends_with(".lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

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

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("compilation-client sits three levels under the repository root")
            .to_path_buf()
    }

    fn frontend_present() -> bool {
        let root = workspace_root();
        root.join(".dekk/env/bin/python").is_file()
            && root
                .join("crates/compiler/frontend/python/apxm_program/_native.so")
                .is_file()
    }

    #[test]
    fn build_package_snapshots_real_bytes_and_commits_air() {
        if !frontend_present() {
            return;
        }
        let dir = tempfile_dir();
        fs::write(
            dir.join("agent.toml"),
            "id = \"tiny\"\nversion = \"0.1.0\"\nschema_version = \"apxm.agent\"\n\n[compile]\nentry = \"src/agent.py\"\nfrontend = \"python\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/agent.py"), PYTHON_PROGRAM).unwrap();

        let snapshot = snapshot_package(&dir).unwrap();
        assert!(
            snapshot
                .file("src/agent.py")
                .is_some_and(|content| !content.bytes.is_empty())
        );
        assert_ne!(snapshot.snapshot_digest, "local");
        assert!(snapshot.file("src/agent.py").unwrap().digest != "local");

        let mut client = CompilationClient::default();
        let digest = client.build_package(&dir).unwrap();
        assert!(digest.starts_with("sha256:"));
        let air = client.artifact_bytes(&digest).expect("committed bytes");
        assert!(air.contains("apxm.air"));
    }

    #[test]
    fn missing_frontend_is_rejected_before_compile() {
        let dir = tempfile_dir();
        fs::write(dir.join("agent.toml"), "id = \"x\"\nversion = \"0.1.0\"\n").unwrap();
        fs::write(dir.join("src.py"), "print('x')").unwrap();
        assert_eq!(snapshot_package(&dir).unwrap_err(), "missing_frontend");
    }

    fn tempfile_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "apxm-compilation-client-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
