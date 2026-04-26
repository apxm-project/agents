//! Compiler-backed implementation of the runtime linker trait.

use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::{self, Pipeline};
use apxm_compiler::{AirEdge, AirModule, AirNode, AirParam};
use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
use apxm_core::types::OptimizationLevel;
use apxm_core::types::execution::{ExecutionDag, TaskDag};
use apxm_core::types::{AISOperationType, DependencyType, Value};
use apxm_core::utils::build::MlirEnvReport;
use apxm_core::{log_debug, log_info};
use apxm_runtime::{InnerPlanLinker, RuntimeError};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

/// Compiler-backed implementation of the runtime linker trait.
pub struct CompilerInnerPlanLinker {
    context: Arc<parking_lot::Mutex<apxm_compiler::Context>>,
}

impl CompilerInnerPlanLinker {
    pub fn new() -> Result<Self, RuntimeError> {
        // Check MLIR availability first; if not ready, return an error so the
        // caller can fall back to NoOpLinker (graph-direct mode).
        let report = MlirEnvReport::detect();
        report.apply_env();
        if !report.is_ready() {
            return Err(RuntimeError::State(format!(
                "MLIR toolchain not available for inner-plan linker: {}",
                report.summary()
            )));
        }

        let context = apxm_compiler::Context::new().map_err(|e| {
            RuntimeError::State(format!("Failed to initialize compiler context: {}", e))
        })?;

        Ok(Self {
            context: Arc::new(parking_lot::Mutex::new(context)),
        })
    }
}

#[async_trait]
impl InnerPlanLinker for CompilerInnerPlanLinker {
    async fn link_inner_plan(
        &self,
        air_payload: &str,
        source_name: &str,
    ) -> Result<ExecutionDag, RuntimeError> {
        log_debug!(
            "driver::inner_plan",
            source = %source_name,
            payload_length = air_payload.len(),
            "Linking inner plan AIR"
        );

        let context = self.context.lock();
        let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);
        let module = pipeline.compile(air_payload).map_err(|e| {
            RuntimeError::State(format!(
                "Inner plan AIR compilation failed for '{}': {}",
                source_name, e
            ))
        })?;

        let artifact_bytes = module.generate_artifact_bytes().map_err(|e| {
            RuntimeError::State(format!("Inner plan artifact generation failed: {}", e))
        })?;

        let artifact = Artifact::from_bytes(&artifact_bytes).map_err(|e| {
            RuntimeError::State(format!("Inner plan artifact parsing failed: {}", e))
        })?;

        let dag = artifact.into_entry_dag().ok_or_else(|| {
            RuntimeError::State("Inner plan artifact contains no entry DAG".to_string())
        })?;

        // Validate the inner DAG before returning it to the runtime.
        // This ensures we catch cycles or inconsistent token/node references
        // early and return a clear error instead of allowing the scheduler
        // to detect a deadlock at runtime.
        if let Err(e) = dag.validate() {
            return Err(RuntimeError::State(format!(
                "Inner plan DAG validation failed: {}",
                e
            )));
        }

        log_info!(
            "driver::inner_plan",
            source = %source_name,
            nodes = dag.nodes.len(),
            edges = dag.edges.len(),
            "Inner plan linked successfully"
        );

        Ok(dag)
    }

    async fn link_task_dag(&self, dag: TaskDag) -> Result<ExecutionDag, RuntimeError> {
        log_debug!(
            "driver::inner_plan",
            dag_name = %dag.name,
            tasks = dag.tasks.len(),
            "Linking inner plan task DAG via graph canonicalization"
        );

        let air_module = task_dag_to_air_module(&dag)?;
        let air_text = air_module.to_air().map_err(|e| {
            RuntimeError::State(format!("Inner plan task AIR emission failed: {}", e))
        })?;
        let context = self.context.lock();
        let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);
        let module = pipeline.compile(&air_text).map_err(|e| {
            RuntimeError::State(format!("Inner plan task graph compilation failed: {}", e))
        })?;

        let artifact_bytes = module.generate_artifact_bytes().map_err(|e| {
            RuntimeError::State(format!("Inner plan task artifact generation failed: {}", e))
        })?;
        let artifact = Artifact::from_bytes(&artifact_bytes).map_err(|e| {
            RuntimeError::State(format!("Inner plan task artifact parsing failed: {}", e))
        })?;
        let execution_dag = artifact.into_entry_dag().ok_or_else(|| {
            RuntimeError::State("Inner plan task artifact contains no entry DAG".to_string())
        })?;
        execution_dag.validate()?;

        log_info!(
            "driver::inner_plan",
            nodes = execution_dag.nodes.len(),
            edges = execution_dag.edges.len(),
            "Inner plan task DAG linked successfully via graph path"
        );

        Ok(execution_dag)
    }
}

fn task_dag_to_air_module(dag: &TaskDag) -> Result<AirModule, RuntimeError> {
    dag.validate()?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_node_ids = HashSet::new();
    let mut first_node_by_task = HashMap::new();
    let mut last_node_by_task = HashMap::new();

    for task in &dag.tasks {
        let realized_nodes: Vec<u64> = if task.nodes.is_empty() {
            vec![task.id * 1000]
        } else {
            task.nodes.clone()
        };

        for (index, node_id) in realized_nodes.iter().copied().enumerate() {
            if !seen_node_ids.insert(node_id) {
                return Err(RuntimeError::State(format!(
                    "TaskDag '{}' maps to duplicate node id {}",
                    dag.name, node_id
                )));
            }

            let node_name = if realized_nodes.len() == 1 {
                task.name.clone()
            } else {
                format!("{}_{index}", task.name)
            };

            let mut attributes = HashMap::new();
            attributes.insert(
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String(task.description.clone()),
            );

            nodes.push(AirNode {
                id: node_id,
                name: node_name,
                op: AISOperationType::Ask,
                attributes,
            });
        }

        let first = *realized_nodes.first().expect("realized_nodes is non-empty");
        let last = *realized_nodes.last().expect("realized_nodes is non-empty");
        first_node_by_task.insert(task.id, first);
        last_node_by_task.insert(task.id, last);

        for pair in realized_nodes.windows(2) {
            edges.push(AirEdge {
                from: pair[0],
                to: pair[1],
                dependency: DependencyType::Data,
            });
        }
    }

    for task in &dag.tasks {
        let to = *first_node_by_task.get(&task.id).ok_or_else(|| {
            RuntimeError::State(format!("Missing first node mapping for task {}", task.id))
        })?;
        for dep in &task.depends_on {
            let from = *last_node_by_task.get(dep).ok_or_else(|| {
                RuntimeError::State(format!("Missing dependency mapping for task {}", dep))
            })?;
            edges.push(AirEdge {
                from,
                to,
                dependency: DependencyType::Data,
            });
        }
    }

    let mut metadata = HashMap::new();
    if dag.metadata.is_entry {
        metadata.insert(graph_meta::IS_ENTRY.to_string(), Value::Bool(true));
    }

    let air_module = AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| dag.name.clone()),
        nodes,
        edges,
        parameters: dag
            .metadata
            .parameters
            .iter()
            .map(|p| AirParam {
                name: p.name.clone(),
                type_name: p.type_name.clone(),
            })
            .collect(),
        metadata,
    };

    air_module
        .validate()
        .map_err(|e| RuntimeError::State(format!("TaskDag graph validation failed: {}", e)))?;

    Ok(air_module)
}
