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

/// The closed structural region lineage set emitted by the source frontends.
/// These names describe source control structure only; they are not AIS kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralRegionKind {
    Branch,
    Loop,
    Join,
    Switch,
    Try,
    Throw,
    Yield,
    Return,
}

impl StructuralRegionKind {
    /// Every structural region lineage kind, for schema drift checks.
    pub const ALL: &'static [Self] = &[
        Self::Branch,
        Self::Loop,
        Self::Join,
        Self::Switch,
        Self::Try,
        Self::Throw,
        Self::Yield,
        Self::Return,
    ];
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

/// One digest-bound source span for a structural AIR region.
///
/// `region_id` is the canonical AIR structural-region identity. For a loop it
/// is the loop body region (the AIR loop adopts that identity); for the other
/// controls it is the control node's structural region identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionSpan {
    pub region_id: String,
    pub source_file: String,
    pub span: Span,
    pub structural_kind: StructuralRegionKind,
}

/// One digest-bound source span for a typed graph edge (data or context flow).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSpan {
    pub edge_id: String,
    pub source_file: String,
    pub span: Span,
}

/// The `SourceMapBody` shared by AIR, artifacts, and standalone source maps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMap {
    pub schema_version: SourceMapVersion,
    pub source_language: SourceLanguage,
    pub node_spans: Vec<NodeSpan>,
    pub region_annotations: Vec<RegionAnnotation>,
    /// Structural source spans are additive for compatibility with pre-lineage
    /// vectors; all current frontends emit one for every structural control.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub region_spans: Vec<RegionSpan>,
    /// Typed data/context edge source lineage emitted by current frontends.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_spans: Vec<EdgeSpan>,
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
        let mut seen_region_spans = std::collections::HashSet::new();
        for entry in &self.region_spans {
            if !is_identifier(&entry.region_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    entry.region_id.clone(),
                    "region span region_id is not a contract identifier",
                ));
            }
            if entry.source_file.trim().is_empty() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    entry.region_id.clone(),
                    "region span source_file is not empty",
                ));
            }
            if !entry.span.is_forward() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonForwardSpan,
                    entry.region_id.clone(),
                    "region span end precedes its start",
                ));
            }
            if !seen_region_spans.insert(entry.region_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    entry.region_id.clone(),
                    "region span region_id is not unique",
                ));
            }
        }
        let mut seen_edge_spans = std::collections::HashSet::new();
        for entry in &self.edge_spans {
            if !is_identifier(&entry.edge_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    entry.edge_id.clone(),
                    "edge span edge_id is not a contract identifier",
                ));
            }
            if entry.source_file.trim().is_empty() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    entry.edge_id.clone(),
                    "edge span source_file is not empty",
                ));
            }
            if !entry.span.is_forward() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonForwardSpan,
                    entry.edge_id.clone(),
                    "edge span end precedes its start",
                ));
            }
            if !seen_edge_spans.insert(entry.edge_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    entry.edge_id.clone(),
                    "edge span edge_id is not unique",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map_with_region(region: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [],
            "region_spans": [region]
        })
    }

    #[test]
    fn structural_region_span_accepts_loop_and_return_lineage() {
        let value = serde_json::json!({
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [],
            "region_spans": [
                {"region_id": "program.loop.body", "source_file": "agent.py",
                 "span": {"start_line": 2, "start_column": 0, "end_line": 4, "end_column": 1},
                 "structural_kind": "loop"},
                {"region_id": "program.return.1", "source_file": "agent.py",
                 "span": {"start_line": 5, "start_column": 0, "end_line": 5, "end_column": 12},
                 "structural_kind": "return"}
            ]
        });
        assert!(verify_source_map_json(&value).is_accepted());
    }

    #[test]
    fn unknown_structural_region_kind_fails_closed() {
        let value = map_with_region(serde_json::json!({
            "region_id": "program.branch.1",
            "source_file": "agent.py",
            "span": {"start_line": 1, "start_column": 0, "end_line": 1, "end_column": 1},
            "structural_kind": "future_control"
        }));
        assert!(!verify_source_map_json(&value).is_accepted());
    }

    #[test]
    fn malformed_and_duplicate_structural_region_spans_fail_closed() {
        let malformed = map_with_region(serde_json::json!({
            "region_id": "program.branch.1",
            "source_file": "agent.py",
            "span": {"start_line": 4, "start_column": 0, "end_line": 2, "end_column": 0},
            "structural_kind": "branch"
        }));
        assert!(!verify_source_map_json(&malformed).is_accepted());

        let duplicate = serde_json::json!({
            "schema_version": "apxm.source-map", "source_language": "python",
            "node_spans": [], "region_annotations": [], "region_spans": [
                {"region_id": "program.join.1", "source_file": "agent.py",
                 "span": {"start_line": 1, "start_column": 0, "end_line": 1, "end_column": 1},
                 "structural_kind": "join"},
                {"region_id": "program.join.1", "source_file": "agent.py",
                 "span": {"start_line": 2, "start_column": 0, "end_line": 2, "end_column": 1},
                 "structural_kind": "join"}
            ]
        });
        assert!(!verify_source_map_json(&duplicate).is_accepted());
    }

    #[test]
    fn missing_structural_region_lineage_field_fails_closed() {
        let value = map_with_region(serde_json::json!({
            "region_id": "program.branch.1",
            "source_file": "agent.py",
            "span": {"start_line": 1, "start_column": 0, "end_line": 1, "end_column": 1}
        }));
        assert!(!verify_source_map_json(&value).is_accepted());
    }
}
