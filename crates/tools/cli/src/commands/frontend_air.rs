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
}
