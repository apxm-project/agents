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

