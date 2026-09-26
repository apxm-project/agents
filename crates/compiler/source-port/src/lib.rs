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
//! [`SourceDiagnosticCode::FrontendUnavailable`], and so does a host whose
//! kernel cannot provide the capture boundary described in
//! [`confinement`].
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

pub mod confinement;
pub mod diagnostic;
mod frontend;
pub mod package_snapshot;

use std::path::{Path, PathBuf};

use apxm_core::{grammar, types::host_capability};
use apxm_program::air::AirModule;
use apxm_program::frontend_graph::FrontendGraph;
use apxm_program::source_map::SourceMap;

pub use crate::confinement::{
    CAPTURE_SCRATCH_DIR_VARIABLE, CONFINEMENT_BOUNDARY, CONFINEMENT_MODE_VARIABLE, ConfinementMode,
    ConfinementReadiness, ConfinementStatus, capture_confinement_readiness,
};
pub use crate::diagnostic::{
    COMPILE_DIAGNOSTICS_SCHEMA, CompileDiagnostic, DiagnosticReport, DiagnosticReportVersion,
    Location, MAX_FIELD_PATH, MAX_MESSAGE_BYTES, MAX_RELATED, MAX_REPORT_ITEMS, Phase, Related,
    Severity, SourceDiagnostic, SourceDiagnosticCode, TypeFact, bound_message,
};
pub use crate::frontend::Frontend;
pub use crate::package_snapshot::{
    PACKAGE_SNAPSHOT_CONTRACT, PackageSnapshot, SnapshotContent, SnapshotError, content_digest,
    snapshot_identity_digest,
};

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
    /// The host capability ids the package declares, without the reserved
    /// `host:` prefix. The minted Capability set the frontend compiles against
    /// is the builtin catalogue united with these; a `host:` reference to
    /// anything else is a compile error naming the line and column of the
    /// reference. A caller that compiles source outside a package declares
    /// none, and every `host:` reference then rejects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<String>,
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
            host_capabilities: Vec::new(),
        }
    }

    /// Declare the host capability ids this package mints.
    #[must_use]
    pub fn with_host_capabilities(
        mut self,
        ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.host_capabilities = ids.into_iter().map(Into::into).collect();
        self
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
                    "the declared {} authoring frontend driver is not a file",
                    frontend.wire(),
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
                    "the declared {} authoring frontend root is not a directory",
                    frontend.wire(),
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
    /// Digest of the canonical AIR bytes used as the artifact identity input.
    pub air_digest: String,
    /// Opaque lineage commitment for this source/AIR/compiler combination.
    pub execution_lineage_ref: String,
    /// Non-error diagnostics raised while compiling, in emission order. A
    /// compiled source never carries an error.
    pub diagnostics: Vec<SourceDiagnostic>,
}

/// Bound a list of source-port diagnostics into one wire report.
///
/// `stopped_at` is the latest phase any error was raised in: the port stops at
/// the first failing phase, so no check after it ran. A list without an error
/// ran every phase the port owns, and the report says nothing stopped.
#[must_use]
pub fn diagnostic_report(diagnostics: &[SourceDiagnostic]) -> DiagnosticReport {
    let stopped_at = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.phase)
        .max();
    DiagnosticReport::from_diagnostics(
        diagnostics
            .iter()
            .map(SourceDiagnostic::to_compile_diagnostic),
        stopped_at,
    )
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

    reject_undeclared_host_references(request)?;

    let captured = frontend::capture(
        request.frontend,
        roots.root(request.frontend),
        drivers.driver(request.frontend),
        &request.entrypoint,
        &request.source,
        &request.host_capabilities,
    )?;

    let source_digest = content_digest(request.source.as_bytes());
    let mut compiled =
        compile_captured_graph(request.frontend, captured.frontend_graph, &source_digest)?;
    compiled.diagnostics = captured.diagnostics;
    Ok(compiled)
}

/// Refuse a `host:` reference the package's manifest does not declare, naming
/// where in the submitted source it is written.
///
/// The frontends close the same set inside the interpreter and refuse the same
/// reference, but an interpreter cannot say *where*: its markers run at module
/// evaluation, with no source node to point at. This runs first and over the
/// exact submitted text, so an author gets the location of every undeclared
/// reference rather than only its spelling. A capability reference is always a
/// literal — both frontends refuse a computed one — so scanning quoted
/// occurrences of the reserved prefix finds every reference a program can make.
///
/// Each occurrence is its own diagnostic with its own location, in source
/// order. The scan is a static check of the submitted text before any
/// evaluation, so it reports in the type-check phase: nothing later ran.
fn reject_undeclared_host_references(
    request: &SourceBundleRequest,
) -> Result<(), Vec<SourceDiagnostic>> {
    let declared = request
        .host_capabilities
        .iter()
        .map(|id| host_capability::host_capability_ref(id))
        .collect::<std::collections::BTreeSet<_>>();
    let source_file = request.frontend.submitted_source_file();
    let diagnostics = quoted_host_references(&request.source)
        .into_iter()
        .filter(|reference| !declared.contains(&reference.text))
        .map(|reference| {
            let span = apxm_program::source_map::Span {
                start_line: reference.line,
                start_column: reference.column,
                end_line: reference.line,
                end_column: reference.column + reference.width,
            };
            SourceDiagnostic::new(
                SourceDiagnosticCode::GraphRejected,
                format!(
                    "the Capability reference '{}' names the host-fulfilled namespace, and \
                     the package manifest declares no matching [[capabilities.host]] entry",
                    reference.text
                ),
            )
            .with_phase(Phase::TypeCheck)
            .with_location(Location {
                source_file: source_file.to_owned(),
                span,
            })
        })
        .collect::<Vec<_>>();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// One quoted `host:` reference in submitted source, in source-map
/// coordinates: a 1-based line, a 0-based character column, and the width of
/// the reference in characters.
#[derive(Debug, PartialEq, Eq)]
struct HostReference {
    line: u32,
    column: u32,
    width: u32,
    text: String,
}

/// Every quoted `host:` reference in `source`, in source order.
///
/// Only an occurrence immediately behind a quote is a reference: the prefix in
/// prose or in a comment is text about the namespace, not a use of it. Columns
/// count characters rather than bytes, so a reference after a non-ASCII
/// character lands where an editor puts it.
fn quoted_host_references(source: &str) -> Vec<HostReference> {
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        for (offset, _) in line.match_indices(host_capability::HOST_CAPABILITY_REF_PREFIX) {
            if offset == 0 || !matches!(bytes[offset - 1], b'"' | b'\'' | b'`') {
                continue;
            }
            let tail = &line[offset..];
            let end = tail
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric()
                        || matches!(character, '.' | '_' | '-' | ':'))
                })
                .unwrap_or(tail.len());
            let text = tail[..end].to_owned();
            found.push(HostReference {
                line: u32::try_from(index + 1).unwrap_or(u32::MAX),
                column: u32::try_from(line[..offset].chars().count()).unwrap_or(u32::MAX),
                width: u32::try_from(text.chars().count()).unwrap_or(u32::MAX),
                text,
            });
        }
    }
    found
}

/// Project one verifier or lowering diagnostic, anchoring its location.
///
/// A verifier location is either the identity of a graph entity or a field
/// path. A semantic or control node becomes `node_id`, with the span the
/// frontend recorded for it; any other graph entity becomes a field path into
/// the collection that holds it; anything else is a dotted field path.
fn lowering_diagnostic(
    graph: &FrontendGraph,
    diagnostic: apxm_program::diagnostic::Diagnostic,
) -> SourceDiagnostic {
    let location = diagnostic.location.trim().to_owned();
    let mut projected =
        SourceDiagnostic::new(SourceDiagnosticCode::GraphRejected, diagnostic.message)
            .with_detail_code(diagnostic.code.slug())
            .with_phase(Phase::Lowering);
    if location.is_empty() {
        return projected;
    }
    let is_node = graph
        .call_intents
        .iter()
        .any(|intent| intent.node_id == location)
        || graph
            .control_intents
            .iter()
            .any(|intent| intent.node_id == location);
    if is_node {
        if let Some(span) = graph
            .source_map
            .node_spans
            .iter()
            .find(|span| span.node_id == location)
        {
            projected = projected.with_location(Location {
                source_file: span.source_file.clone(),
                span: span.span,
            });
        }
        // A rejected graph may contain an authored name that is not an
        // identifier. Keep its source span, but never emit that name as a
        // diagnostic node identity on the strict compile protocol.
        return if grammar::is_identifier(&location) {
            projected.with_node_id(location)
        } else {
            projected
        };
    }
    if let Some(span) = graph
        .source_map
        .region_spans
        .iter()
        .find(|span| span.region_id == location)
    {
        projected = projected.with_location(Location {
            source_file: span.source_file.clone(),
            span: span.span,
        });
    }
    let collection = if graph
        .declarations
        .iter()
        .any(|declaration| declaration.decl_id == location)
    {
        Some("declarations")
    } else if graph.values.iter().any(|value| value.value_id == location) {
        Some("values")
    } else if graph
        .regions
        .iter()
        .any(|region| region.region_id == location)
    {
        Some("regions")
    } else if graph
        .hook_bindings
        .iter()
        .any(|binding| binding.hook_id == location)
    {
        Some("hook_bindings")
    } else if graph
        .program_definitions
        .iter()
        .any(|definition| definition.program_id == location)
    {
        Some("program_definitions")
    } else {
        None
    };
    let field_path = match collection {
        Some(collection) => vec![collection.to_owned(), location],
        None => location
            .split('.')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect(),
    };
    projected.with_field_path(field_path)
}

fn compile_captured_graph(
    frontend: Frontend,
    captured: serde_json::Value,
    source_digest: &str,
) -> Result<CompiledSource, Vec<SourceDiagnostic>> {
    let frontend_graph: FrontendGraph = serde_json::from_value(captured).map_err(|error| {
        vec![SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            format!(
                "the {} authoring frontend recorded a value outside apxm.frontend-graph: {error}",
                frontend.wire()
            ),
        )]
    })?;

    // A frontend records the source language it authored. A graph that claims a
    // different one is not the graph this request asked for.
    if frontend_graph.source_language != frontend.source_language() {
        return Err(vec![SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            format!(
                "the {} authoring frontend recorded source_language '{}'",
                frontend.wire(),
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
            .map(|diagnostic| lowering_diagnostic(&frontend_graph, diagnostic))
            .collect::<Vec<_>>()
    })?;

    let source_map = air.source_map.clone();
    let air_bytes = serde_json::to_vec(&air).map_err(|error| {
        vec![SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            format!("canonical AIR cannot be serialized: {error}"),
        )]
    })?;
    let air_digest = format!("sha256:{}", content_digest(&air_bytes));
    let execution_lineage_ref = apxm_program::execution_lineage_ref(
        source_digest,
        &air_digest,
        apxm_program::EXECUTION_LINEAGE_COMPILER_IDENTITY,
    );
    if execution_lineage_ref.trim().is_empty() {
        return Err(vec![SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            "canonical compilation did not produce an execution lineage reference",
        )]);
    }
    Ok(CompiledSource {
        frontend_graph,
        air,
        source_map,
        air_digest,
        execution_lineage_ref,
        diagnostics: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::Deserialize;
    use serde_json::Value;

    use super::{
        Frontend, Phase, SourceDiagnosticCode, compile_captured_graph, quoted_host_references,
    };

    #[derive(Debug, Deserialize)]
    struct FrontendGraphVector {
        name: String,
        input: Value,
        expected_valid: bool,
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("the source-port crate sits three levels under the repository root")
            .to_path_buf()
    }

    fn frontend_graph_vector(name: &str) -> FrontendGraphVector {
        let vectors: Vec<FrontendGraphVector> = serde_json::from_str(
            &fs::read_to_string(
                repository_root().join("contracts/vectors/apxm.frontend-graph.json"),
            )
            .expect("read checked-in frontend-graph vectors"),
        )
        .expect("decode checked-in frontend-graph vectors");
        vectors
            .into_iter()
            .find(|vector| vector.name == name)
            .unwrap_or_else(|| panic!("frontend-graph vector '{name}' is checked in"))
    }

    fn requested_frontend(vector: &FrontendGraphVector) -> Frontend {
        match vector.input["source_language"]
            .as_str()
            .expect("vector source_language is a string")
        {
            "python" => Frontend::Python,
            "typescript" => Frontend::Typescript,
            other => panic!("unexpected vector source_language '{other}'"),
        }
    }

    #[test]
    fn valid_frontend_graph_vector_compiles_through_the_source_port_boundary() {
        let vector = frontend_graph_vector("valid-frontend-graph-typed-intents");
        assert!(
            vector.expected_valid,
            "the checked-in positive vector stays positive"
        );
        let frontend = requested_frontend(&vector);

        let compiled = compile_captured_graph(frontend, vector.input, "vector-source")
            .expect("the checked-in positive vector compiles through source-port lowering");

        assert_eq!(
            compiled.frontend_graph.source_language,
            frontend.source_language(),
            "the compiled graph preserves the requested source language"
        );
        assert_eq!(
            compiled.source_map, compiled.air.source_map,
            "the source port returns the AIR source map it lowered"
        );
        assert!(
            !compiled.air.semantic_operations.is_empty(),
            "the checked-in positive vector lowers to executable AIR"
        );
    }

    #[test]
    fn incompatible_frontend_graph_schema_version_is_invalid_frontend_output() {
        let vector = frontend_graph_vector("incompatible-frontend-graph-schema-version-rejected");
        assert!(
            !vector.expected_valid,
            "the checked-in negative vector stays negative"
        );
        let diagnostics =
            compile_captured_graph(requested_frontend(&vector), vector.input, "vector-source")
                .expect_err("an incompatible frontend-graph version is rejected");

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendOutputInvalid]
        );
    }

    #[test]
    fn unknown_frontend_graph_field_is_invalid_frontend_output() {
        let vector = frontend_graph_vector("unknown-frontend-graph-field-rejected");
        assert!(
            !vector.expected_valid,
            "the checked-in negative vector stays negative"
        );
        let diagnostics =
            compile_captured_graph(requested_frontend(&vector), vector.input, "vector-source")
                .expect_err("a captured graph with an unknown field is rejected");

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendOutputInvalid]
        );
    }

    #[test]
    fn unknown_semantic_discriminant_is_invalid_frontend_output() {
        let vector = frontend_graph_vector("ais-kind-in-graph-intent-rejected");
        assert!(
            !vector.expected_valid,
            "the checked-in negative vector stays negative"
        );
        let diagnostics =
            compile_captured_graph(requested_frontend(&vector), vector.input, "vector-source")
                .expect_err(
                    "a captured graph with an AIS discriminant in source intent is rejected",
                );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendOutputInvalid]
        );
    }

    /// A verifier diagnostic anchored at a graph node keeps every item, names
    /// the node, carries the span the frontend recorded for it, and keeps the
    /// verifier's own closed code rather than flattening it into text.
    #[test]
    fn verifier_diagnostics_are_structured_and_all_kept() {
        let vector = frontend_graph_vector("valid-frontend-graph-typed-intents");
        let frontend = requested_frontend(&vector);
        let mut graph = vector.input;
        let first = graph["call_intents"][0].clone();
        let second = graph["call_intents"][1].clone();
        let node = first["node_id"].as_str().unwrap().to_owned();
        let other = second["node_id"].as_str().unwrap().to_owned();
        graph["call_intents"].as_array_mut().unwrap().push(first);
        graph["call_intents"].as_array_mut().unwrap().push(second);
        graph["source_map"]["node_spans"] = serde_json::json!([{
            "node_id": node,
            "source_file": "submitted_source.py",
            "span": {"start_line": 4, "start_column": 2, "end_line": 4, "end_column": 9}
        }]);

        let diagnostics = compile_captured_graph(frontend, graph, "vector-source")
            .expect_err("duplicate node ids do not verify");

        let duplicates = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.wire_code() == "duplicate_node_id")
            .collect::<Vec<_>>();
        assert_eq!(
            duplicates.len(),
            2,
            "every diagnostic is kept: {diagnostics:?}"
        );
        for diagnostic in &diagnostics {
            assert_eq!(diagnostic.code, SourceDiagnosticCode::GraphRejected);
            assert_eq!(diagnostic.phase, Phase::Lowering);
            assert!(!diagnostic.message.contains("duplicate_node_id:"));
        }
        let anchored = duplicates
            .iter()
            .find(|diagnostic| diagnostic.node_id.as_deref() == Some(node.as_str()))
            .expect("the duplicate names its node");
        let location = anchored
            .location
            .as_ref()
            .expect("the node's span is carried");
        assert_eq!(location.source_file, "submitted_source.py");
        assert_eq!(location.span.start_line, 4);
        assert!(anchored.field_path.is_none());
        let unspanned = duplicates
            .iter()
            .find(|diagnostic| diagnostic.node_id.as_deref() == Some(other.as_str()))
            .expect("the second duplicate names its node");
        assert!(unspanned.location.is_none());
    }

    /// A reference behind a quote is found in source-map coordinates: a
    /// 1-based line and a 0-based character column, past non-ASCII text.
    #[test]
    fn quoted_host_references_use_source_map_coordinates() {
        let found = quoted_host_references("x = 1\n\u{e9}\u{e9} = \"host:a.b\" # host:c\n'host:d'");
        let coordinates = found
            .iter()
            .map(|reference| {
                (
                    reference.line,
                    reference.column,
                    reference.width,
                    reference.text.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            coordinates,
            vec![(2, 6, 8, "host:a.b"), (3, 1, 6, "host:d")]
        );
    }
}
