//! Emit AIR from frontend graph JSON through the Rust AIR printer.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use apxm_compiler::{AirModule, AirProgram, FrontendGraph};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FrontendAirInput {
    Graph(FrontendGraph),
    Graphs(Vec<FrontendGraph>),
    Program { modules: Vec<FrontendGraph> },
}

pub fn emit_air_command(input: Option<PathBuf>) -> Result<()> {
    let json = read_graph_json(input.as_ref())?;
    let parsed: FrontendAirInput =
        serde_json::from_str(&json).context("failed to parse frontend graph JSON")?;
    let air = emit_air_from_input(parsed)?;
    print!("{air}");
    Ok(())
}

fn read_graph_json(input: Option<&PathBuf>) -> Result<String> {
    if let Some(path) = input {
        return std::fs::read_to_string(path).with_context(|| {
            format!("failed to read frontend graph JSON from {}", path.display())
        });
    }

    let mut json = String::new();
    std::io::stdin()
        .read_to_string(&mut json)
        .context("failed to read frontend graph JSON from stdin")?;
    if json.trim().is_empty() {
        anyhow::bail!("expected frontend graph JSON on stdin");
    }
    Ok(json)
}

fn emit_air_from_input(input: FrontendAirInput) -> Result<String> {
    match input {
        FrontendAirInput::Graph(graph) => {
            let module = graph.to_air_module()?;
            module.to_air().map_err(Into::into)
        }
        FrontendAirInput::Graphs(graphs) | FrontendAirInput::Program { modules: graphs } => {
            let modules = graphs
                .iter()
                .map(FrontendGraph::to_air_module)
                .collect::<Result<Vec<AirModule>, _>>()?;
            AirProgram::new(modules).to_air().map_err(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn emits_air_from_single_frontend_graph_json() {
        let input: FrontendAirInput = serde_json::from_value(json!({
            "name": "hello",
            "nodes": [
                {"id": 1, "name": "ask", "op": "ASK", "attributes": {"template_str": "hi"}}
            ],
            "edges": [],
            "parameters": [],
            "metadata": {"is_entry": true}
        }))
        .expect("graph json parses");

        let air = emit_air_from_input(input).expect("air emits");
        assert!(air.contains("module {"));
        assert!(air.contains("func.func @hello"));
        assert!(air.contains("ais.ask"));
    }

    #[test]
    fn emits_air_from_multiple_frontend_graphs_json() {
        let input: FrontendAirInput = serde_json::from_value(json!([
            {
                "name": "first",
                "nodes": [
                    {"id": 1, "name": "ask", "op": "ASK", "attributes": {"template_str": "one"}}
                ],
                "edges": [],
                "parameters": [],
                "metadata": {"is_entry": true}
            },
            {
                "name": "second",
                "nodes": [
                    {"id": 1, "name": "think", "op": "THINK", "attributes": {"template_str": "two"}}
                ],
                "edges": [],
                "parameters": [],
                "metadata": {"is_entry": false}
            }
        ]))
        .expect("program json parses");

        let air = emit_air_from_input(input).expect("air emits");
        assert!(air.contains("func.func @first"));
        assert!(air.contains("func.func @second"));
    }

    /// Proves the `FrontendEdge`/`FrontendParameter` serde `alias` fields
    /// (`frontend_graph.rs:40-53`) converge, not just parse: the same
    /// logical graph, wire-shaped once the way Python's historical
    /// `from_id`/`to_id` field names would produce it and once the way
    /// TypeScript's `from`/`to`/`typeName` shape produces it, must emit
    /// byte-identical AIR through the single printer. A silent field-name
    /// drift between frontends would otherwise parse into two structurally
    /// different `FrontendGraph` values without either side erroring.
    #[test]
    fn python_and_typescript_wire_shapes_emit_identical_air() {
        let python_wire: FrontendAirInput = serde_json::from_value(json!({
            "name": "wire_shape_parity",
            "nodes": [
                {
                    "id": 1,
                    "name": "coder",
                    "op": "AGENT",
                    "attributes": {"profile": "codex", "prompt": "Fix it"}
                },
                {"id": 2, "name": "out", "op": "RETURN", "attributes": {}}
            ],
            "edges": [
                {"from_id": 1, "to_id": 2, "dependency": "Data"}
            ],
            "parameters": [{"name": "topic", "type_name": "str"}],
            "metadata": {"is_entry": true}
        }))
        .expect("python wire-shape json parses");

        let typescript_wire: FrontendAirInput = serde_json::from_value(json!({
            "name": "wire_shape_parity",
            "nodes": [
                {
                    "id": 1,
                    "name": "coder",
                    "op": "AGENT",
                    "attributes": {"profile": "codex", "prompt": "Fix it"}
                },
                {"id": 2, "name": "out", "op": "RETURN", "attributes": {}}
            ],
            "edges": [
                {"from": 1, "to": 2, "dependency": "Data"}
            ],
            "parameters": [{"name": "topic", "typeName": "str"}],
            "metadata": {"is_entry": true}
        }))
        .expect("typescript wire-shape json parses");

        let python_air = emit_air_from_input(python_wire).expect("python wire-shape graph emits");
        let typescript_air =
            emit_air_from_input(typescript_wire).expect("typescript wire-shape graph emits");
        assert_eq!(
            python_air, typescript_air,
            "Python (`from_id`/`to_id`, `type_name`) and TypeScript (`from`/`to`, \
             `typeName`) wire shapes for the same logical graph must emit byte-identical AIR"
        );
    }

    /// Drift detector: an unrecognized top-level field must be rejected via
    /// `#[serde(deny_unknown_fields)]`, not silently ignored. Guards against
    /// a frontend adding an attribute that the shared DTO does not know
    /// about, which would otherwise surface as silent missing behavior
    /// instead of a loud parse failure.
    #[test]
    fn rejects_frontend_graph_with_unknown_top_level_field() {
        // Deserialize straight to `FrontendGraph` (not the untagged
        // `FrontendAirInput`) so the failure surfaces `deny_unknown_fields`'s
        // specific field-name message rather than the untagged enum's
        // generic "did not match any variant" text.
        let err = serde_json::from_value::<FrontendGraph>(json!({
            "name": "drifted",
            "nodes": [],
            "edges": [],
            "parameters": [],
            "metadata": {},
            "unexpected_extra_field": true
        }))
        .expect_err("unknown top-level field must be rejected, not ignored");
        assert!(err.to_string().contains("unexpected_extra_field"));

        // The same drift must also be caught through the real CLI entry
        // point (`FrontendAirInput`, as `apxm emit-air` deserializes it),
        // even though the untagged enum's error text is generic.
        assert!(
            serde_json::from_value::<FrontendAirInput>(json!({
                "name": "drifted",
                "nodes": [],
                "edges": [],
                "parameters": [],
                "metadata": {},
                "unexpected_extra_field": true
            }))
            .is_err(),
            "apxm emit-air's input parser must also reject the unknown field"
        );
    }

    /// Fixtures under `tests/fixtures/frontend_graph_parity/` are the shared
    /// vectors this CLI's `emit_air_from_input`, the Python
    /// `test_air_parity.py` pytest suite, and the TypeScript
    /// `emit-air.test.ts` vitest suite all diff against. This test is the
    /// single source of truth that regenerates/guards the checked-in
    /// `*.golden.air` files against the canonical printer, so Python/TS
    /// parity tests always compare against real Rust output, not a
    /// hand-maintained copy.
    fn assert_fixture_matches_golden(fixture_json: &str, golden_air: &str) {
        let parsed: FrontendAirInput =
            serde_json::from_str(fixture_json).expect("fixture json parses");
        let air = emit_air_from_input(parsed).expect("fixture graph emits AIR");
        assert_eq!(
            air, golden_air,
            "fixture output drifted from checked-in golden .air"
        );
    }

    #[test]
    fn ask_flow_fixture_matches_golden_air() {
        assert_fixture_matches_golden(
            include_str!("../../tests/fixtures/frontend_graph_parity/ask_flow.json"),
            include_str!("../../tests/fixtures/frontend_graph_parity/ask_flow.golden.air"),
        );
    }

    #[test]
    fn parametrized_flow_fixture_matches_golden_air() {
        assert_fixture_matches_golden(
            include_str!("../../tests/fixtures/frontend_graph_parity/parametrized_flow.json"),
            include_str!("../../tests/fixtures/frontend_graph_parity/parametrized_flow.golden.air"),
        );
    }

    #[test]
    fn profiled_agent_flow_fixture_matches_golden_air() {
        assert_fixture_matches_golden(
            include_str!("../../tests/fixtures/frontend_graph_parity/profiled_agent_flow.json"),
            include_str!(
                "../../tests/fixtures/frontend_graph_parity/profiled_agent_flow.golden.air"
            ),
        );
    }

    #[test]
    fn multi_flow_conversational_fixture_matches_golden_air() {
        assert_fixture_matches_golden(
            include_str!(
                "../../tests/fixtures/frontend_graph_parity/multi_flow_conversational.json"
            ),
            include_str!(
                "../../tests/fixtures/frontend_graph_parity/multi_flow_conversational.golden.air"
            ),
        );
    }
}
