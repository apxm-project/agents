use std::env;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let schema =
        manifest_dir.join("../../../contracts/schemas/apxm.committed-native-model-usage.json");
    println!("cargo:rerun-if-changed={}", schema.display());
    let bytes = fs::read(&schema).expect("read committed native model usage schema");
    let digest = format!("sha256:{:x}", Sha256::digest(bytes));
    println!("cargo:rustc-env=APXM_COMMITTED_NATIVE_MODEL_USAGE_SCHEMA_DIGEST={digest}");
}
