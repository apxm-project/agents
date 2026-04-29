//! vLLM-specific node attribute names and accepted values.
//!
//! These constants live in the vLLM backend crate (not in `apxm-ais` or
//! `apxm-core`) because they are meaningful only when the resolved route's
//! backend is vLLM. The AIS contract stays backend-agnostic; vLLM consumers
//! import from here.
//!
//! Workflow authors are not expected to set these directly. The runtime
//! resolves vLLM-specific behavior from explicit node attributes when set,
//! otherwise from the `APXM_VLLM_CACHE_SALT` env var (which the benchmark
//! harness wires per child process), otherwise from production defaults.

/// Node attribute name carrying the vLLM cache-salt selector.
///
/// Optional. When unset, vLLM's prefix cache is reused across executions
/// (production default). When set, the runtime resolves the selector to a
/// concrete salt string and forwards it as `extra_body.cache_salt` on the
/// vLLM HTTP request.
pub const CACHE_SALT_ATTR: &str = "vllm_cache_salt";

/// Sentinel selectors recognized by the runtime. Any other string value is
/// treated as a literal salt and passed through as-is.
pub const CACHE_SALT_EXECUTION: &str = "execution";
pub const CACHE_SALT_EXECUTION_ID: &str = "execution_id";
pub const CACHE_SALT_GRAPH_EXECUTION: &str = "graph_execution";

/// Sentinel value (case-insensitive) recognized as "no salting requested".
/// An empty string is also treated this way.
pub const CACHE_SALT_NONE_LITERAL: &str = "none";

/// Env var the benchmark harness sets to request per-execution salting
/// without the workflow declaring any backend-specific attribute.
pub const CACHE_SALT_ENV_VAR: &str = "APXM_VLLM_CACHE_SALT";

/// Resolution chain for the cache-salt selector:
///   explicit node attribute  →  `APXM_VLLM_CACHE_SALT` env var  →  None.
///
/// Returns the caller-supplied selector string. Final substitution
/// (`"execution"` → execution_id, `"graph_execution"` → `{graph_id}:{execution_id}`)
/// is the runtime's responsibility because it requires `ExecutionContext`.
///
/// Empty / `"none"` (case-insensitive) values yield `None` so the caller
/// skips applying any salt.
pub fn resolved_cache_salt_selector(node_attr: Option<&str>) -> Option<String> {
    let candidate = node_attr
        .map(str::to_owned)
        .or_else(|| std::env::var(CACHE_SALT_ENV_VAR).ok());
    let value = candidate?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(CACHE_SALT_NONE_LITERAL) {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to run a closure with the env var temporarily set to `value`.
    /// Cleans up regardless of the closure's outcome. Tests in this module
    /// must not run in parallel because they mutate process-global env state;
    /// we serialize them via `#[test]`-level `serial_test` if it ever lands,
    /// but for now they each save/restore.
    use std::sync::Mutex;

    /// Serializes env-mutating tests in this module. Tests in other modules
    /// don't touch CACHE_SALT_ENV_VAR, so process-wide contention is bounded
    /// to this set of tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[allow(unsafe_code)]
    fn with_env<F: FnOnce()>(value: Option<&str>, f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(CACHE_SALT_ENV_VAR).ok();
        // SAFETY: edition 2024 marks env mutators unsafe because they race
        // with concurrent reads in multi-threaded code. Tests in this module
        // serialize their own access via save/restore.
        unsafe {
            match value {
                Some(v) => std::env::set_var(CACHE_SALT_ENV_VAR, v),
                None => std::env::remove_var(CACHE_SALT_ENV_VAR),
            }
        }
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var(CACHE_SALT_ENV_VAR, v),
                None => std::env::remove_var(CACHE_SALT_ENV_VAR),
            }
        }
    }

    #[test]
    fn explicit_attr_wins_over_env() {
        with_env(Some("execution"), || {
            let resolved = resolved_cache_salt_selector(Some("graph_execution"));
            assert_eq!(resolved.as_deref(), Some("graph_execution"));
        });
    }

    #[test]
    fn env_used_when_attr_absent() {
        with_env(Some("execution"), || {
            assert_eq!(
                resolved_cache_salt_selector(None).as_deref(),
                Some("execution")
            );
        });
    }

    #[test]
    fn neither_attr_nor_env_yields_none() {
        with_env(None, || {
            assert_eq!(resolved_cache_salt_selector(None), None);
        });
    }

    #[test]
    fn none_literal_treated_as_unset() {
        with_env(None, || {
            assert_eq!(resolved_cache_salt_selector(Some("none")), None);
            assert_eq!(resolved_cache_salt_selector(Some("None")), None);
            assert_eq!(resolved_cache_salt_selector(Some("")), None);
            assert_eq!(resolved_cache_salt_selector(Some("   ")), None);
        });
    }

    #[test]
    fn literal_salt_passed_through() {
        with_env(None, || {
            assert_eq!(
                resolved_cache_salt_selector(Some("custom-tenant-42")).as_deref(),
                Some("custom-tenant-42"),
            );
        });
    }

    #[test]
    fn empty_env_treated_as_unset() {
        with_env(Some(""), || {
            assert_eq!(resolved_cache_salt_selector(None), None);
        });
    }
}
