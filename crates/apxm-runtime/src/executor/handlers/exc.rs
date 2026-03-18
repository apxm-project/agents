//! EXC operation - Execute code in sandbox

use super::{ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_optional_u64_attribute};
use crate::sandbox::policy::SandboxPolicy;
use crate::sandbox::process::ProcessSandbox;
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Extract code from 'code' attribute, or fall back to first input
    let code = get_optional_string_attribute(node, graph_attrs::CODE)?
        .or_else(|| inputs.first().and_then(|v| v.as_str().map(|s| s.to_string())))
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: "EXC requires 'code' attribute or input".to_string(),
        })?;

    let interpreter = get_optional_string_attribute(node, "interpreter")?
        .unwrap_or_else(|| "bash".to_string());

    let timeout_secs = get_optional_u64_attribute(node, "timeout")?.unwrap_or(30);

    let policy = SandboxPolicy {
        timeout: std::time::Duration::from_secs(timeout_secs),
        ..SandboxPolicy::default()
    };

    let sandbox = ProcessSandbox::new(policy);

    // AAM transition
    let transition_label =
        crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("exc:{}:started", node.id),
        Value::String(format!("Executing {} script", interpreter)),
        transition_label.clone(),
    );

    let result = sandbox.execute_script(&interpreter, &code).await.map_err(|e| {
        apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Sandbox execution failed: {e}"),
        }
    })?;

    if result.timed_out {
        ctx.aam.set_belief(
            format!("exc:{}:timed_out", node.id),
            Value::Bool(true),
            transition_label,
        );
        return Err(apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Script execution timed out after {timeout_secs} seconds"),
        });
    }

    let output = if result.exit_code != 0 {
        format!(
            "Exit code: {}\nStdout: {}\nStderr: {}",
            result.exit_code, result.stdout, result.stderr
        )
    } else {
        result.stdout
    };

    ctx.aam.set_belief(
        format!("exc:{}:completed", node.id),
        Value::String(output.clone()),
        transition_label,
    );

    Ok(Value::String(output))
}
