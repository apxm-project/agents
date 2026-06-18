fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let spec_path = manifest_dir.join("openapi/openapi-session-v1.yaml");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let spec_file = std::fs::File::open(&spec_path).unwrap_or_else(|error| {
        panic!(
            "failed to open OpenAPI spec {}: {error}",
            spec_path.display()
        )
    });
    let spec: openapiv3::OpenAPI =
        serde_yaml::from_reader(spec_file).expect("OpenAPI spec parses as YAML");

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .expect("progenitor generates client tokens");
    let ast = syn::parse2(tokens).expect("generated tokens parse as Rust syntax");
    let content = prettyplease::unparse(&ast);

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("codegen.rs"), content).expect("write generated client");
}
