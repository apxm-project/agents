//! Template resolution for workflow parameter and output substitution.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Resolve {{step_id.output}} and {{PARAM_NAME}} in a template string.
///
/// - `{{step_id.output}}` is replaced with the value from `step_outputs[step_id]`
/// - `{{PARAM_NAME}}` is replaced with the value from `params[PARAM_NAME]`
/// - Unknown references are replaced with empty string
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use apxm_runtime::workflow::template::resolve;
///
/// let mut step_outputs = HashMap::new();
/// step_outputs.insert("architect".to_string(), "Use MVC pattern".to_string());
///
/// let mut params = HashMap::new();
/// params.insert("TASK".to_string(), "Add auth".to_string());
///
/// let result = resolve(
///     "Task: {{TASK}}, Plan: {{architect.output}}",
///     &step_outputs,
///     &params
/// );
/// assert_eq!(result, "Task: Add auth, Plan: Use MVC pattern");
/// ```
pub fn resolve(
    template: &str,
    step_outputs: &HashMap<String, String>,
    params: &HashMap<String, String>,
) -> String {
    static TEMPLATE_RE: OnceLock<Regex> = OnceLock::new();
    let re = TEMPLATE_RE.get_or_init(|| Regex::new(r"\{\{([^}]+)\}\}").unwrap());

    re.replace_all(template, |caps: &regex::Captures| {
        let key = caps.get(1).unwrap().as_str().trim();

        // Check if it's a step output reference (e.g., "architect.output")
        if let Some((step_id, field)) = key.split_once('.') {
            if field == "output" {
                if let Some(value) = step_outputs.get(step_id) {
                    return value.clone();
                }
            }
            // Unknown step or field - return empty string
            return String::new();
        }

        // Check if it's a parameter reference
        if let Some(value) = params.get(key) {
            return value.clone();
        }

        // Unknown reference - return empty string
        String::new()
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_param() {
        let mut params = HashMap::new();
        params.insert("TASK".to_string(), "Add feature X".to_string());

        let result = resolve("Task: {{TASK}}", &HashMap::new(), &params);
        assert_eq!(result, "Task: Add feature X");
    }

    #[test]
    fn test_resolve_step_output() {
        let mut outputs = HashMap::new();
        outputs.insert("architect".to_string(), "Use MVC".to_string());

        let result = resolve("Plan: {{architect.output}}", &outputs, &HashMap::new());
        assert_eq!(result, "Plan: Use MVC");
    }

    #[test]
    fn test_resolve_multiple() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), "val_a".to_string());
        outputs.insert("b".to_string(), "val_b".to_string());

        let mut params = HashMap::new();
        params.insert("PARAM".to_string(), "param_val".to_string());

        let result = resolve("{{PARAM}}, {{a.output}}, {{b.output}}", &outputs, &params);
        assert_eq!(result, "param_val, val_a, val_b");
    }

    #[test]
    fn test_resolve_unknown_reference() {
        let result = resolve("{{unknown}}", &HashMap::new(), &HashMap::new());
        assert_eq!(result, "");
    }

    #[test]
    fn test_resolve_unknown_step() {
        let result = resolve("{{unknown.output}}", &HashMap::new(), &HashMap::new());
        assert_eq!(result, "");
    }

    #[test]
    fn test_resolve_no_templates() {
        let result = resolve("plain text", &HashMap::new(), &HashMap::new());
        assert_eq!(result, "plain text");
    }

    #[test]
    fn test_resolve_whitespace() {
        let mut params = HashMap::new();
        params.insert("VAR".to_string(), "value".to_string());

        let result = resolve("{{ VAR }}", &HashMap::new(), &params);
        assert_eq!(result, "value");
    }
}
