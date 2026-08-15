//! Canonical package-to-AIR compile through the Compilation Service.

use std::path::{Path, PathBuf};

use anyhow::Result;
use apxm_compilation_client::CompilationClient;

/// Compile a canonical-authored agent package to canonical `apxm.air` JSON.
pub fn compile_service_canonical_command(
    agent_dir: PathBuf,
    config: Option<PathBuf>,
) -> Result<()> {
    let air_json = emit_canonical_air_from_agent(&agent_dir, config.as_deref())?;
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(air_json.as_bytes())?;
    Ok(())
}

/// Snapshot the package, compile through the Compilation Service, and return
/// the committed AIR JSON. The CLI never spawns `python --air` or `node`.
pub(crate) fn emit_canonical_air_from_agent(
    agent_dir: &Path,
    config_path: Option<&Path>,
) -> Result<String> {
    if !agent_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "'{}' is not a directory",
            agent_dir.display()
        ));
    }
    let _ = config_path;
    let mut client = CompilationClient::default();
    let digest = client
        .build_package(agent_dir)
        .map_err(|error| anyhow::anyhow!("Compilation Service refused the package: {error}"))?;
    client
        .artifact_bytes(&digest)
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow::anyhow!("Compilation Service committed {digest} but stored no AIR bytes")
        })
}
