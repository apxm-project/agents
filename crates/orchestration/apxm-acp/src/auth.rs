use std::env;

/// Resolve an ACP authentication credential for a given method ID.
///
/// Priority order (matching ACPX's exact resolution):
/// 1. Exact env var: `{method_id}`
/// 2. Normalized: `ACPX_AUTH_{NORM}`
/// 3. Normalized: `{NORM}`
///
/// Normalization: trim, replace `[^a-zA-Z0-9]` with `_`, strip leading/trailing `_`, uppercase.
pub fn resolve_auth_credential(method_id: &str) -> Option<String> {
    // 1. Exact env var
    if let Ok(val) = env::var(method_id) {
        if !val.is_empty() {
            return Some(val);
        }
    }

    let norm = normalize(method_id);

    // 2. ACPX_AUTH_{NORM}
    let acpx_key = format!("{}{norm}", crate::constants::auth::ENV_PREFIX);
    if let Ok(val) = env::var(&acpx_key) {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // 3. {NORM}
    if let Ok(val) = env::var(&norm) {
        if !val.is_empty() {
            return Some(val);
        }
    }

    None
}

/// Normalize a method ID to an environment variable name.
fn normalize(s: &str) -> String {
    let replaced: String = s
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = replaced.trim_matches('_');
    trimmed.to_ascii_uppercase()
}

