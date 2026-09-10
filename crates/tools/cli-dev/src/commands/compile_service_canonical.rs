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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::tempdir;

    use crate::commands::agent::{AgentToml, agent_build};

    /// Spawns the package's Python entry, which imports the apxm_program frontend;
    /// it runs only where that package is installed on the subprocess path (as the
    /// packed-frontend gate arranges), not under the bare CLI test harness.
    #[ignore = "requires the apxm_program frontend installed on the subprocess path"]
    #[test]
    fn studio_style_source_package_builds_from_its_compile_entry_alone() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("studio-generated");
        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("agent.toml"),
            "id = \"studio-generated\"\n\
             version = \"0.1.0\"\n\
             schema_version = \"apxm.agent\"\n\n\
             [compile]\n\
             entry = \"python/main.py\"\n\
             frontend = \"python\"\n",
        )
        .unwrap();
        fs::write(
            root.join("python/main.py"),
            "from typing import TypedDict\n\
             \n\
             from apxm_program import Agent, Model\n\
             \n\
             \n\
             class StudioInput(TypedDict):\n\
             \x20\x20\x20\x20message: str\n\
             \n\
             \n\
             class StudioOutput(TypedDict):\n\
             \x20\x20\x20\x20message: str\n\
             \n\
             \n\
             StudioModel = Model[StudioInput, StudioOutput](\"model.target\")\n\
             \n\
             \n\
             @Agent(input=StudioInput, output=StudioOutput, model=StudioModel)\n\
             async def StudioGenerated(agent, request):\n\
             \x20\x20\x20\x20while request[\"message\"] != \"\":\n\
             \x20\x20\x20\x20\x20\x20\x20\x20reply = await StudioModel(request)\n\
             \x20\x20\x20\x20\x20\x20\x20\x20request = await agent.yield_(reply)\n\
             \x20\x20\x20\x20raise ValueError(\"missing studio input\")\n\n\
             if __name__ == \"__main__\":\n\
             \x20\x20\x20\x20print(StudioGenerated.canonical_air(), end=\"\")\n",
        )
        .unwrap();
        agent_build(&root, true).expect("a generated source package must build");
        let agent: AgentToml =
            toml::from_str(&fs::read_to_string(root.join("agent.toml")).unwrap()).unwrap();
        assert_eq!(
            agent
                .compile
                .as_ref()
                .and_then(|compile| compile.entry.as_deref()),
            Some("python/main.py")
        );

        let air = emit_canonical_air_from_agent(&root, None)
            .expect("canonical compile-service must compile the package-level program entry");
        assert!(air.contains("\"schema_version\":\"apxm.air\""));
        assert!(air.contains("\"op\":\"model.call\""));
    }
}
