//! Canonical Event HTTP projection. Loopback by default.

use apxm_kernel::event_api::EventHttpMethod;

/// Bind address policy for Event HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindPolicy {
    /// 127.0.0.1 only.
    Loopback,
    /// Non-loopback requires the accepted security ADR profile.
    NonLoopback { security_profile_complete: bool },
}

impl BindPolicy {
    /// Non-loopback is impossible unless the security profile is complete.
    #[must_use]
    pub fn allowed(self) -> bool {
        match self {
            Self::Loopback => true,
            Self::NonLoopback {
                security_profile_complete,
            } => security_profile_complete,
        }
    }
}

/// Map HTTP method names onto the frozen Event HTTP paths.
#[must_use]
pub fn route(method: EventHttpMethod) -> &'static str {
    method.path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_allowed_and_nonloopback_is_not_by_default() {
        assert!(BindPolicy::Loopback.allowed());
        assert!(
            !BindPolicy::NonLoopback {
                security_profile_complete: false
            }
            .allowed()
        );
    }

    #[test]
    fn fulfill_path_is_frozen() {
        assert_eq!(route(EventHttpMethod::Fulfill), "/v1/events/fulfill");
    }
}
