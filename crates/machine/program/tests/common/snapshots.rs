use serde_json::Value;

use crate::common::{agents_root, load_json};

/// Load an immutable external-owner schema snapshot by its published path.
#[must_use]
pub(crate) fn load_contract_snapshot(relative: &str) -> Value {
    let filename = std::path::Path::new(relative)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("contract snapshot filename");
    let filename = if filename.starts_with("apxm.") {
        filename.to_owned()
    } else {
        format!("apxm.{filename}")
    };
    load_json(
        agents_root()
            .join("crates/machine/program/tests/fixtures/contracts")
            .join(filename),
    )
}
