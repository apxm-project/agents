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

#[cfg(test)]
mod tests {
    use super::*;

    struct StubSource {
        name: &'static str,
        value: Value,
    }

    impl MetricsSource for StubSource {
        fn section_name(&self) -> &'static str {
            self.name
        }
        fn collect(&self) -> Value {
            self.value.clone()
        }
    }

    #[test]
    fn report_contains_schema_version_and_sections() {
        let mut report = MetricsReport::new();
        report.add_source(&StubSource {
            name: metrics_keys::SECTION_COMPILER,
            value: serde_json::json!({"passes": []}),
        });
        report.add_source(&StubSource {
            name: metrics_keys::SECTION_RUNTIME,
            value: serde_json::json!({"execution": {}}),
        });

        let json = report.to_json();
        let obj = json.as_object().expect("report must be an object");

        assert_eq!(
            obj[metrics_keys::SCHEMA_VERSION],
            metrics_keys::SCHEMA_VERSION_VALUE
        );
        assert!(obj.contains_key(metrics_keys::SECTION_COMPILER));
        assert!(obj.contains_key(metrics_keys::SECTION_RUNTIME));
    }

    #[test]
    fn null_sections_are_omitted() {
        let mut report = MetricsReport::new();
        report.add_source(&StubSource {
            name: metrics_keys::SECTION_BACKENDS,
            value: Value::Null,
        });

        let json = report.to_json();
        let obj = json.as_object().expect("report must be an object");

        assert!(!obj.contains_key(metrics_keys::SECTION_BACKENDS));
        // Only schema_version should be present.
        assert_eq!(obj.len(), 1);
    }

    #[test]
    fn trait_is_object_safe() {
        // Proving dyn-compatibility: we can construct a Box<dyn MetricsSource>
        // and call both methods through the trait object.
        let boxed: Box<dyn MetricsSource> = Box::new(StubSource {
            name: "test",
            value: serde_json::json!(42),
        });
        assert_eq!(boxed.section_name(), "test");
        assert_eq!(boxed.collect(), serde_json::json!(42));
    }

    #[test]
    fn empty_report_has_only_schema_version() {
        let report = MetricsReport::new();
        let json = report.to_json();
        let obj = json.as_object().expect("report must be an object");

        assert_eq!(obj.len(), 1);
        assert_eq!(
            obj[metrics_keys::SCHEMA_VERSION],
            metrics_keys::SCHEMA_VERSION_VALUE
        );
    }
}
