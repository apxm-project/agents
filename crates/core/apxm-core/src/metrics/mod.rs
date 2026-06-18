use std::collections::BTreeMap;

use serde_json::Value;

use crate::constants::session::metrics_keys;

/// Post-execution metrics collector. Each subsystem (compiler, runtime,
/// backends) implements this trait and produces its JSON section.
/// Not a streaming event bus — use `apxm_events::EventEmitter` for live events.
pub trait MetricsSource {
    /// Top-level JSON key under which `collect()` output is nested
    /// (e.g. "compiler", "runtime", "backends").
    fn section_name(&self) -> &'static str;

    /// Produce this source's JSON section. Returning `Value::Null` means
    /// "nothing to report" and the section is omitted from the final report.
    fn collect(&self) -> Value;
}

/// Schema-versioned envelope that merges sections from multiple `MetricsSource`
/// implementations into a single JSON document.
pub struct MetricsReport {
    schema_version: u32,
    sections: BTreeMap<&'static str, Value>,
}

impl MetricsReport {
    pub fn new() -> Self {
        Self {
            schema_version: metrics_keys::SCHEMA_VERSION_VALUE,
            sections: BTreeMap::new(),
        }
    }

    /// Add a source's section to the report. `Value::Null` sections are dropped.
    pub fn add_source(&mut self, source: &dyn MetricsSource) {
        let value = source.collect();
        if !value.is_null() {
            self.sections.insert(source.section_name(), value);
        }
    }

    /// Serialize to a flat JSON object: `{ "schema_version": N, ...sections }`.
    pub fn to_json(&self) -> Value {
        let mut map = serde_json::Map::with_capacity(1 + self.sections.len());
        map.insert(
            metrics_keys::SCHEMA_VERSION.to_owned(),
            Value::Number(self.schema_version.into()),
        );
        for (&key, value) in &self.sections {
            map.insert(key.to_owned(), value.clone());
        }
        Value::Object(map)
    }
}

impl Default for MetricsReport {
    fn default() -> Self {
        Self::new()
    }
}
