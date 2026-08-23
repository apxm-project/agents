use serde_json::Value;

/// The closed string members of a schema `enum` at a `$defs` path.
#[must_use]
pub(crate) fn schema_enum(schema: &Value, def: &str, property: &str) -> Vec<String> {
    let mut members: Vec<String> = schema["$defs"][def]["properties"][property]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("enum at $defs.{def}.properties.{property}"))
        .iter()
        .map(|v| v.as_str().expect("enum member is a string").to_string())
        .collect();
    members.sort();
    members
}
