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
