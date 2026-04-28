//! `/api/agents` — builtin agent profile listing.

use axum::response::{IntoResponse, Json};

use apxm_core::types::operations::metadata::get_all_operations;

/// GET /api/agents
pub async fn agents_handler() -> impl IntoResponse {
    let profiles: Vec<serde_json::Value> = get_all_operations()
        .filter(|op| {
            matches!(
                format!("{:?}", op.category).as_str(),
                "Coordination" | "Communication"
            )
        })
        .flat_map(|op| {
            op.fields
                .iter()
                .filter(|f| f.ref_type.is_some())
                .map(move |f| {
                    serde_json::json!({
                        "op": op.name,
                        "field": f.name,
                        "ref_type": format!("{:?}", f.ref_type),
                    })
                })
        })
        .collect();

    let builtin_agents = vec![
        serde_json::json!({ "name": "architect", "description": "Designs system architecture and high-level solutions", "skills": ["design", "planning", "analysis"], "category": "engineering" }),
        serde_json::json!({ "name": "coder", "description": "Implements code based on specifications", "skills": ["coding", "implementation", "debugging"], "category": "engineering" }),
        serde_json::json!({ "name": "reviewer", "description": "Reviews code for quality, bugs, and best practices", "skills": ["review", "testing", "quality"], "category": "engineering" }),
        serde_json::json!({ "name": "tester", "description": "Writes and runs tests to validate implementations", "skills": ["testing", "validation", "qa"], "category": "engineering" }),
        serde_json::json!({ "name": "researcher", "description": "Researches topics and gathers information", "skills": ["research", "analysis", "summarization"], "category": "knowledge" }),
        serde_json::json!({ "name": "planner", "description": "Creates detailed plans and task breakdowns", "skills": ["planning", "decomposition", "scheduling"], "category": "management" }),
        serde_json::json!({ "name": "coordinator", "description": "Orchestrates multi-agent collaboration", "skills": ["coordination", "delegation", "monitoring"], "category": "management" }),
        serde_json::json!({ "name": "writer", "description": "Creates documentation and written content", "skills": ["writing", "documentation", "communication"], "category": "content" }),
        serde_json::json!({ "name": "analyst", "description": "Analyzes data and produces insights", "skills": ["analysis", "statistics", "visualization"], "category": "knowledge" }),
        serde_json::json!({ "name": "debugger", "description": "Diagnoses and fixes bugs systematically", "skills": ["debugging", "diagnostics", "root-cause-analysis"], "category": "engineering" }),
        serde_json::json!({ "name": "optimizer", "description": "Optimizes performance and resource usage", "skills": ["optimization", "profiling", "benchmarking"], "category": "engineering" }),
        serde_json::json!({ "name": "security", "description": "Audits code and systems for security vulnerabilities", "skills": ["security", "auditing", "compliance"], "category": "engineering" }),
        serde_json::json!({ "name": "devops", "description": "Manages deployment, CI/CD, and infrastructure", "skills": ["deployment", "ci-cd", "infrastructure"], "category": "operations" }),
        serde_json::json!({ "name": "data_engineer", "description": "Designs and manages data pipelines", "skills": ["data", "pipelines", "etl"], "category": "data" }),
        serde_json::json!({ "name": "ml_engineer", "description": "Builds and deploys machine learning models", "skills": ["ml", "training", "inference"], "category": "data" }),
        serde_json::json!({ "name": "ux_designer", "description": "Designs user interfaces and experiences", "skills": ["design", "ux", "accessibility"], "category": "design" }),
    ];

    Json(serde_json::json!({
        "agents": builtin_agents,
        "operation_refs": profiles,
    }))
}
