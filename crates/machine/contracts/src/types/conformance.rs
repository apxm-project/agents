use serde::{Deserialize, Serialize};

use crate::types::host::HostTier;

/// Signed conformance report produced by a host-sdk conformance harness.
///
/// Mirrors `apxm.conformance-report.v1`: passed vector ids are flat strings;
/// failures are structured by vector name and reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub report_id: String,
    pub host_id: String,
    pub profile: String,
    pub tier: HostTier,
    pub passed: Vec<String>,
    pub failed: Vec<FailedVector>,
    pub timestamp: String,
    pub signed_by: Option<String>,
    pub signature: Option<String>,
    pub harness_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedVector {
    pub name: String,
    pub reason: String,
    pub actual: Option<serde_json::Value>,
    pub expected: Option<serde_json::Value>,
}

impl ConformanceReport {
    pub fn schema_v1() -> &'static str {
        "apxm.conformance-report.v1"
    }
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

    /// Returns true iff the report has a host id, at least one result, and no
    /// vector appears in both `passed[]` and `failed[]`.
    pub fn verify_report(&self, report: &ConformanceReport) -> bool {
        if report.host_id.is_empty() || report.passed.is_empty() && report.failed.is_empty() {
            return false;
        }
        !report.failed.iter().any(|failed| {
            report
                .passed
                .iter()
                .any(|passed| passed == &failed.name)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(passed: Vec<&str>, failed: Vec<FailedVector>) -> ConformanceReport {
        ConformanceReport {
            report_id: "report-1".into(),
            host_id: "browser".into(),
            profile: "browser-mv3-t1".into(),
            tier: HostTier::LinkTools,
            passed: passed.into_iter().map(str::to_string).collect(),
            failed,
            timestamp: "2026-06-30T00:00:00Z".into(),
            signed_by: None,
            signature: None,
            harness_version: Some("test".into()),
        }
    }

    #[test]
    fn consistent_pass_report_verifies() {
        let h = ConformanceHarness::new();
        let r = report(vec!["v1", "v2"], vec![]);
        assert!(h.verify_report(&r));
    }

    #[test]
    fn overlapping_report_fails_verify() {
        let h = ConformanceHarness::new();
        let r = report(
            vec!["v1"],
            vec![FailedVector {
                name: "v1".into(),
                reason: "overlap".into(),
                actual: None,
                expected: None,
            }],
        );
        assert!(!h.verify_report(&r));
    }
}
