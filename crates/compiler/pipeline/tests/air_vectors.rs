//! Compile contract AIR vector fixtures.

use apxm_compiler::{Context, Pipeline};
use std::path::PathBuf;

fn contracts_air_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../../contracts/vectors/air")
}

#[test]
fn contract_air_vectors_compile() {
    let air_dir = contracts_air_dir();
    let manifest_path = air_dir.join("manifest.json");
    if !manifest_path.exists() {
        eprintln!(
            "skipping: contracts air vectors not present at {}",
            air_dir.display()
        );
        return;
    }
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    let fixtures = manifest["fixtures"].as_array().expect("fixtures array");
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::new(&context);
    for fixture in fixtures {
        let file = fixture["file"].as_str().expect("file");
        let air_path = air_dir.join(file);
        let air = std::fs::read_to_string(&air_path).expect("read air fixture");
        pipeline
            .compile(&air)
            .unwrap_or_else(|e| panic!("compile failed for {}: {e}", file));
    }
}
