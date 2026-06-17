//! Bind-address helpers for auth policy and deployment posture.

use std::net::{IpAddr, SocketAddr};

use apxm_driver::ServerAuthConfig;

/// Returns true when the socket listens only on loopback interfaces.
pub(crate) fn is_loopback_addr(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Effective bearer-auth requirement for the resolved listen address.
///
/// Non-loopback binds are always fail-closed (auth on) regardless of the
/// config default. Loopback honors `require_auth` so local dev and contract
/// tests stay unauthenticated unless explicitly enabled.
pub(crate) fn effective_require_auth(bind: &SocketAddr, config: &ServerAuthConfig) -> bool {
    if is_loopback_addr(bind) {
        config.require_auth
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_honors_config_default_off() {
        let bind = "127.0.0.1:18800".parse().expect("addr");
        let config = ServerAuthConfig::default();
        assert!(!effective_require_auth(&bind, &config));
    }

    #[test]
    fn loopback_honors_explicit_on() {
        let bind = "127.0.0.1:18800".parse().expect("addr");
        let config = ServerAuthConfig {
            require_auth: true,
            ..ServerAuthConfig::default()
        };
        assert!(effective_require_auth(&bind, &config));
    }

    #[test]
    fn non_loopback_always_requires_auth() {
        let bind = "0.0.0.0:18800".parse().expect("addr");
        let config = ServerAuthConfig::default();
        assert!(effective_require_auth(&bind, &config));
    }

    #[test]
    fn detects_ipv6_loopback() {
        let bind = "[::1]:18800".parse().expect("addr");
        assert!(is_loopback_addr(&bind));
    }
}
