use crate::common::agents_root;

/// The SHA-256 content address of a published contract document's exact bytes,
/// in the `sha256:<hex>` form the Port Contracts use.
#[must_use]
pub(crate) fn contract_file_digest(relative: &str) -> String {
    use sha2::{Digest, Sha256};

    let path = agents_root().join("contracts").join(relative);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Whether a schema is published at `contracts/schemas/<schema_id>.json`.
#[must_use]
pub(crate) fn contract_file_exists(relative: &str) -> bool {
    agents_root().join("contracts").join(relative).is_file()
}
