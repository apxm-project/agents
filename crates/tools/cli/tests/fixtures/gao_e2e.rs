//! Cross-repo note: server-side gao package compile coverage already exists.
//!
//! See `workspace/server/crates/server/src/tests.rs`:
//! - `agent_compile_endpoint_compiles_real_gao_via_agents_cli`
//! - `agent_package_compile_endpoint_503_when_agents_cli_not_spawnable`
//!
//! Agents-owned end-to-end fixture tests live in
//! `examples/agents/gao/tests/e2e_manifest.rs` and run via:
//!
//! ```text
//! cd workspace/agents
//! cargo test --features driver -p apxm-cli 'agent:: gao'
//! ```
//!
//! A future `POST /v1/agents/{id}/sessions` handler should reuse the
//! same declarative recv-loop AIR contract validated here.

#[test]
fn server_gao_e2e_documentation_stub() {
    // Intentionally empty: documents where cross-repo server tests live.
}
