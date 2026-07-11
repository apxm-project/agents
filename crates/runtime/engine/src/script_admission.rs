//! Shared script-artifact admission policy for author-supplied `python_tools`
//! and `typescript_tools` sections.
//!
//! One policy, two languages: an artifact-local script section (Python or
//! TypeScript) is admitted only when the operator has explicitly asserted
//! BOTH `APXM_TRUST_SCRIPT_ARTIFACTS` (trust) and `APXM_SANDBOX_SCRIPTS`
//! (isolation). Server, driver, and Runtime all enforce this fail-closed gate.

use apxm_core::constants::env::{APXM_SANDBOX_SCRIPTS, APXM_TRUST_SCRIPT_ARTIFACTS};

/// True only when the operator has explicitly trusted script artifacts
/// (Python or TypeScript) AND required OS-level worker sandboxing. Fail
/// closed: absent either var, script sections must be rejected wherever
/// this gate is consulted.
pub fn script_artifacts_trusted() -> bool {
    std::env::var_os(APXM_TRUST_SCRIPT_ARTIFACTS).is_some() && script_sandbox_required()
}

/// Whether the operator requires script workers (Python or TypeScript) to
/// run under an OS-isolating sandbox backend. Shared by both languages —
/// there is one language-neutral sandbox opt-in for both bridges.
pub fn script_sandbox_required() -> bool {
    std::env::var_os(APXM_SANDBOX_SCRIPTS).is_some()
}

/// Test-only support shared by every test module in this crate that needs to
/// mutate the process-wide script admission variables. The test runner
/// runs all unit tests for a crate in one process across multiple threads, so
/// unsynchronized env mutation across e.g. `script_admission::tests` and
/// `runtime::tests` would race; every such test must acquire [`ENV_LOCK`]
/// first.
#[cfg(test)]
pub(crate) mod test_support {
    use apxm_core::constants::env::{APXM_SANDBOX_SCRIPTS, APXM_TRUST_SCRIPT_ARTIFACTS};
    use std::sync::Mutex;

    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that clears both admission env vars on drop so a panicking
    /// test can't leak trust/sandbox state into the next test that acquires
    /// [`ENV_LOCK`].
    pub(crate) struct EnvGuard;

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // Safety: caller holds `ENV_LOCK` for the lifetime of this guard,
            // so no other thread observes these vars concurrently.
            #[allow(unsafe_code)]
            unsafe {
                std::env::remove_var(APXM_TRUST_SCRIPT_ARTIFACTS);
                std::env::remove_var(APXM_SANDBOX_SCRIPTS);
            }
        }
    }

    /// Set both vars (or neither/one) for the current test. Must be called
    /// while holding `ENV_LOCK`.
    pub(crate) fn set_vars(trust: bool, sandbox: bool) {
        #[allow(unsafe_code)]
        unsafe {
            if trust {
                std::env::set_var(APXM_TRUST_SCRIPT_ARTIFACTS, "1");
            } else {
                std::env::remove_var(APXM_TRUST_SCRIPT_ARTIFACTS);
            }
            if sandbox {
                std::env::set_var(APXM_SANDBOX_SCRIPTS, "1");
            } else {
                std::env::remove_var(APXM_SANDBOX_SCRIPTS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{ENV_LOCK, EnvGuard, set_vars};
    use super::*;

    #[test]
    fn fails_closed_with_no_vars() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        set_vars(false, false);
        assert!(!script_artifacts_trusted());
        assert!(!script_sandbox_required());
    }

    #[test]
    fn fails_closed_with_trust_only() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        set_vars(true, false);
        assert!(!script_artifacts_trusted());
    }

    #[test]
    fn fails_closed_with_sandbox_only() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        set_vars(false, true);
        assert!(!script_artifacts_trusted());
        assert!(script_sandbox_required());
    }

    #[test]
    fn trusted_with_both_vars() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        set_vars(true, true);
        assert!(script_artifacts_trusted());
    }
}
