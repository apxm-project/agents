//! The closed rejection vocabulary of the source-bundle compile port.
//!
//! A caller binds this crate to compile submitted source text; it never learns
//! that an interpreter ran. Every failure — an absent interpreter, an absent
//! frontend package, a syntax error, a raw AIS spelling, a drifted model
//! reference, an unresolved Tool reference, a graph that does not lower — is one
//! member of [`SourceDiagnosticCode`]. There is no open variant and no free-form
//! error channel.

use std::fmt;

use apxm_program::source_map::Span;
use serde::{Deserialize, Serialize};

/// The closed set of source-port rejection reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceDiagnosticCode {
    /// The submitted request was not a compilable unit: an empty entrypoint, an
    /// empty source text, or a source text past the accepted size.
    RequestInvalid,
    /// The authoring frontend for the selected language cannot run on this host:
    /// no interpreter, no frontend package, or no compiler bridge inside it.
    FrontendUnavailable,
    /// The submitted source text did not capture. Invalid syntax, a raw AIS
    /// spelling, a drifted model reference, an unresolved Tool reference, and
    /// every other authoring rejection carry this code.
    SourceRejected,
    /// The declared entrypoint is absent from the submitted source, or is not an
    /// authored Agent program.
    EntrypointNotAnAgentProgram,
    /// The frontend produced something other than exactly one FrontendGraph
    /// document: unparseable output, injected bytes, or an out-of-shape value.
    FrontendOutputInvalid,
    /// The captured FrontendGraph did not verify or did not lower to canonical
    /// AIR.
    GraphRejected,
}

impl SourceDiagnosticCode {
    /// The stable, machine-readable slug for this reason.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::RequestInvalid => "request_invalid",
            Self::FrontendUnavailable => "frontend_unavailable",
            Self::SourceRejected => "source_rejected",
            Self::EntrypointNotAnAgentProgram => "entrypoint_not_an_agent_program",
            Self::FrontendOutputInvalid => "frontend_output_invalid",
            Self::GraphRejected => "graph_rejected",
        }
    }

    /// Decode the closed reason token of a capture harness's rejection
    /// record. An unrecognized token is not trusted as a reason: the port
    /// treats the harness output as invalid rather than inventing a code.
    pub(crate) fn from_harness_token(token: &str) -> Option<Self> {
        match token {
            "harness_request_invalid" => Some(Self::RequestInvalid),
            "frontend_unavailable" => Some(Self::FrontendUnavailable),
            "source_rejected" => Some(Self::SourceRejected),
            "entrypoint_not_an_agent_program" => Some(Self::EntrypointNotAnAgentProgram),
            _ => None,
        }
    }
}

impl fmt::Display for SourceDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

/// The versioned identity of a compile diagnostic report.
pub const COMPILE_DIAGNOSTICS_SCHEMA: &str = "apxm.compile-diagnostics.v1";
/// The most diagnostics one report carries; the rest are counted, not sent.
pub const MAX_REPORT_ITEMS: usize = 64;
/// The largest message one diagnostic carries, in UTF-8 bytes.
pub const MAX_MESSAGE_BYTES: usize = 1024;
/// The deepest field path one diagnostic carries.
pub const MAX_FIELD_PATH: usize = 16;
/// The most related notes one diagnostic carries.
pub const MAX_RELATED: usize = 8;

/// How strongly a diagnostic bears on the compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The compile cannot commit an artifact.
    Error,
    /// The compile may commit; the author should look.
    Warning,
    /// Informational only.
    Info,
}

/// The compile phase that produced a diagnostic, in pipeline order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The request envelope itself.
    Request,
    /// The submitted package: manifest, snapshot, integrity.
    Package,
    /// Starting and running the authoring frontend.
    FrontendCapture,
    /// Type checking of the submitted source.
    TypeCheck,
    /// Capturing typed intent from the checked source.
    Capture,
    /// Verifying and lowering the captured graph to AIR.
    Lowering,
    /// Admitting the lowered program against the package's grants.
    Admission,
}

/// A source location in the same coordinates as the executable artifact's
/// source map: 1-based lines, 0-based columns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    /// The portable source file name, never a host filesystem path.
    pub source_file: String,
    /// The span, in the source-map convention.
    pub span: Span,
}

impl Location {
    /// A location at one point: `line` 1-based, `column` 0-based.
    #[must_use]
    pub fn point(source_file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            source_file: source_file.into(),
            span: Span {
                start_line: line,
                start_column: column,
                end_line: line,
                end_column: column,
            },
        }
    }
}

/// A secondary note attached to a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Related {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

/// A type fact a diagnostic compares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeFact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_digest: Option<String>,
}

/// One diagnostic on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileDiagnostic {
    pub severity: Severity,
    /// A closed, machine-readable slug.
    pub code: String,
    pub phase: Phase,
    /// Human-readable; bounded, never a host path.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_path: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<Related>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<TypeFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<TypeFact>,
}

impl CompileDiagnostic {
    /// A diagnostic with only its required fields.
    #[must_use]
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        phase: Phase,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            phase,
            message: bound_message(message.into()),
            location: None,
            node_id: None,
            field_path: None,
            related: None,
            expected: None,
            actual: None,
        }
    }

    /// Enforce the wire bounds: message size, field-path depth, related count.
    #[must_use]
    pub fn bounded(mut self) -> Self {
        self.message = bound_message(std::mem::take(&mut self.message));
        if let Some(path) = &mut self.field_path {
            path.truncate(MAX_FIELD_PATH);
        }
        if let Some(related) = &mut self.related {
            related.truncate(MAX_RELATED);
            for note in related.iter_mut() {
                note.message = bound_message(std::mem::take(&mut note.message));
            }
        }
        self
    }
}

/// The versioned identity of a [`DiagnosticReport`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticReportVersion {
    #[serde(rename = "apxm.compile-diagnostics.v1")]
    V1,
}

/// A bounded, ordered report of every diagnostic one compile emitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    pub schema_version: DiagnosticReportVersion,
    /// At most [`MAX_REPORT_ITEMS`], in emission order.
    pub items: Vec<CompileDiagnostic>,
    /// Whether diagnostics beyond `items` were emitted and dropped.
    pub truncated: bool,
    /// How many diagnostics were emitted in total.
    pub total_count: u32,
    /// The phase after which no further checks ran. Absent when every phase
    /// ran; a consumer never reads a later phase as verified when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<Phase>,
}

impl Default for DiagnosticReport {
    fn default() -> Self {
        Self::empty()
    }
}

impl DiagnosticReport {
    /// A report with no diagnostics from a compile that ran every phase.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: DiagnosticReportVersion::V1,
            items: Vec::new(),
            truncated: false,
            total_count: 0,
            stopped_at: None,
        }
    }

    /// Bound `diagnostics` to [`MAX_REPORT_ITEMS`], keeping emission order.
    #[must_use]
    pub fn from_diagnostics(
        diagnostics: impl IntoIterator<Item = CompileDiagnostic>,
        stopped_at: Option<Phase>,
    ) -> Self {
        let mut items = Vec::new();
        let mut total = 0_usize;
        for diagnostic in diagnostics {
            total += 1;
            if items.len() < MAX_REPORT_ITEMS {
                items.push(diagnostic.bounded());
            }
        }
        Self {
            schema_version: DiagnosticReportVersion::V1,
            truncated: total > items.len(),
            total_count: u32::try_from(total).unwrap_or(u32::MAX),
            items,
            stopped_at,
        }
    }

    /// The code of the first error, the primary slug a failed compile reports.
    #[must_use]
    pub fn first_error_code(&self) -> Option<&str> {
        self.items
            .iter()
            .find(|item| item.severity == Severity::Error)
            .map(|item| item.code.as_str())
    }

    /// Whether any carried item is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.severity == Severity::Error)
    }
}

/// Truncate a message to [`MAX_MESSAGE_BYTES`] on a character boundary.
#[must_use]
pub fn bound_message(mut message: String) -> String {
    if message.len() > MAX_MESSAGE_BYTES {
        let mut end = MAX_MESSAGE_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

/// One rejection or note from the source port: a closed reason, the phase that
/// raised it, and whatever structure the phase knows. The message is never
/// used for control flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDiagnostic {
    /// The closed reason for the rejection.
    pub code: SourceDiagnosticCode,
    /// A more specific closed slug the raising phase reports (a verifier
    /// code, a frontend capture code). Absent when `code` is the whole story.
    pub detail_code: Option<String>,
    /// A human-readable explanation of the rejection.
    pub message: String,
    pub severity: Severity,
    pub phase: Phase,
    // The structural fields are boxed: most diagnostics carry none of them,
    // and every rejection travels in a `Result`'s error variant.
    pub location: Option<Box<Location>>,
    pub node_id: Option<String>,
    pub field_path: Option<Box<Vec<String>>>,
    pub related: Option<Box<Vec<Related>>>,
    pub expected: Option<Box<TypeFact>>,
    pub actual: Option<Box<TypeFact>>,
}

impl SourceDiagnostic {
    /// An error-severity diagnostic in the phase its code implies.
    pub fn new(code: SourceDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            detail_code: None,
            message: message.into(),
            severity: Severity::Error,
            phase: code.default_phase(),
            location: None,
            node_id: None,
            field_path: None,
            related: None,
            expected: None,
            actual: None,
        }
    }

    #[must_use]
    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_detail_code(mut self, detail_code: impl Into<String>) -> Self {
        self.detail_code = Some(detail_code.into());
        self
    }

    #[must_use]
    pub fn with_location(mut self, location: Location) -> Self {
        self.location = Some(Box::new(location));
        self
    }

    #[must_use]
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    #[must_use]
    pub fn with_field_path(mut self, field_path: Vec<String>) -> Self {
        self.field_path = Some(Box::new(field_path));
        self
    }

    /// The slug this diagnostic reports on the wire.
    #[must_use]
    pub fn wire_code(&self) -> &str {
        self.detail_code.as_deref().unwrap_or(self.code.slug())
    }

    /// Project this diagnostic onto the wire shape.
    #[must_use]
    pub fn to_compile_diagnostic(&self) -> CompileDiagnostic {
        CompileDiagnostic {
            severity: self.severity,
            code: self.wire_code().to_owned(),
            phase: self.phase,
            message: bound_message(self.message.clone()),
            location: self.location.as_deref().cloned(),
            node_id: self.node_id.clone(),
            field_path: self.field_path.as_deref().cloned(),
            related: self.related.as_deref().cloned(),
            expected: self.expected.as_deref().cloned(),
            actual: self.actual.as_deref().cloned(),
        }
        .bounded()
    }
}

impl SourceDiagnosticCode {
    /// The phase a diagnostic with this code belongs to when the raising site
    /// states no narrower one.
    #[must_use]
    pub const fn default_phase(self) -> Phase {
        match self {
            Self::RequestInvalid => Phase::Request,
            Self::FrontendUnavailable | Self::FrontendOutputInvalid => Phase::FrontendCapture,
            Self::SourceRejected | Self::EntrypointNotAnAgentProgram => Phase::Capture,
            Self::GraphRejected => Phase::Lowering,
        }
    }
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.slug(), self.message)
    }
}
