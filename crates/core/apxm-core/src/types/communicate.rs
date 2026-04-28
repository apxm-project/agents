//! Typed dispatch surface for the COMMUNICATE operation's protocol attribute.

use std::fmt;
use std::str::FromStr;

/// Transport selected by a COMMUNICATE node's `protocol` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommunicateProtocol {
    /// In-process sub-flow execution via FlowRegistry.
    Local,
    /// HTTP POST to an external APXM agent's `/v1/receive` endpoint.
    Http,
    /// HTTPS variant of the HTTP protocol.
    Https,
    /// ACP JSON-RPC over stdio to a spawned agent subprocess.
    Acp,
    /// Fan-out to ALL registered agents in parallel.
    Broadcast,
}

impl CommunicateProtocol {
    /// Wire string used in `.air` attributes and metric labels.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Http => "http",
            Self::Https => "https",
            Self::Acp => "acp",
            Self::Broadcast => "broadcast",
        }
    }

    /// True if this protocol routes over the HTTP family (http, https).
    pub const fn is_http_family(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }
}

impl fmt::Display for CommunicateProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CommunicateProtocol {
    type Err = UnknownProtocol;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "acp" => Ok(Self::Acp),
            "broadcast" => Ok(Self::Broadcast),
            _ => Err(UnknownProtocol(s.to_string())),
        }
    }
}

/// Returned by `CommunicateProtocol::from_str` for unrecognized values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProtocol(pub String);

impl fmt::Display for UnknownProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown communicate protocol: {}", self.0)
    }
}

impl std::error::Error for UnknownProtocol {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_variants() {
        for v in [
            CommunicateProtocol::Local,
            CommunicateProtocol::Http,
            CommunicateProtocol::Https,
            CommunicateProtocol::Acp,
            CommunicateProtocol::Broadcast,
        ] {
            assert_eq!(v.as_str().parse::<CommunicateProtocol>().unwrap(), v);
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        let err = "websocket".parse::<CommunicateProtocol>().unwrap_err();
        assert_eq!(err.0, "websocket");
    }

    #[test]
    fn as_str_returns_canonical_wire_strings() {
        assert_eq!(CommunicateProtocol::Local.as_str(), "local");
        assert_eq!(CommunicateProtocol::Http.as_str(), "http");
        assert_eq!(CommunicateProtocol::Https.as_str(), "https");
        assert_eq!(CommunicateProtocol::Acp.as_str(), "acp");
        assert_eq!(CommunicateProtocol::Broadcast.as_str(), "broadcast");
    }

    #[test]
    fn is_http_family_classifies_correctly() {
        assert!(CommunicateProtocol::Http.is_http_family());
        assert!(CommunicateProtocol::Https.is_http_family());
        assert!(!CommunicateProtocol::Local.is_http_family());
        assert!(!CommunicateProtocol::Acp.is_http_family());
        assert!(!CommunicateProtocol::Broadcast.is_http_family());
    }
}
