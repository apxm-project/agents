//! Per-execution collector for `x-apxm-fields-honored` evidence.
//!
//! Each per-node LLM response carries a `metadata["fields_honored"]`
//! list populated by the OpenAI backend's
//! `parse_apxm_fields_honored_header` (see `apxm-backends`). This
//! collector folds those per-request lists into a per-backend union
//! for the entire execution, so `dispatch_ir_accounting_json` can
//! report `fields_honored` as runtime evidence — distinct from the
//! static `fields_capability_supported_by_backend` partition.
//!
//! Plan 07 §2 closure. The fork-side emitter is at
//! `external/vllm@490aaad0c`; the APXM-side parser is at
//! `apxm-backends/openai/backend.rs::parse_apxm_fields_honored_header`;
//! this is the per-execution aggregation point Plan 07 §2's
//! "Remaining work" §2 names.

use std::collections::{BTreeSet, HashMap};

use parking_lot::Mutex;

#[derive(Debug, Default)]
pub struct FieldsHonoredCollector {
    // BTreeSet so the per-backend list is stable+deduped.
    inner: Mutex<HashMap<String, BTreeSet<String>>>,
}

impl FieldsHonoredCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one per-request honor list under `backend_name`. Empty
    /// lists are no-ops — an empty `fields_honored` from a backend
    /// means "this backend did not emit the x-apxm-fields-honored
    /// header for this request" (either it doesn't run the fork-side
    /// emitter, or the request had no apxm block). Either way the
    /// per-backend union is unaffected.
    pub fn record<I, S>(&self, backend_name: &str, fields: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut iter = fields.into_iter().peekable();
        if iter.peek().is_none() {
            return;
        }
        let mut guard = self.inner.lock();
        let bucket = guard.entry(backend_name.to_owned()).or_default();
        for field in iter {
            bucket.insert(field.into());
        }
    }

    /// Snapshot the per-backend union as a plain HashMap suitable for
    /// JSON serialization. Returns a copy; the collector remains
    /// usable for subsequent records.
    pub fn snapshot(&self) -> HashMap<String, Vec<String>> {
        self.inner
            .lock()
            .iter()
            .map(|(backend, fields)| (backend.clone(), fields.iter().cloned().collect()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::constants::llm::apxm::dispatch_fields as df;

    #[test]
    fn record_unions_across_calls() {
        let c = FieldsHonoredCollector::new();
        c.record("vllm-fork", [df::PRIORITY, df::PIN_RELEASE]);
        c.record("vllm-fork", [df::PRIORITY, df::PREFIX_COHORTS]);
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let mut got = snap["vllm-fork"].clone();
        got.sort();
        let mut expected = vec![
            df::PIN_RELEASE.to_owned(),
            df::PREFIX_COHORTS.to_owned(),
            df::PRIORITY.to_owned(),
        ];
        expected.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn record_separates_backends() {
        let c = FieldsHonoredCollector::new();
        c.record("vllm-fork", [df::PRIORITY]);
        c.record("openai-shim", [df::REQUEST_HINTS]);
        let snap = c.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["vllm-fork"], vec![df::PRIORITY.to_owned()]);
        assert_eq!(snap["openai-shim"], vec![df::REQUEST_HINTS.to_owned()]);
    }

    #[test]
    fn empty_record_is_noop() {
        let c = FieldsHonoredCollector::new();
        c.record::<_, String>("vllm-fork", Vec::<String>::new());
        assert!(c.snapshot().is_empty());
    }
}
