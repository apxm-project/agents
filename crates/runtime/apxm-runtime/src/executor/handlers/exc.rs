//! EXC operation - Execute code in sandbox

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute,
    get_optional_u64_attribute,
};
use crate::sandbox::constants::{executables, session_prefixes};
use crate::sandbox::{ExecRequest, IsolationLevel, ValidationResult};
use apxm_core::constants::{defaults, graph::attrs as graph_attrs, runtime::belief_keys};

const ERR_EXC_SANDBOX_SELECT_PREFIX: &str = "Sandbox selection failed";
const ERR_EXC_SANDBOX_SESSION_PREFIX: &str = "Sandbox session failed";
const ERR_EXC_SANDBOX_EXEC_PREFIX: &str = "Sandbox execution failed";
const ERR_EXC_SANDBOX_DEGRADED: &str = "Sandbox backend selected with degraded guarantees";
const ERR_EXC_SANDBOX_NOT_CONFIGURED: &str =
    "Sandbox execution requires a configured sandbox backend";

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Extract code from 'code' attribute, or fall back to first input
    let code = get_optional_string_attribute(node, graph_attrs::CODE)?
        .or_else(|| {
            inputs
                .first()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: "EXC requires 'code' attribute or input".to_string(),
        })?;

    let interpreter = get_optional_string_attribute(node, graph_attrs::INTERPRETER)?
        .unwrap_or_else(|| executables::BASH.to_string());

    let timeout_secs = get_optional_u64_attribute(node, graph_attrs::TIMEOUT)?
        .unwrap_or(defaults::DEFAULT_TIMEOUT_MS / 1000);

    // AAM transition
    let transition_label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!("{}{}:started", belief_keys::EXC_PREFIX, node.id),
        Value::String(format!("Executing {} script", interpreter)),
        transition_label.clone(),
    );

    let timeout = std::time::Duration::from_secs(timeout_secs);
    let result = if ctx.sandbox_registry.is_empty() {
        return Err(apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: ERR_EXC_SANDBOX_NOT_CONFIGURED.to_string(),
        });
    } else {
        let script_dir =
            tempfile::tempdir().map_err(|e| apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{ERR_EXC_SANDBOX_EXEC_PREFIX}: {e}"),
            })?;
        let script_path = script_dir.path().join(session_prefixes::SCRIPT);
        tokio::fs::write(&script_path, &code).await.map_err(|e| {
            apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{ERR_EXC_SANDBOX_EXEC_PREFIX}: {e}"),
            }
        })?;

        let request = ExecRequest {
            min_isolation: IsolationLevel::OsLevel,
            program: interpreter.clone(),
            args: vec![script_path.to_string_lossy().into_owned()],
            working_dir: Some(script_dir.path().to_path_buf()),
            timeout,
            read_paths: vec![script_dir.path().to_path_buf()],
            write_paths: vec![script_dir.path().to_path_buf()],
            needs_network: false,
            needs_process_spawn: true,
            origin_op: Some(node.op_type.to_string()),
            origin_node_id: Some(node.id),
            ..ExecRequest::default()
        };

        let selection = ctx
            .sandbox_registry
            .select_for_request(&request)
            .map_err(|e| apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{ERR_EXC_SANDBOX_SELECT_PREFIX}: {e}"),
            })?;

        if let ValidationResult::Degraded { warnings } = &selection.validation {
            tracing::warn!(
                node_id = node.id,
                backend = %selection.backend.capabilities().name,
                warnings = ?warnings,
                "Sandbox backend selected with degraded guarantees"
            );
            return Err(apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{ERR_EXC_SANDBOX_DEGRADED}: {}", warnings.join("; ")),
            });
        }

        let session = selection.backend.create_session().await.map_err(|e| {
            apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{ERR_EXC_SANDBOX_SESSION_PREFIX}: {e}"),
            }
        })?;

        let exec_result = match selection.backend.execute(&session, request).await {
            Ok(result) => result,
            Err(error) => {
                let _ = selection.backend.destroy_session(session).await;
                return Err(apxm_core::error::RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("{ERR_EXC_SANDBOX_EXEC_PREFIX}: {error}"),
                });
            }
        };

        let _ = selection.backend.destroy_session(session).await;
        exec_result
    };

    if result.timed_out {
        ctx.aam.set_belief(
            format!("{}{}:timed_out", belief_keys::EXC_PREFIX, node.id),
            Value::Bool(true),
            transition_label,
        );
        return Err(apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Script execution timed out after {timeout_secs} seconds"),
        });
    }

    let output = match result.exit_code {
        Some(0) => result.stdout,
        Some(code) => format!(
            "Exit code: {code}\nStdout: {}\nStderr: {}",
            result.stdout, result.stderr
        ),
        None => format!(
            "Exit status unavailable\nStdout: {}\nStderr: {}",
            result.stdout, result.stderr
        ),
    };

    ctx.aam.set_belief(
        format!("{}{}:completed", belief_keys::EXC_PREFIX, node.id),
        Value::String(output.clone()),
        transition_label,
    );

    Ok(Value::String(output))
}
