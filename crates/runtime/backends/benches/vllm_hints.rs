//! Criterion benchmarks for vLLM graph-aware hints overhead.

use apxm_backends::llm::backends::LLMRequest;
use apxm_backends::llm::backends::vllm::{ApxmGraphHints, GraphMetadata, NodeSpec};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_request_with_hints(c: &mut Criterion) {
    c.bench_function("llm_request_with_hints", |b| {
        b.iter(|| {
            let hints = ApxmGraphHints::critical_path(
                "benchmark-graph",
                "exec-001",
                "reasoning-node",
                "node-execution:reasoning-node",
                vec!["node:6".into(), "node:7".into(), "node:8".into()],
            );

            let request = LLMRequest::new(black_box("Benchmark prompt"))
                .with_temperature(0.0)
                .with_apxm_hints(hints);

            let _json = serde_json::to_value(&request.apxm_hints).unwrap();
            black_box(request);
        });
    });
}

fn bench_request_without_hints(c: &mut Criterion) {
    c.bench_function("llm_request_without_hints", |b| {
        b.iter(|| {
            let request = LLMRequest::new(black_box("Benchmark prompt")).with_temperature(0.0);

            black_box(request);
        });
    });
}

fn bench_hints_serialization(c: &mut Criterion) {
    c.bench_function("hints_serialization", |b| {
        let hints = ApxmGraphHints::critical_path(
            "benchmark-graph",
            "exec-001",
            "reasoning-node",
            "node-execution:reasoning-node",
            vec!["node:6".into()],
        );

        b.iter(|| {
            let json = serde_json::to_value(black_box(&hints)).unwrap();
            black_box(json);
        });
    });
}

fn bench_graph_registration_payload(c: &mut Criterion) {
    c.bench_function("graph_registration_payload", |b| {
        let metadata = GraphMetadata::new("workflow-bench", "exec-bench")
            .with_critical_path_length(10)
            .with_nodes(vec![
                NodeSpec {
                    node_ref: "architect".into(),
                    successor_refs: vec!["coder-1".into(), "coder-2".into()],
                    estimated_input_tokens: Some(500),
                    is_critical_path: true,
                },
                NodeSpec {
                    node_ref: "coder-1".into(),
                    successor_refs: vec!["reviewer".into()],
                    estimated_input_tokens: Some(1500),
                    is_critical_path: false,
                },
            ]);

        b.iter(|| {
            let json = serde_json::to_value(black_box(&metadata)).unwrap();
            black_box(json);
        });
    });
}

criterion_group!(
    benches,
    bench_request_with_hints,
    bench_request_without_hints,
    bench_hints_serialization,
    bench_graph_registration_payload
);
criterion_main!(benches);
