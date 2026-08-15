//! `apxm.source-map` — closed consumer types and verification.
//!
//! A source map is a non-executable mapping from operations and regions back to
//! authored spans. It carries no runtime placement, endpoint, or credential:
//! `deny_unknown_fields` enforces that closure at the decode boundary.

use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict};
use crate::grammar::is_identifier;

/// The single accepted `schema_version` for a source map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceMapVersion {
    #[serde(rename = "apxm.source-map")]
    V1,
}

/// The closed authoring-language set shared across the semantic surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceLanguage {
    Python,
    Typescript,
}

impl SourceLanguage {
    /// Every authoring language, so a drift guard reads the closure off the
    /// enum instead of restating it.
    pub const ALL: &'static [Self] = &[Self::Python, Self::Typescript];

    /// The canonical wire string, identical to the schema `source_language` enum.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Typescript => "typescript",
        }
    }
}

/// The closed generic region-annotation set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionAnnotationKind {
    None,
    StructuralLoop,
}

impl RegionAnnotationKind {
    /// Every region annotation, so a drift guard reads the closure off the enum
    /// instead of restating it.
    pub const ALL: &'static [Self] = &[Self::None, Self::StructuralLoop];
}

/// A forward source span. Column is 0-based; line is 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl Span {
    /// Whether the span end is at or after its start.
    #[must_use]
    pub fn is_forward(&self) -> bool {
        (self.start_line, self.start_column) <= (self.end_line, self.end_column)
    }
}

/// One operation-to-source mapping entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpan {
    pub node_id: String,
    pub source_file: String,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_annotation: Option<String>,
}

/// One region annotation entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionAnnotation {
    pub region_id: String,
    pub annotation: RegionAnnotationKind,
}

/// The `SourceMapBody` shared by AIR, artifacts, and standalone source maps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMap {
    pub schema_version: SourceMapVersion,
    pub source_language: SourceLanguage,
    pub node_spans: Vec<NodeSpan>,
    pub region_annotations: Vec<RegionAnnotation>,
}

impl SourceMap {
    /// Verify a decoded source map. Grammar and forward-span checks only; the
    /// closed shape is already guaranteed by decode.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();
        self.collect(&mut verdict);
        verdict.finish()
    }

    pub(crate) fn collect(&self, verdict: &mut Verdict) {
        for entry in &self.node_spans {
            if !is_identifier(&entry.node_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    entry.node_id.clone(),
                    "node span node_id is not a contract identifier",
                ));
            }
            if !entry.span.is_forward() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonForwardSpan,
                    entry.node_id.clone(),
                    "span end precedes its start",
                ));
            }
        }
        for entry in &self.region_annotations {
            if !is_identifier(&entry.region_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    entry.region_id.clone(),
                    "region annotation region_id is not a contract identifier",
                ));
            }
        }
    }
}

/// Verify a source map presented as JSON, failing closed on decode errors.
#[must_use]
pub fn verify_source_map_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<SourceMap>(value.clone()) {
        Ok(map) => map.verify(),
        Err(error) => crate::diagnostic::schema_violation("source_map", &error),
    }
}
