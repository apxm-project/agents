//! The closed rejection vocabulary of the source-bundle compile port.
//!
//! A caller binds this crate to compile submitted source text; it never learns
//! that an interpreter ran. Every failure — an absent interpreter, an absent
//! frontend package, a syntax error, a raw AIS spelling, a drifted model
//! reference, an unresolved Tool reference, a graph that does not lower — is one
//! member of [`SourceDiagnosticCode`]. There is no open variant and no free-form
//! error channel.

use std::fmt;

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

    /// Decode the closed reason token a capture harness reports on its first
    /// stderr line. An unrecognized token is not trusted as a reason: the port
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

/// One rejection: a closed code and a human-readable explanation. The message is
/// never used for control flow.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceDiagnostic {
    /// The closed reason for the rejection.
    pub code: SourceDiagnosticCode,
    /// A human-readable explanation of the rejection.
    pub message: String,
}

impl SourceDiagnostic {
    pub fn new(code: SourceDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.slug(), self.message)
    }
}
