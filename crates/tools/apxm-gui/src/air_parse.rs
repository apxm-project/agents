//! Lightweight parser for `.air` (MLIR textual format) into graph JSON.
//!
//! Extracts nodes and SSA data-flow edges from single-block AIS IR so the GUI
//! can visualise `.air` files without requiring a full MLIR toolchain.

use apxm_core::types::{AISOperationType, get_operation_spec};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Parse AIR textual IR into a JSON object matching the `AirModule` / `ApxmGraph` shape:
/// `{ name, nodes, edges, parameters, metadata }`.
pub fn parse_air_text(source: &str) -> Result<Value, String> {
    let trimmed = source.trim();
    if !(trimmed.starts_with("module") || trimmed.starts_with("func.func")) {
        return Err("not a valid AIR textual file".into());
    }

    let mut func_name = String::from("unnamed");
    let mut parameters: Vec<Value> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();

    // SSA name → node id that *defines* it
    let mut defs: HashMap<String, u64> = HashMap::new();
    let mut next_id: u64 = 0;

    for line in source.lines() {
        let line = line.trim();

        // Extract function name: `func.func @name(...)`
        if line.starts_with("func.func @") {
            if let Some(rest) = line.strip_prefix("func.func @") {
                func_name = rest
                    .split(|c: char| c == '(' || c == ' ')
                    .next()
                    .unwrap_or("unnamed")
                    .to_string();

                // Extract parameters from `(%arg0: !ais.token {ais.param_name = "x", ...}, ...)`
                if let Some(paren_start) = rest.find('(') {
                    if let Some(paren_end) = find_matching_paren(rest, paren_start) {
                        let params_str = &rest[paren_start + 1..paren_end];
                        for param in split_params(params_str) {
                            if let Some(name) = extract_attr_value(&param, "ais.param_name") {
                                let type_name = extract_attr_value(&param, "ais.param_type")
                                    .unwrap_or_else(|| "str".to_string());
                                parameters.push(json!({ "name": name, "type_name": type_name }));
                            }
                        }
                    }
                }
            }
            continue;
        }

        // Skip non-operation lines
        if line.is_empty()
            || line.starts_with("module")
            || line.starts_with("}")
            || line.starts_with("//")
        {
            continue;
        }

        // Handle `func.return %var : type` — connects to the referenced node but
        // doesn't create a new node if the referenced value is already a Return op.
        if line.starts_with("func.return") {
            let used_refs = extract_operand_refs(line);
            let already_returns = used_refs.iter().any(|r| {
                defs.get(r).map_or(false, |&def_id| {
                    nodes
                        .iter()
                        .any(|n| n["id"] == def_id && n["op"] == "Return")
                })
            });
            if !already_returns {
                let id = next_id;
                next_id += 1;
                nodes.push(json!({
                    "id": id,
                    "name": "return",
                    "op": "Return",
                    "attributes": {}
                }));
                for used in &used_refs {
                    if let Some(&from_id) = defs.get(used) {
                        edges.push(json!({ "from": from_id, "to": id, "dependency": "Data" }));
                    }
                }
            }
            continue;
        }

        // Parse AIS operations: `%result = ais.mnemonic ...` or `ais.mnemonic ...`
        if !line.contains("ais.") {
            continue;
        }

        let (result_name, op_part) = if let Some(eq_pos) = line.find(" = ais.") {
            let result = line[..eq_pos].trim_start_matches('%').to_string();
            let rest = &line[eq_pos + 3..]; // skip " = "
            (Some(result), rest.to_string())
        } else if let Some(idx) = line.find("ais.") {
            (None, line[idx..].to_string())
        } else {
            continue;
        };

        // Extract mnemonic: `ais.<mnemonic> ...`
        let mnemonic = op_part
            .strip_prefix("ais.")
            .and_then(|s| s.split(|c: char| c.is_whitespace() || c == '"').next())
            .unwrap_or("unknown")
            .to_string();

        let op_type = mnemonic_to_op_type(&mnemonic);
        let node_name = result_name.as_deref().unwrap_or(&mnemonic).to_string();

        let id = next_id;
        next_id += 1;

        // Extract attributes from the line
        let mut attrs: HashMap<String, Value> = HashMap::new();

        // Extract quoted strings (first string after mnemonic is typically a name/template)
        let after_mnemonic = op_part
            .strip_prefix(&format!("ais.{mnemonic}"))
            .unwrap_or("");
        let quoted_strings = extract_quoted_strings(after_mnemonic);
        if let Some(first) = quoted_strings.first() {
            match mnemonic.as_str() {
                "spawn_agent" => {
                    attrs.insert("agent_name".into(), json!(first));
                }
                "const_str" => {
                    let preview = if first.len() > 80 {
                        format!("{}...", &first[..80])
                    } else {
                        first.clone()
                    };
                    attrs.insert("value".into(), json!(preview));
                }
                "communicate" => {
                    attrs.insert("template".into(), json!(first));
                }
                "print" => {
                    attrs.insert("format".into(), json!(first));
                }
                "ask" | "think" => {
                    attrs.insert("template".into(), json!(first));
                }
                _ => {
                    attrs.insert("value".into(), json!(first));
                }
            }
        }

        // For communicate, extract `to "recipient"`
        if mnemonic == "communicate" {
            if let Some(to_name) = extract_communicate_target(after_mnemonic) {
                attrs.insert("to".into(), json!(to_name));
            }
        }

        // Extract {key = "value"} inline attributes
        if let Some(brace_attrs) = extract_brace_attrs(after_mnemonic) {
            for (k, v) in brace_attrs {
                attrs.insert(k, json!(v));
            }
        }

        nodes.push(json!({
            "id": id,
            "name": node_name,
            "op": op_type,
            "attributes": attrs
        }));

        if let Some(ref name) = result_name {
            defs.insert(name.clone(), id);
        }

        // Build edges from operand references
        for used in extract_operand_refs(&op_part) {
            if let Some(&from_id) = defs.get(&used) {
                edges.push(json!({ "from": from_id, "to": id, "dependency": "Data" }));
            }
        }
    }

    // Heuristic: infer Control edges from SpawnAgent → Communicate by name matching.
    // When a communicate node name contains an agent prefix that matches a spawn node,
    // add a control-flow dependency (spawn must happen before communicate).
    infer_agent_edges(&nodes, &mut edges);

    Ok(json!({
        "name": func_name,
        "nodes": nodes,
        "edges": edges,
        "parameters": parameters,
        "metadata": {}
    }))
}

/// Infer control edges between SpawnAgent and Communicate nodes by name matching.
/// The Python SDK names spawned agents like `dev_team_architect_0` and their
/// communicate ops like `architect_msg_1`. We extract the agent role from both
/// and connect matching pairs.
fn infer_agent_edges(nodes: &[Value], edges: &mut Vec<Value>) {
    let existing: std::collections::HashSet<(u64, u64)> = edges
        .iter()
        .filter_map(|e| Some((e["from"].as_u64()?, e["to"].as_u64()?)))
        .collect();

    let spawns: Vec<(u64, &str)> = nodes
        .iter()
        .filter(|n| n["op"] == "SpawnAgent")
        .filter_map(|n| Some((n["id"].as_u64()?, n["name"].as_str()?)))
        .collect();

    let communicates: Vec<(u64, &str)> = nodes
        .iter()
        .filter(|n| n["op"] == "Communicate")
        .filter_map(|n| Some((n["id"].as_u64()?, n["name"].as_str()?)))
        .collect();

    for &(spawn_id, spawn_name) in &spawns {
        let role = extract_agent_role(spawn_name);
        if role.is_empty() {
            continue;
        }
        for &(comm_id, comm_name) in &communicates {
            if comm_name.starts_with(&format!("{role}_"))
                || comm_name.contains(&format!("_{role}_"))
            {
                if !existing.contains(&(spawn_id, comm_id)) {
                    edges.push(json!({ "from": spawn_id, "to": comm_id, "dependency": "Control" }));
                }
            }
        }
    }
}

/// Extract the agent role from a spawn node name.
/// E.g. `dev_team_architect_0` → `architect`, `worker_1` → `worker`.
fn extract_agent_role(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').collect();
    if parts.len() >= 2 {
        // Last part is usually a numeric suffix; role is second-to-last
        let last = *parts.last().unwrap();
        if last.chars().all(|c| c.is_ascii_digit()) && parts.len() >= 3 {
            return parts[parts.len() - 2].to_string();
        }
        // Fallback: skip common prefixes
        if parts.len() >= 3 {
            return parts[parts.len() - 2].to_string();
        }
        return parts.last().unwrap().to_string();
    }
    name.to_string()
}

/// Resolve MLIR mnemonic to the PascalCase display name from the shared core operation catalog.
fn mnemonic_to_op_type(mnemonic: &str) -> String {
    match mnemonic.parse::<AISOperationType>() {
        Ok(op) => get_operation_spec(op).name.to_string(),
        Err(_) => mnemonic.to_string(),
    }
}

/// Extract `%name` references used as operands (in parentheses, brackets, or comma lists).
fn extract_operand_refs(line: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = line.as_bytes();

    // Skip past `%result = ` at the start (that's a definition, not a use)
    let mut i = if let Some(eq) = line.find(" = ais.") {
        eq + 3
    } else {
        0
    };
    while i < bytes.len() {
        // Skip quoted strings entirely
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'%' {
            let start = i + 1;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i > start {
                refs.push(line[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }

    refs
}

/// Extract quoted strings from a line (handling backslash escapes).
fn extract_quoted_strings(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut val = String::new();
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'n' => val.push('\n'),
                        b't' => val.push('\t'),
                        b'"' => val.push('"'),
                        b'\\' => val.push('\\'),
                        c => {
                            val.push('\\');
                            val.push(c as char);
                        }
                    }
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                val.push(bytes[i] as char);
                i += 1;
            }
            result.push(val);
        } else {
            i += 1;
        }
    }
    result
}

/// Extract `to "name"` from communicate ops.
fn extract_communicate_target(s: &str) -> Option<String> {
    let idx = s.find(" to ")?;
    let after = &s[idx + 4..];
    let strings = extract_quoted_strings(after);
    strings.into_iter().next()
}

/// Extract `{key = "value", ...}` inline attributes.
fn extract_brace_attrs(s: &str) -> Option<Vec<(String, String)>> {
    let brace_start = s.find('{')?;
    let brace_end = find_matching_brace(s, brace_start)?;
    let inner = &s[brace_start + 1..brace_end];

    let mut attrs = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].trim().to_string();
            let val = part[eq_pos + 1..].trim().trim_matches('"').to_string();
            if !key.is_empty() {
                attrs.push((key, val));
            }
        }
    }

    Some(attrs)
}

fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let bytes = s.as_bytes();
    let mut in_string = false;
    for i in start..bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_string = !in_string;
        }
        if in_string {
            continue;
        }
        if bytes[i] == b'(' {
            depth += 1;
        }
        if bytes[i] == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn find_matching_brace(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let bytes = s.as_bytes();
    let mut in_string = false;
    for i in start..bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_string = !in_string;
        }
        if in_string {
            continue;
        }
        if bytes[i] == b'{' {
            depth += 1;
        }
        if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Split comma-separated parameter declarations, respecting nested braces.
fn split_params(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let bytes = s.as_bytes();
    let mut in_string = false;

    for &b in bytes {
        if b == b'"' {
            in_string = !in_string;
        }
        if in_string {
            current.push(b as char);
            continue;
        }
        match b {
            b'{' => {
                depth += 1;
                current.push('{');
            }
            b'}' => {
                depth -= 1;
                current.push('}');
            }
            b',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(b as char),
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

/// Extract a named attribute value from a parameter string like `{ais.param_name = "x"}`.
fn extract_attr_value(s: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr} = ");
    let idx = s.find(&pattern)?;
    let after = &s[idx + pattern.len()..];
    let strings = extract_quoted_strings(after);
    strings.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_air() {
        let air = r#"module {
  func.func @test_flow() -> !ais.token attributes {ais.entry} {
    %a = ais.spawn_agent "worker" {profile = "claude"} : !ais.token
    %b = ais.const_str "hello world" : !ais.token
    %c = ais.communicate "{b}" to "worker" (%b : !ais.token) {input_names = ["b"]} : !ais.token
    ais.print "result: {c}" [%c : !ais.token] {input_names = ["c"]}
    func.return %c : !ais.token
  }
}"#;
        let result = parse_air_text(air).unwrap();
        assert_eq!(result["name"], "test_flow");
        assert_eq!(result["nodes"].as_array().unwrap().len(), 5); // spawn, const, comm, print, return
        assert!(!result["edges"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_with_params() {
        let air = r#"module {
  func.func @my_flow(%arg0: !ais.token {ais.param_name = "input", ais.param_type = "str"}) -> !ais.token attributes {ais.entry} {
    %a = ais.ask "question" [%arg0 : !ais.token] : !ais.token
    func.return %a : !ais.token
  }
}"#;
        let result = parse_air_text(air).unwrap();
        assert_eq!(result["name"], "my_flow");
        assert_eq!(result["parameters"].as_array().unwrap().len(), 1);
        assert_eq!(result["parameters"][0]["name"], "input");
    }
}
