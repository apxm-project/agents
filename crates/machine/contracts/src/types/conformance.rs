use serde::{Deserialize, Serialize};

use crate::types::host::HostTier;

pub const PROFILE_DIRECT: &str = "public-webhook-t0";
pub const PROFILE_LINK_TOOLS: &str = "browser-mv3-t1";
pub const PROFILE_LINK_RUNTIME: &str = "host-runtime-t2";

pub const VECTOR_DIRECT_HOST_ID_PRESENT: &str = "tier.direct.host_id_present";
pub const VECTOR_DIRECT_MODE_DECLARED: &str = "tier.direct.mode_declared";
pub const VECTOR_LINK_TOOLS_HOST_ID_PRESENT: &str = "tier.link_tools.host_id_present";
pub const VECTOR_LINK_TOOLS_MODE_DECLARED: &str = "tier.link_tools.mode_declared";
pub const VECTOR_LINK_RUNTIME_HOST_ID_PRESENT: &str = "tier.link_runtime.host_id_present";
pub const VECTOR_LINK_RUNTIME_MODE_DECLARED: &str = "tier.link_runtime.mode_declared";

pub fn profile_for_tier(tier: HostTier) -> &'static str {
    match tier {
        HostTier::Direct => PROFILE_DIRECT,
        HostTier::LinkTools => PROFILE_LINK_TOOLS,
        HostTier::LinkRuntime => PROFILE_LINK_RUNTIME,
    }
}

pub fn vectors_for_tier(tier: HostTier) -> &'static [&'static str] {
    match tier {
        HostTier::Direct => &[VECTOR_DIRECT_HOST_ID_PRESENT, VECTOR_DIRECT_MODE_DECLARED],
        HostTier::LinkTools => &[
            VECTOR_LINK_TOOLS_HOST_ID_PRESENT,
            VECTOR_LINK_TOOLS_MODE_DECLARED,
        ],
        HostTier::LinkRuntime => &[
            VECTOR_LINK_RUNTIME_HOST_ID_PRESENT,
            VECTOR_LINK_RUNTIME_MODE_DECLARED,
        ],
    }
}

/// Signed conformance report produced by a host-sdk conformance harness.
///
/// Implements `apxm.conformance-report`: passed vector ids are flat strings;
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
        "apxm.conformance-report"
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

    /// Returns true iff the report has a host id, declares the canonical profile
    /// for its tier, carries at least one result, uses only tier-owned vector
    /// ids, and no vector appears in both `passed[]` and `failed[]`.
    pub fn verify_report(&self, report: &ConformanceReport) -> bool {
        if report.host_id.is_empty() || report.passed.is_empty() && report.failed.is_empty() {
            return false;
        }
        if report.profile != profile_for_tier(report.tier) {
            return false;
        }
        let allowed = vectors_for_tier(report.tier);
        if report
            .passed
            .iter()
            .any(|passed| !allowed.contains(&passed.as_str()))
            || report
                .failed
                .iter()
                .any(|failed| !allowed.contains(&failed.name.as_str()))
        {
            return false;
        }
        !report
            .failed
            .iter()
            .any(|failed| report.passed.iter().any(|passed| passed == &failed.name))
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
        let r = report(
            vec![
                VECTOR_LINK_TOOLS_HOST_ID_PRESENT,
                VECTOR_LINK_TOOLS_MODE_DECLARED,
            ],
            vec![],
        );
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

    #[test]
    fn cross_tier_vector_fails_verify() {
        let h = ConformanceHarness::new();
        let r = report(vec![VECTOR_DIRECT_HOST_ID_PRESENT], vec![]);
        assert!(!h.verify_report(&r));
    }

    #[test]
    fn wrong_profile_fails_verify() {
        let h = ConformanceHarness::new();
        let mut r = report(vec![VECTOR_LINK_TOOLS_HOST_ID_PRESENT], vec![]);
        r.profile = PROFILE_DIRECT.into();
        assert!(!h.verify_report(&r));
    }
}
