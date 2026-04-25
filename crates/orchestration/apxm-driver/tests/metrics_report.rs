//! Integration tests for the unified MetricsReport (schema v2).
//!
//! These tests assert the structural SHAPE of the report -- key presence,
//! schema version, nesting -- not exact numeric values. Custom stubs are used
//! in place of the real MetricsSource implementations so these tests run
//! without the `metrics` feature gate.

use apxm_compiler::PipelineDiagnostics;
use apxm_compiler::passes::metrics::CompilerMetricsSource;
use apxm_core::constants::session::metrics_keys;
use apxm_core::metrics::{MetricsReport, MetricsSource};
use serde_json::Value;

const MOCK_BACKEND_KIND: &str = "mock-backend-kind";
const MOCK_BACKEND_NAME: &str = "mock-backend";

fn empty_compiler_diagnostics() -> PipelineDiagnostics {
    PipelineDiagnostics::default()
}

struct StubRuntimeSource;

impl MetricsSource for StubRuntimeSource {
    fn section_name(&self) -> &'static str {
        metrics_keys::SECTION_RUNTIME
    }

    fn collect(&self) -> Value {
        use metrics_keys::execution_keys;
        serde_json::json!({
            metrics_keys::RUNTIME_EXECUTION: {
                execution_keys::NODES_EXECUTED: 2,
                execution_keys::NODES_FAILED: 0,
                execution_keys::DURATION_MS: 100
            },
            metrics_keys::RUNTIME_SCHEDULER: {},
            metrics_keys::TOKEN_ACCOUNTING: {
                metrics_keys::TOTAL: {
                    metrics_keys::INPUT_TOKENS: 10,
                    metrics_keys::OUTPUT_TOKENS: 20,
                    metrics_keys::TOTAL_TOKENS: 30,
                    metrics_keys::CALL_COUNT: 1
                }
            }
        })
    }
}

struct StubBackendSource {
    has_data: bool,
}

impl MetricsSource for StubBackendSource {
    fn section_name(&self) -> &'static str {
        metrics_keys::SECTION_BACKENDS
    }

    fn collect(&self) -> Value {
        if !self.has_data {
            return Value::Null;
        }
        use metrics_keys::graph_status_keys as gsk;
        use metrics_keys::llm_keys;
        let mut per_backend = serde_json::Map::new();
        per_backend.insert(
            MOCK_BACKEND_NAME.to_string(),
            serde_json::json!({ llm_keys::TOTAL_REQUESTS: 1 }),
        );
        serde_json::json!({
            metrics_keys::BACKENDS_AGGREGATE: {
                llm_keys::TOTAL_REQUESTS: 1,
                llm_keys::SUCCESSFUL_REQUESTS: 1,
                llm_keys::FAILED_REQUESTS: 0
            },
            metrics_keys::BACKENDS_PER_BACKEND: per_backend,
            metrics_keys::BACKENDS_GRAPHS: [{
                gsk::BACKEND_KIND: MOCK_BACKEND_KIND,
                gsk::BACKEND_NAME: MOCK_BACKEND_NAME,
                gsk::GRAPH_ID: "g1",
                gsk::PINNED_BLOCKS: 7,
                gsk::PINNED_HANDLES: 3,
                gsk::CRITICAL_PATH_LENGTH: 4,
                gsk::NODE_COUNT: 5
            }]
        })
    }
}

#[test]
fn metrics_report_includes_all_three_sections() {
    let mut report = MetricsReport::new();
    let diag = empty_compiler_diagnostics();
    report.add_source(&CompilerMetricsSource { diagnostics: &diag });
    report.add_source(&StubRuntimeSource);
    report.add_source(&StubBackendSource { has_data: true });

    let json = report.to_json();
    let obj = json.as_object().expect("report is an object");

    assert_eq!(
        obj[metrics_keys::SCHEMA_VERSION],
        metrics_keys::SCHEMA_VERSION_VALUE
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_COMPILER),
        "missing compiler section"
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_RUNTIME),
        "missing runtime section"
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_BACKENDS),
        "missing backends section"
    );

    // Compiler sub-keys
    let compiler = &obj[metrics_keys::SECTION_COMPILER];
    assert!(
        compiler.get(metrics_keys::COMPILER_PASSES).is_some(),
        "missing compiler.passes"
    );
    assert!(
        compiler.get(metrics_keys::COMPILER_SUMMARY).is_some(),
        "missing compiler.summary"
    );

    // Runtime sub-keys
    let runtime = &obj[metrics_keys::SECTION_RUNTIME];
    assert!(
        runtime.get(metrics_keys::RUNTIME_EXECUTION).is_some(),
        "missing runtime.execution"
    );
    assert!(
        runtime.get(metrics_keys::TOKEN_ACCOUNTING).is_some(),
        "missing runtime.token_accounting"
    );

    // Backends sub-keys
    let backends = &obj[metrics_keys::SECTION_BACKENDS];
    assert!(
        backends.get(metrics_keys::BACKENDS_AGGREGATE).is_some(),
        "missing backends.aggregate"
    );
    assert!(
        backends.get(metrics_keys::BACKENDS_PER_BACKEND).is_some(),
        "missing backends.per_backend"
    );
    assert!(
        backends.get(metrics_keys::BACKENDS_GRAPHS).is_some(),
        "missing backends.graphs"
    );
    // Graph-aware backend snapshot shape
    let graphs = backends[metrics_keys::BACKENDS_GRAPHS]
        .as_array()
        .expect("graphs is array");
    assert_eq!(graphs.len(), 1);
    use metrics_keys::graph_status_keys as gsk;
    assert!(graphs[0].get(gsk::BACKEND_KIND).is_some());
    assert!(graphs[0].get(gsk::BACKEND_NAME).is_some());
    assert!(graphs[0].get(gsk::PINNED_BLOCKS).is_some());
    assert!(graphs[0].get(gsk::PINNED_HANDLES).is_some());
    assert!(graphs[0].get(gsk::CRITICAL_PATH_LENGTH).is_some());
}

#[test]
fn metrics_report_compile_only_omits_runtime() {
    let mut report = MetricsReport::new();
    let diag = empty_compiler_diagnostics();
    report.add_source(&CompilerMetricsSource { diagnostics: &diag });

    let json = report.to_json();
    let obj = json.as_object().expect("report is an object");

    assert_eq!(
        obj[metrics_keys::SCHEMA_VERSION],
        metrics_keys::SCHEMA_VERSION_VALUE
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_COMPILER),
        "missing compiler section"
    );
    assert!(
        !obj.contains_key(metrics_keys::SECTION_RUNTIME),
        "runtime section should be absent in compile-only"
    );
    assert!(
        !obj.contains_key(metrics_keys::SECTION_BACKENDS),
        "backends section should be absent in compile-only"
    );
}

#[test]
fn metrics_report_handles_no_llm_calls() {
    let mut report = MetricsReport::new();
    let diag = empty_compiler_diagnostics();
    report.add_source(&CompilerMetricsSource { diagnostics: &diag });
    report.add_source(&StubRuntimeSource);
    report.add_source(&StubBackendSource { has_data: false });

    let json = report.to_json();
    let obj = json.as_object().expect("report is an object");

    assert_eq!(
        obj[metrics_keys::SCHEMA_VERSION],
        metrics_keys::SCHEMA_VERSION_VALUE
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_COMPILER),
        "missing compiler section"
    );
    assert!(
        obj.contains_key(metrics_keys::SECTION_RUNTIME),
        "missing runtime section"
    );
    assert!(
        !obj.contains_key(metrics_keys::SECTION_BACKENDS),
        "backends section should be absent when no LLM calls were made"
    );
}
