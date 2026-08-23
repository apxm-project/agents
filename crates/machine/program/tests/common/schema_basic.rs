#[path = "schema.rs"]
mod schema;

/// Compile a published owner schema without sibling repository references.
#[must_use]
pub(crate) fn compile_schema(
    relative: &str,
    referenced_snapshots: &[&str],
) -> jsonschema::JSONSchema {
    schema::compile_schema_with(relative, referenced_snapshots, &[])
}
