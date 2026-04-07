//! Parser for .air (Agent IR) text format.
//!
//! Converts canonical text IR to ApxmGraph, analogous to LLVM's llvm-as.

use crate::{ApxmGraph, GraphEdge, GraphNode, GraphError, Parameter};
use apxm_core::types::{AISOperationType, DependencyType, Value};
use std::collections::HashMap;

/// Parse .air text format into an ApxmGraph.
pub fn parse_air(input: &str) -> Result<ApxmGraph, GraphError> {
    let mut parser = AirParser::new(input);
    parser.parse()
}

struct AirParser<'a> {
    lines: Vec<&'a str>,
    line_num: usize,
}

impl<'a> AirParser<'a> {
    fn new(input: &'a str) -> Self {
        let lines: Vec<&str> = input.lines().collect();
        Self {
            lines,
            line_num: 0,
        }
    }

    fn parse(&mut self) -> Result<ApxmGraph, GraphError> {
        let mut name = String::new();
        let mut metadata = HashMap::new();
        let mut parameters = Vec::new();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut node_name_to_id = HashMap::new();
        let mut next_node_id = 1u64;

        while self.line_num < self.lines.len() {
            let line = self.lines[self.line_num].trim();
            self.line_num += 1;

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Parse header comments
            if let Some(rest) = line.strip_prefix("; ") {
                if let Some(graph_name) = rest.strip_prefix("graph: ") {
                    name = graph_name.trim().to_string();
                } else if let Some(param_line) = rest.strip_prefix("params:") {
                    // Parameters section header - continue to param lines
                    if param_line.trim().is_empty() {
                        continue;
                    }
                } else if rest.trim().starts_with('%') {
                    // Parameter declaration: ";   %name: type"
                    if let Some((param_name, type_name)) = self.parse_parameter_line(rest)? {
                        parameters.push(Parameter {
                            name: param_name.to_string(),
                            type_name: type_name.to_string(),
                        });
                    }
                } else if rest.starts_with("edges:") {
                    // Edges section - parse edges
                    while self.line_num < self.lines.len() {
                        let edge_line = self.lines[self.line_num].trim();
                        if !edge_line.starts_with(';') {
                            break;
                        }
                        self.line_num += 1;

                        if let Some(rest) = edge_line.strip_prefix("; ") {
                            if let Some(edge) = self.parse_edge_line(rest, &node_name_to_id)? {
                                edges.push(edge);
                            }
                        }
                    }
                } else {
                    // Other metadata (is_entry, description, etc.)
                    if let Some((k, v)) = rest.split_once(':') {
                        let key = k.trim();
                        let value = v.trim();
                        // Parse value as appropriate type
                        metadata.insert(
                            key.to_string(),
                            self.parse_metadata_value(value)?,
                        );
                    }
                }
                continue;
            }

            // Parse node definitions: %name = ais.op {attrs}
            if line.starts_with('%') {
                let node = self.parse_node_line(line, next_node_id)?;
                node_name_to_id.insert(node.name.clone(), node.id);
                next_node_id += 1;
                nodes.push(node);
            }
        }

        Ok(ApxmGraph {
            name,
            nodes,
            edges,
            parameters,
            metadata,
        })
    }

    fn parse_parameter_line<'b>(&self, line: &'b str) -> Result<Option<(&'b str, &'b str)>, GraphError> {
        // Parse ";   %name: type"
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('%') {
            if let Some((name, type_name)) = rest.split_once(':') {
                return Ok(Some((name.trim(), type_name.trim())));
            }
        }
        Ok(None)
    }

    fn parse_edge_line(
        &self,
        line: &str,
        node_map: &HashMap<String, u64>,
    ) -> Result<Option<GraphEdge>, GraphError> {
        // Parse ";   %from -> %to (DependencyType)"
        let trimmed = line.trim();

        // Find the arrow
        if let Some((from_part, rest)) = trimmed.split_once("->") {
            let from_name = from_part.trim().strip_prefix('%').unwrap_or(from_part.trim());

            // Find dependency type in parens
            if let Some((to_part, dep_part)) = rest.split_once('(') {
                let to_name = to_part.trim().strip_prefix('%').unwrap_or(to_part.trim());
                let dep_str = dep_part.trim().trim_end_matches(')');

                let from_id = *node_map.get(from_name).ok_or_else(|| {
                    GraphError::Validation(format!(
                        "Edge references unknown node: {}",
                        from_name
                    ))
                })?;

                let to_id = *node_map.get(to_name).ok_or_else(|| {
                    GraphError::Validation(format!(
                        "Edge references unknown node: {}",
                        to_name
                    ))
                })?;

                let dependency = self.parse_dependency_type(dep_str)?;

                return Ok(Some(GraphEdge {
                    from: from_id,
                    to: to_id,
                    dependency,
                }));
            }
        }

        Ok(None)
    }

    fn parse_dependency_type(&self, s: &str) -> Result<DependencyType, GraphError> {
        match s.trim() {
            "Data" => Ok(DependencyType::Data),
            "Control" => Ok(DependencyType::Control),
            "Effect" => Ok(DependencyType::Effect),
            other => Err(GraphError::Validation(format!(
                "Unknown dependency type: {}",
                other
            ))),
        }
    }

    fn parse_node_line(&mut self, line: &str, node_id: u64) -> Result<GraphNode, GraphError> {
        // Parse: %name = ais.op {key = "value", key2 = value2}

        // Split on '=' to get name and rest
        let (name_part, rest) = line.split_once('=').ok_or_else(|| {
            GraphError::Validation(format!("Line {}: Invalid node syntax, missing '='", self.line_num))
        })?;

        let node_name = name_part.trim().strip_prefix('%').ok_or_else(|| {
            GraphError::Validation(format!("Line {}: Node name must start with %", self.line_num))
        })?.to_string();

        let rest = rest.trim();

        // Parse "ais.op" or "ais.op {attrs}"
        let (op_part, attrs_part) = if let Some((op, attrs)) = rest.split_once('{') {
            (op.trim(), Some(attrs.trim()))
        } else {
            (rest.trim(), None)
        };

        // Extract operation name from "ais.op_name"
        let op_name = op_part.strip_prefix("ais.").ok_or_else(|| {
            GraphError::Validation(format!("Line {}: Operation must start with 'ais.'", self.line_num))
        })?;

        // Convert op_name to AISOperationType
        let op = self.parse_operation_type(op_name)?;

        // Parse attributes if present
        let attributes = if let Some(attrs) = attrs_part {
            self.parse_attributes(attrs)?
        } else {
            HashMap::new()
        };

        Ok(GraphNode {
            id: node_id,
            name: node_name,
            op,
            attributes,
        })
    }

    fn parse_operation_type(&self, name: &str) -> Result<AISOperationType, GraphError> {
        // Convert lowercase snake_case to uppercase (e.g., "spawn_agent" -> "SPAWN_AGENT")
        let upper = name.to_uppercase();

        // Try to parse as operation type
        use std::str::FromStr;
        AISOperationType::from_str(&upper).map_err(|_| {
            GraphError::Validation(format!(
                "Line {}: Unknown operation type: {}",
                self.line_num, name
            ))
        })
    }

    fn parse_attributes(&mut self, attrs_str: &str) -> Result<HashMap<String, Value>, GraphError> {
        let mut attributes = HashMap::new();

        // Remove trailing }
        let attrs_str = attrs_str.trim_end_matches('}').trim();

        if attrs_str.is_empty() {
            return Ok(attributes);
        }

        // Parse key-value pairs
        // Handle comma-separated attributes, tracking bracket/brace depth to avoid
        // splitting on commas inside arrays or objects
        let mut current_key = String::new();
        let mut current_value = String::new();
        let mut in_string = false;
        let mut in_value = false;
        let mut bracket_depth = 0; // Track [] depth
        let mut brace_depth = 0;   // Track {} depth
        let mut chars = attrs_str.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '"' if !in_string => {
                    in_string = true;
                    current_value.push(c);
                }
                '"' if in_string => {
                    in_string = false;
                    current_value.push(c);
                }
                '[' if !in_string => {
                    bracket_depth += 1;
                    current_value.push(c);
                }
                ']' if !in_string => {
                    bracket_depth -= 1;
                    current_value.push(c);
                }
                '{' if !in_string => {
                    brace_depth += 1;
                    current_value.push(c);
                }
                '}' if !in_string => {
                    brace_depth -= 1;
                    current_value.push(c);
                }
                '=' if !in_string && !in_value && bracket_depth == 0 && brace_depth == 0 => {
                    current_key = current_value.trim().to_string();
                    current_value.clear();
                    in_value = true;
                }
                ',' if !in_string && bracket_depth == 0 && brace_depth == 0 => {
                    // End of this key-value pair (only if not inside [] or {})
                    if !current_key.is_empty() {
                        let value = self.parse_value(current_value.trim())?;
                        attributes.insert(current_key.clone(), value);
                    }
                    current_key.clear();
                    current_value.clear();
                    in_value = false;
                }
                _ => {
                    current_value.push(c);
                }
            }
        }

        // Don't forget the last pair
        if !current_key.is_empty() && !current_value.trim().is_empty() {
            let value = self.parse_value(current_value.trim())?;
            attributes.insert(current_key, value);
        }

        Ok(attributes)
    }

    fn parse_value(&self, s: &str) -> Result<Value, GraphError> {
        let s = s.trim();

        // String (quoted)
        if s.starts_with('"') && s.ends_with('"') {
            let content = &s[1..s.len() - 1];
            // Unescape common escapes
            let unescaped = content
                .replace("\\n", "\n")
                .replace("\\t", "\t")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
            return Ok(Value::String(unescaped));
        }

        // Boolean
        if s == "true" {
            return Ok(Value::Bool(true));
        }
        if s == "false" {
            return Ok(Value::Bool(false));
        }

        // Number (integer or float)
        if let Ok(i) = s.parse::<i64>() {
            return Ok(Value::Number(i.into()));
        }
        if let Ok(f) = s.parse::<f64>() {
            return Ok(Value::Number(f.into()));
        }

        // Array (simplified - just JSON arrays for now)
        if s.starts_with('[') && s.ends_with(']') {
            // Use serde_json for simplicity
            let json_val: serde_json::Value = serde_json::from_str(s)
                .map_err(|e| GraphError::Validation(format!("Invalid array: {}", e)))?;
            return self.json_to_value(&json_val);
        }

        // Object (simplified - just JSON objects for now)
        if s.starts_with('{') && s.ends_with('}') {
            let json_val: serde_json::Value = serde_json::from_str(s)
                .map_err(|e| GraphError::Validation(format!("Invalid object: {}", e)))?;
            return self.json_to_value(&json_val);
        }

        // Otherwise treat as unquoted string (for simple values)
        Ok(Value::String(s.to_string()))
    }

    fn parse_metadata_value(&self, s: &str) -> Result<Value, GraphError> {
        // Try bool first
        if s == "true" {
            return Ok(Value::Bool(true));
        }
        if s == "false" {
            return Ok(Value::Bool(false));
        }

        // Try number
        if let Ok(i) = s.parse::<i64>() {
            return Ok(Value::Number(i.into()));
        }
        if let Ok(f) = s.parse::<f64>() {
            return Ok(Value::Number(f.into()));
        }

        // Default to string
        Ok(Value::String(s.to_string()))
    }

    fn json_to_value(&self, json: &serde_json::Value) -> Result<Value, GraphError> {
        match json {
            serde_json::Value::Null => Ok(Value::Null),
            serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Value::Number(i.into()))
                } else if let Some(f) = n.as_f64() {
                    Ok(Value::Number(f.into()))
                } else {
                    Err(GraphError::Validation("Invalid number".to_string()))
                }
            }
            serde_json::Value::String(s) => Ok(Value::String(s.clone())),
            serde_json::Value::Array(arr) => {
                let values: Result<Vec<_>, _> = arr.iter().map(|v| self.json_to_value(v)).collect();
                Ok(Value::Array(values?))
            }
            serde_json::Value::Object(obj) => {
                let mut map = HashMap::new();
                for (k, v) in obj {
                    map.insert(k.clone(), self.json_to_value(v)?);
                }
                Ok(Value::Object(map))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_air() {
        let air = r#"
; Agent IR (.air) — canonical intermediate representation
; graph: hello
; is_entry: true

; params:
;   %topic: str

  %greeting = ais.const_str {value = "Hello {0}"}
  %ask = ais.ask {template_str = "Process: {0}"}

  ; edges:
  ;   %greeting -> %ask (Data)
"#;

        let graph = parse_air(air).expect("parse failed");
        assert_eq!(graph.name, "hello");
        assert_eq!(graph.parameters.len(), 1);
        assert_eq!(graph.parameters[0].name, "topic");
        assert_eq!(graph.parameters[0].type_name, "str");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);

        // Check metadata
        assert_eq!(
            graph.metadata.get("is_entry"),
            Some(&Value::Bool(true))
        );

        // Check nodes
        assert_eq!(graph.nodes[0].name, "greeting");
        assert_eq!(graph.nodes[0].op, AISOperationType::ConstStr);
        assert_eq!(
            graph.nodes[0].attributes.get("value"),
            Some(&Value::String("Hello {0}".to_string()))
        );

        assert_eq!(graph.nodes[1].name, "ask");
        assert_eq!(graph.nodes[1].op, AISOperationType::Ask);

        // Check edges
        assert_eq!(graph.edges[0].from, 1);
        assert_eq!(graph.edges[0].to, 2);
        assert_eq!(graph.edges[0].dependency, DependencyType::Data);
    }

    #[test]
    fn parse_complex_attributes() {
        let air = r#"
; graph: test

  %node = ais.merge {strategy = "concat", tokens = ["{{node_1}}", "{{node_2}}"]}
"#;

        let graph = parse_air(air).expect("parse failed");
        assert_eq!(graph.nodes.len(), 1);

        let node = &graph.nodes[0];
        assert_eq!(
            node.attributes.get("strategy"),
            Some(&Value::String("concat".to_string()))
        );

        // Check that tokens array was parsed
        if let Some(Value::Array(arr)) = node.attributes.get("tokens") {
            assert_eq!(arr.len(), 2);
        } else {
            panic!("tokens should be an array");
        }
    }

    #[test]
    fn parse_multiple_edges() {
        let air = r#"
; graph: multi

  %a = ais.const_str {value = "a"}
  %b = ais.const_str {value = "b"}
  %c = ais.merge {}

  ; edges:
  ;   %a -> %c (Data)
  ;   %b -> %c (Data)
"#;

        let graph = parse_air(air).expect("parse failed");
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].dependency, DependencyType::Data);
        assert_eq!(graph.edges[1].dependency, DependencyType::Data);
    }

    #[test]
    fn parse_control_and_effect_dependencies() {
        let air = r#"
; graph: deps

  %spawn = ais.spawn_agent {agent_name = "worker"}
  %comm = ais.communicate {recipient = "worker"}
  %mem = ais.umem {key = "state"}

  ; edges:
  ;   %spawn -> %comm (Control)
  ;   %comm -> %mem (Effect)
"#;

        let graph = parse_air(air).expect("parse failed");
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].dependency, DependencyType::Control);
        assert_eq!(graph.edges[1].dependency, DependencyType::Effect);
    }

    #[test]
    fn parse_escaped_strings() {
        let air = r#"
; graph: escape

  %node = ais.const_str {value = "Line 1\nLine 2\tTabbed"}
"#;

        let graph = parse_air(air).expect("parse failed");
        let val = graph.nodes[0].attributes.get("value").unwrap();
        if let Value::String(s) = val {
            assert!(s.contains('\n'));
            assert!(s.contains('\t'));
        } else {
            panic!("Expected string value");
        }
    }
}
