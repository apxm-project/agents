//! Owner-local stdin adapter for the public source-bundle compiler port.

use std::io::{Read, Write};

use apxm_program::ExecutableArtifact;
use apxm_source_port::{
    FrontendDrivers, FrontendRoots, SourceBundleRequest, compile_source_bundle,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [python_package, typescript_package, python, node] = arguments.as_slice() else {
        return Err(
            "expected Python package, TypeScript package, Python driver, Node driver".into(),
        );
    };
    let mut input = String::new();
    std::io::stdin()
        .take((apxm_source_port::MAX_SOURCE_BYTES * 8) as u64)
        .read_to_string(&mut input)?;
    let request: SourceBundleRequest = serde_json::from_str(&input)?;
    let compiled = compile_source_bundle(
        &request,
        &FrontendRoots::new(
            std::fs::canonicalize(python_package)?,
            std::fs::canonicalize(typescript_package)?,
        ),
        &FrontendDrivers::new(std::fs::canonicalize(python)?, std::fs::canonicalize(node)?),
    )
    .map_err(|diagnostics| format!("{diagnostics:?}"))?;
    let artifact = ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)?;
    std::io::stdout().write_all(&artifact.encode()?)?;
    Ok(())
}
