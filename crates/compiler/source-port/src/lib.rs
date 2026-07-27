//! The Server-callable source-bundle compile port.
//!
//! This crate is the one library entry point that turns submitted authoring
//! source text into a typed FrontendGraph, canonical AIR, and a source map. A
//! caller binds it exactly as it binds `apxm_program::lower_frontend_graph_json`
//! and calls [`compile_source_bundle`]. The caller supplies the exact frontend
//! package roots and interpreter drivers at its composition boundary; this
//! crate reimplements no capture and discovers no checkout or driver path.
//!
//! # What the port owns
//!
//! The authoring frontends are genuinely Python and TypeScript, so capturing
//! typed intent from Python source runs the Python frontend and capturing it
//! from TypeScript source runs the TypeScript frontend. That interpreter
//! boundary, its confinement, its failure translation, and its unavailability
//! handling are this port's responsibility and are invisible above it. A caller
//! never names an interpreter, never sees a process, and never handles a panic:
//! an absent interpreter or an absent frontend package returns
//! [`SourceDiagnosticCode::FrontendUnavailable`].
//!
//! # Fail closed
//!
//! [`compile_source_bundle`] returns either a complete [`CompiledSource`] or a
//! non-empty list of diagnostics and no graph. Invalid syntax, a raw AIS
//! spelling, a drifted model reference, and an unresolved Tool reference each
//! reject with diagnostics and nothing else. There is no partial graph, no
//! best-effort graph, and no empty-but-valid graph.
//!
//! # The frontend never emits AIR
//!
//! A frontend records typed source intent; only the Rust lowering in
//! `apxm-program` selects AIS operations and produces AIR. This port enforces
//! that: it accepts exactly one FrontendGraph document from a frontend and
//! lowers it itself.

pub mod diagnostic;
mod frontend;

use std::path::{Path, PathBuf};

use apxm_program::air::AirModule;
use apxm_program::frontend_graph::FrontendGraph;
use apxm_program::source_map::SourceMap;

pub use crate::diagnostic::{SourceDiagnostic, SourceDiagnosticCode};
pub use crate::frontend::Frontend;

/// The largest submitted source text the port accepts. A larger submission is a
/// request rejection, not something handed to an interpreter.
pub const MAX_SOURCE_BYTES: usize = 1_048_576;

/// One submitted source bundle: which frontend authored it, which exported
/// definition is the program, and the source text itself.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBundleRequest {
    /// The authoring frontend that recorded this source.
    pub frontend: Frontend,
    /// The name of the authored Agent program inside the source text.
    pub entrypoint: String,
    /// The submitted authoring source text.
    pub source: String,
}

impl SourceBundleRequest {
    pub fn new(
        frontend: Frontend,
        entrypoint: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            frontend,
            entrypoint: entrypoint.into(),
            source: source.into(),
        }
    }

    fn validate(&self) -> Result<(), SourceDiagnostic> {
        if self.entrypoint.trim().is_empty() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::RequestInvalid,
                "a source bundle names the authored Agent program to capture",
            ));
        }
        if self.source.is_empty() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::RequestInvalid,
                "a source bundle carries authoring source text",
            ));
        }
        if self.source.len() > MAX_SOURCE_BYTES {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::RequestInvalid,
                format!(
                    "the submitted source is {} bytes; the port accepts at most {MAX_SOURCE_BYTES}",
                    self.source.len()
                ),
            ));
        }
        Ok(())
    }
}

/// Where each authoring frontend package is installed on this host. A caller
/// declares these roots once; nothing is discovered from an ambient module path
/// or checkout layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendRoots {
    python: PathBuf,
    typescript: PathBuf,
}

/// The exact interpreter driver for each authoring frontend.
///
/// Drivers are explicit composition data, alongside [`FrontendRoots`]. The
/// source port never searches `PATH`, substitutes an interpreter, or derives a
/// driver from a checkout. That keeps compiler composition outside submitted
/// source and makes a missing declared driver a closed unavailability result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendDrivers {
    python: PathBuf,
    typescript: PathBuf,
}

impl FrontendDrivers {
    /// Bind the exact Python and Node executables that may capture source.
    #[must_use]
    pub fn new(python: impl Into<PathBuf>, typescript: impl Into<PathBuf>) -> Self {
        Self {
            python: python.into(),
            typescript: typescript.into(),
        }
    }

    /// The declared interpreter driver for one selector.
    #[must_use]
    pub fn driver(&self, frontend: Frontend) -> &Path {
        match frontend {
            Frontend::Python => &self.python,
            Frontend::Typescript => &self.typescript,
        }
    }

    /// Verify that a driver is an exact usable file rather than a path subject
    /// to current-directory or `PATH` resolution.
    fn validate(&self, frontend: Frontend) -> Result<(), SourceDiagnostic> {
        let driver = self.driver(frontend);
        if !driver.is_absolute() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::FrontendUnavailable,
                format!(
                    "the declared {} authoring frontend driver must be an absolute path",
                    frontend.wire()
                ),
            ));
        }
        if !driver.is_file() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::FrontendUnavailable,
                format!(
                    "the declared {} authoring frontend driver '{}' is not a file",
                    frontend.wire(),
                    driver.display()
                ),
            ));
        }
        Ok(())
    }
}

impl FrontendRoots {
    pub fn new(python: impl Into<PathBuf>, typescript: impl Into<PathBuf>) -> Self {
        Self {
            python: python.into(),
            typescript: typescript.into(),
        }
    }

    /// The declared package root for one selector.
    #[must_use]
    pub fn root(&self, frontend: Frontend) -> &Path {
        match frontend {
            Frontend::Python => &self.python,
            Frontend::Typescript => &self.typescript,
        }
    }

    /// Verify that a package root is exact composition data rather than a path
    /// resolved through the process working directory.
    fn validate(&self, frontend: Frontend) -> Result<(), SourceDiagnostic> {
        let root = self.root(frontend);
        if !root.is_absolute() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::FrontendUnavailable,
                format!(
                    "the declared {} authoring frontend root must be an absolute path",
                    frontend.wire()
                ),
            ));
        }
        if !root.is_dir() {
            return Err(SourceDiagnostic::new(
                SourceDiagnosticCode::FrontendUnavailable,
                format!(
                    "the declared {} authoring frontend root '{}' is not a directory",
                    frontend.wire(),
                    root.display()
                ),
            ));
        }
        Ok(())
    }
}

/// One successfully compiled source bundle. Every field is complete: the port
/// never returns this value with a graph it could not lower.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSource {
    /// The typed FrontendGraph the authoring frontend recorded.
    pub frontend_graph: FrontendGraph,
    /// Canonical AIR, lowered from the graph by Rust alone.
    pub air: AirModule,
    /// The source map carried by the lowered AIR.
    pub source_map: SourceMap,
}

/// Compile one submitted source bundle into a typed FrontendGraph, canonical
/// AIR, and a source map.
///
/// # Errors
///
/// Returns a non-empty, deterministically ordered list of closed diagnostics and
/// no graph. Every rejection class — an unavailable frontend, invalid syntax, a
/// raw AIS spelling, a drifted model reference, an unresolved Tool reference, a
/// graph that does not verify or lower — arrives here.
pub fn compile_source_bundle(
    request: &SourceBundleRequest,
    roots: &FrontendRoots,
    drivers: &FrontendDrivers,
) -> Result<CompiledSource, Vec<SourceDiagnostic>> {
    request.validate().map_err(|diagnostic| vec![diagnostic])?;
    roots
        .validate(request.frontend)
        .map_err(|diagnostic| vec![diagnostic])?;
    drivers
        .validate(request.frontend)
        .map_err(|diagnostic| vec![diagnostic])?;

    let captured = frontend::capture(
        request.frontend,
        roots.root(request.frontend),
        drivers.driver(request.frontend),
        &request.entrypoint,
        &request.source,
    )
    .map_err(|diagnostic| vec![diagnostic])?;

    let frontend_graph: FrontendGraph =
        serde_json::from_value(captured).map_err(|error| {
            vec![SourceDiagnostic::new(
                SourceDiagnosticCode::FrontendOutputInvalid,
                format!(
                    "the {} authoring frontend recorded a value outside apxm.frontend-graph.v1: {error}",
                    request.frontend.wire()
                ),
            )]
        })?;

    // A frontend records the source language it authored. A graph that claims a
    // different one is not the graph this request asked for.
    if frontend_graph.source_language != request.frontend.source_language() {
        return Err(vec![SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            format!(
                "the {} authoring frontend recorded source_language '{}'",
                request.frontend.wire(),
                frontend_graph.source_language.wire()
            ),
        )]);
    }

    // Rust alone selects AIS operations and produces AIR. The frontend handed
    // over typed source intent; this is where AIR comes into existence.
    let air = apxm_program::frontend_graph_to_air(&frontend_graph).map_err(|verdict| {
        verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| {
                SourceDiagnostic::new(
                    SourceDiagnosticCode::GraphRejected,
                    format!(
                        "{}:{}: {}",
                        diagnostic.code.slug(),
                        diagnostic.location,
                        diagnostic.message
                    ),
                )
            })
            .collect::<Vec<_>>()
    })?;

    let source_map = air.source_map.clone();
    Ok(CompiledSource {
        frontend_graph,
        air,
        source_map,
    })
}
