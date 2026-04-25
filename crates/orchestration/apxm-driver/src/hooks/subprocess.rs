use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use apxm_core::types::TimingBreakdown;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::{ExecutionEventEmitter, TokenUsageSummary};

use crate::config::{HookConfig, HookEvent};

pub struct SubprocessHookEmitter {
    hooks: Vec<HookConfig>,
}

impl SubprocessHookEmitter {
    pub fn new(hooks: Vec<HookConfig>) -> Self {
        Self { hooks }
    }

    fn fire(&self, event: HookEvent, fields: impl IntoIterator<Item = (&'static str, String)>) {
        let mut values = HashMap::from([("event".to_string(), event.as_str().to_string())]);
        values.extend(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value)),
        );

        for hook in self.hooks.iter().filter(|hook| hook.event == event) {
            let rendered = render_command(&hook.command, &values);
            let shell = hook.shell.as_deref().unwrap_or("sh").to_string();
            let event_name = event.as_str().to_string();
            let command_for_log = rendered.clone();

            match Command::new(&shell).arg("-c").arg(&rendered).spawn() {
                Ok(mut child) => {
                    std::thread::spawn(move || match child.wait() {
                        Ok(status) if !status.success() => {
                            tracing::warn!(
                                event = %event_name,
                                command = %command_for_log,
                                status = %status,
                                "Execution hook command exited non-zero; continuing"
                            );
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(
                                event = %event_name,
                                command = %command_for_log,
                                error = %error,
                                "Execution hook command wait failed; continuing"
                            );
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        event = %event_name,
                        shell = %shell,
                        command = %command_for_log,
                        error = %error,
                        "Execution hook command failed to spawn; continuing"
                    );
                }
            }
        }
    }
}

impl ExecutionEventEmitter for SubprocessHookEmitter {
    fn emit_llm_token(&self, _content: &str) {}

    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>) {
        self.fire(
            HookEvent::ToolStart,
            [
                ("tool_name", name.to_string()),
                ("args_json", serde_json::to_string(args).unwrap_or_default()),
            ],
        );
    }

    fn emit_tool_end(&self, name: &str, result: &Value) {
        self.fire(
            HookEvent::ToolEnd,
            [
                ("tool_name", name.to_string()),
                (
                    "result_json",
                    serde_json::to_string(result).unwrap_or_default(),
                ),
            ],
        );
    }

    fn emit_graph_start(&self, execution_id: &str, node_count: usize) {
        self.fire(
            HookEvent::GraphStart,
            [
                ("execution_id", execution_id.to_string()),
                ("node_count", node_count.to_string()),
            ],
        );
    }

    fn emit_graph_end(&self, execution_id: &str, node_count: usize, success: bool) {
        self.fire(
            HookEvent::GraphEnd,
            [
                ("execution_id", execution_id.to_string()),
                ("node_count", node_count.to_string()),
                ("success", success.to_string()),
            ],
        );
    }

    fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
        self.fire(
            HookEvent::NodeStart,
            [
                ("node_id", node_id.to_string()),
                ("op_type", op_type.to_string()),
            ],
        );
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        duration: Duration,
        success: bool,
        tokens: Option<TokenUsageSummary>,
        timing: Option<TimingBreakdown>,
    ) {
        let mut fields = vec![
            ("node_id", node_id.to_string()),
            ("op_type", op_type.to_string()),
            ("duration_ms", duration.as_millis().to_string()),
            ("success", success.to_string()),
        ];
        if let Some(tokens) = tokens {
            fields.push(("input_tokens", tokens.input_tokens.to_string()));
            fields.push(("output_tokens", tokens.output_tokens.to_string()));
        }
        if let Some(timing) = timing {
            fields.push(("prefill_ms", timing.prefill_ms.to_string()));
            fields.push(("decode_ms", timing.decode_ms.to_string()));
        }
        self.fire(
            if success {
                HookEvent::NodeComplete
            } else {
                HookEvent::NodeError
            },
            fields,
        );
    }
}

fn render_command(command: &str, values: &HashMap<String, String>) -> String {
    let mut rendered = command.to_string();
    for (key, value) in values {
        let token = format!("{{{{{key}}}}}");
        rendered = rendered.replace(&token, &shell_escape(value));
    }
    rendered
}

fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use tempfile::tempdir;

    fn wait_for_file(path: &std::path::Path) -> String {
        for _ in 0..50 {
            if let Ok(contents) = fs::read_to_string(path) {
                return contents;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {}", path.display());
    }

    #[test]
    fn node_complete_hook_writes_rendered_values() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("hook.out");
        let emitter = SubprocessHookEmitter::new(vec![HookConfig {
            event: HookEvent::NodeComplete,
            command: format!("printf %s {{{{node_id}}}} > {}", output.display()),
            shell: None,
        }]);

        emitter.emit_operation_end(
            42,
            AISOperationType::Ask,
            Duration::from_millis(7),
            true,
            None,
            None,
        );

        assert_eq!(wait_for_file(&output), "42");
    }

    #[test]
    fn failing_hook_is_non_fatal() {
        let emitter = SubprocessHookEmitter::new(vec![HookConfig {
            event: HookEvent::NodeError,
            command: "exit 7".to_string(),
            shell: None,
        }]);

        emitter.emit_operation_end(
            9,
            AISOperationType::Ask,
            Duration::from_millis(1),
            false,
            None,
            None,
        );
    }
}
