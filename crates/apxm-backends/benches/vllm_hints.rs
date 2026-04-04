//! Criterion benchmarks for vLLM graph-aware hints overhead.

use apxm_backends::llm::backends::vllm::{ApxmGraphHints, GraphMetadata, NodeSpec};
use apxm_backends::llm::backends::LLMRequest;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_request_with_hints(c: &mut Criterion) {
    c.bench_function("llm_request_with_hints", |b| {
        b.iter(|| {
            let hints = ApxmGraphHints::critical_path(
                "benchmark-graph",
                "exec-001",
                5,
                "reasoning-node",
                vec![6, 7, 8],
                30_000,
            );

            let request = LLMRequest::new(black_box("Benchmark prompt"))
                .with_temperature(0.0)
                .with_apxm_hints(hints);

            // Serialize to JSON to measure full overhead
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
            5,
            "reasoning-node",
            vec![6, 7, 8, 9, 10],
            30_000,
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
            .with_pin_ttl(30_000)
            .with_critical_path_length(10)
            .with_nodes(vec![
                NodeSpec {
                    node_id: 1,
                    node_name: Some("architect".to_string()),
                    estimated_prompt_tokens: Some(500),
                    downstream_nodes: vec![2, 3],
                    priority_class: Some("critical_path".to_string()),
                    reuse_group: Some("planning".to_string()),
                    is_critical_path: true,
                },
                NodeSpec {
                    node_id: 2,
                    node_name: Some("coder-1".to_string()),
                    estimated_prompt_tokens: Some(1500),
                    downstream_nodes: vec![4],
                    priority_class: Some("parallel".to_string()),
                    reuse_group: None,
                    is_critical_path: false,
                },
                NodeSpec {
                    node_id: 3,
                    node_name: Some("coder-2".to_string()),
                    estimated_prompt_tokens: Some(1500),
                    downstream_nodes: vec![4],
                    priority_class: Some("parallel".to_string()),
                    reuse_group: None,
                    is_critical_path: false,
                },
                NodeSpec {
                    node_id: 4,
                    node_name: Some("reviewer".to_string()),
                    estimated_prompt_tokens: Some(800),
                    downstream_nodes: vec![],
                    priority_class: Some("critical_path".to_string()),
                    reuse_group: None,
                    is_critical_path: true,
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
