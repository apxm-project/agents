//! Embed the checked-in contract-common schema path for frontend codegen.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let common_schema = manifest_dir
        .join("../../machine/program/tests/fixtures/contracts/apxm.contract-common.v1.json");
    println!("cargo:rerun-if-changed={}", common_schema.display());
    println!(
        "cargo:rustc-env=APXM_CONTRACT_COMMON_SCHEMA_PATH={}",
        common_schema.display()
    );
}
