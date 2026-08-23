use crate::published::published_contract_files;

/// Every conformance vector file published under `contracts/vectors/`.
#[must_use]
pub(crate) fn published_vector_files() -> Vec<String> {
    published_contract_files("vectors")
}
