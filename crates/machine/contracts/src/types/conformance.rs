use serde::{Deserialize, Serialize};

/// Signed conformance report produced by the host-sdk conformance harness.
///
/// Studio and eval consume these. The `pass` field is authoritative only when
/// `signed_by` is present; unsigned reports are treated as informational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub schema_version: String,
    /// Adapter identifier (matches HostAdapterCard.id).
    pub adapter_id: String,
    /// Host profile key: "browser", "odoo", "sidecar", "ide", "mobile", "iot", "warehouse".
    pub host_profile: String,
    /// Tier declared by the adapter: "direct", "link-tools", "link-runtime".
    pub tier: String,
    /// Results for each test vector run.
    pub test_vectors: Vec<ConformanceVectorResult>,
    /// Unix timestamp (seconds) when the report was produced.
    pub issued_at: i64,
    /// DID or public key fingerprint of the entity that signed the report (optional).
    pub signed_by: Option<String>,
    /// True iff all required vectors passed.
    pub pass: bool,
}

impl ConformanceReport {
    pub fn schema_v1() -> &'static str {
        "apxm.conformance-report.v1"
    }
}

/// Result for a single conformance test vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceVectorResult {
    /// Vector id (e.g. "tier-selection.link-tools.browser.1").
    pub vector_id: String,
    /// Whether this vector passed.
    pub pass: bool,
    /// Optional machine-readable evidence (e.g. computed output for diff).
    pub evidence: Option<serde_json::Value>,
    /// Error message if pass == false.
    pub error: Option<String>,
}

/// Minimal conformance harness: verifies that a report is internally consistent.
pub struct ConformanceHarness;

impl Default for ConformanceHarness {
    fn default() -> Self {
        Self
    }
}

impl ConformanceHarness {
    pub fn new() -> Self {
        Self
    }

    /// Returns true iff the report is self-consistent: overall pass matches all vectors passing.
    pub fn verify_report(&self, report: &ConformanceReport) -> bool {
        let all_pass = report.test_vectors.iter().all(|v| v.pass);
        report.pass == all_pass && !report.test_vectors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(id: &str, pass: bool) -> ConformanceVectorResult {
        ConformanceVectorResult {
            vector_id: id.into(),
            pass,
            evidence: None,
            error: None,
        }
    }

    #[test]
    fn consistent_pass_report_verifies() {
        let h = ConformanceHarness::new();
        let r = ConformanceReport {
            schema_version: ConformanceReport::schema_v1().into(),
            adapter_id: "browser".into(),
            host_profile: "browser".into(),
            tier: "link-tools".into(),
            test_vectors: vec![vector("v1", true), vector("v2", true)],
            issued_at: 1000000000,
            signed_by: None,
            pass: true,
        };
        assert!(h.verify_report(&r));
    }

    #[test]
    fn inconsistent_report_fails_verify() {
        let h = ConformanceHarness::new();
        let r = ConformanceReport {
            schema_version: ConformanceReport::schema_v1().into(),
            adapter_id: "browser".into(),
            host_profile: "browser".into(),
            tier: "link-tools".into(),
            test_vectors: vec![vector("v1", true), vector("v2", false)],
            issued_at: 1000000000,
            signed_by: None,
            pass: true, // claims pass but v2 failed
        };
        assert!(!h.verify_report(&r));
    }
}
