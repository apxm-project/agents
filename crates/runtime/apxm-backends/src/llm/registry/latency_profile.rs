//! Per-backend EWMA latency profile.
//!
//! Tracks prefill base latency and per-token decode cost using an
//! exponentially-weighted moving average so the runtime can build a stable
//! cost surface for backend-aware dispatch without keeping per-request
//! histograms in hot memory.
//!
//! This module is currently introduced as a skeleton: it ships alongside
//! `HealthMonitor` and exposes a typed `BackendLatencyProfile` and aggregated
//! `LatencyProfileStore` so dispatch logic can adopt it incrementally. It is
//! deliberately not wired into `HealthMonitor::record_success` yet — the
//! prefill/decode breakdown lives in the runtime's `timing_tracker` and will
//! be threaded through in a follow-up once the streaming path produces those
//! samples at the registry boundary.

use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

/// Default smoothing factor (`alpha`) for the EWMA. Smaller `alpha` means
/// more weight on history (smoother), larger `alpha` reacts faster to
/// recent samples.
pub const DEFAULT_LATENCY_EWMA_ALPHA: f64 = 0.2;

/// Minimum samples before the profile is considered usable for dispatch
/// decisions. Below this, callers should treat the estimates as warm-up only.
pub const LATENCY_PROFILE_MIN_SAMPLES: usize = 5;

/// EWMA-tracked cost model for one backend.
#[derive(Debug, Clone)]
pub struct BackendLatencyProfile {
    alpha: f64,
    prefill_ms_ewma: Option<f64>,
    decode_ms_per_token_ewma: Option<f64>,
    prefill_samples: usize,
    decode_samples: usize,
}

impl BackendLatencyProfile {
    /// New empty profile with the default smoothing factor.
    pub fn new() -> Self {
        Self::with_alpha(DEFAULT_LATENCY_EWMA_ALPHA)
    }

    /// New empty profile with a caller-chosen smoothing factor.
    pub fn with_alpha(alpha: f64) -> Self {
        let alpha = alpha.clamp(f64::EPSILON, 1.0);
        Self {
            alpha,
            prefill_ms_ewma: None,
            decode_ms_per_token_ewma: None,
            prefill_samples: 0,
            decode_samples: 0,
        }
    }

    /// Record one prefill observation: total prefill latency (whole prompt).
    pub fn record_prefill(&mut self, latency: Duration) {
        let sample_ms = duration_to_ms(latency);
        self.prefill_ms_ewma = Some(merge(self.prefill_ms_ewma, sample_ms, self.alpha));
        self.prefill_samples += 1;
    }

    /// Record one decode observation: total decode latency for `tokens` output
    /// tokens. Samples with `tokens == 0` are ignored — a zero-token decode
    /// has no per-token signal.
    pub fn record_decode(&mut self, latency: Duration, tokens: u32) {
        if tokens == 0 {
            return;
        }
        let per_token_ms = duration_to_ms(latency) / f64::from(tokens);
        self.decode_ms_per_token_ewma = Some(merge(
            self.decode_ms_per_token_ewma,
            per_token_ms,
            self.alpha,
        ));
        self.decode_samples += 1;
    }

    /// Smoothed prefill latency in milliseconds, or `None` if no samples yet.
    pub fn prefill_ms(&self) -> Option<f64> {
        self.prefill_ms_ewma
    }

    /// Smoothed per-token decode cost in milliseconds, or `None` if no
    /// samples yet.
    pub fn decode_ms_per_token(&self) -> Option<f64> {
        self.decode_ms_per_token_ewma
    }

    /// Predict end-to-end latency for a hypothetical request with the given
    /// expected output token count. Returns `None` when either component is
    /// missing.
    pub fn predict_total_ms(&self, expected_output_tokens: u32) -> Option<f64> {
        let prefill = self.prefill_ms_ewma?;
        let per_token = self.decode_ms_per_token_ewma?;
        Some(prefill + per_token * f64::from(expected_output_tokens))
    }

    /// Whether the profile has enough samples to be used for dispatch.
    pub fn is_warm(&self) -> bool {
        self.prefill_samples >= LATENCY_PROFILE_MIN_SAMPLES
            && self.decode_samples >= LATENCY_PROFILE_MIN_SAMPLES
    }

    /// Number of prefill samples folded into the EWMA.
    pub fn prefill_samples(&self) -> usize {
        self.prefill_samples
    }

    /// Number of decode samples folded into the EWMA.
    pub fn decode_samples(&self) -> usize {
        self.decode_samples
    }
}

impl Default for BackendLatencyProfile {
    fn default() -> Self {
        Self::new()
    }
}

fn merge(prev: Option<f64>, sample: f64, alpha: f64) -> f64 {
    match prev {
        None => sample,
        Some(prev) => alpha * sample + (1.0 - alpha) * prev,
    }
}

fn duration_to_ms(latency: Duration) -> f64 {
    (latency.as_secs_f64()) * 1000.0
}

/// Thread-safe per-backend EWMA store. Mirrors the shape of `HealthMonitor`
/// so callers can hold one of each side-by-side without juggling locks.
pub struct LatencyProfileStore {
    profiles: Arc<DashMap<String, Mutex<BackendLatencyProfile>>>,
}

impl LatencyProfileStore {
    pub fn new() -> Self {
        Self {
            profiles: Arc::new(DashMap::new()),
        }
    }

    pub fn register_backend(&self, name: &str) {
        self.profiles
            .entry(name.to_string())
            .or_insert_with(|| Mutex::new(BackendLatencyProfile::new()));
    }

    pub fn unregister_backend(&self, name: &str) {
        self.profiles.remove(name);
    }

    pub fn record_prefill(&self, name: &str, latency: Duration) {
        if let Some(entry) = self.profiles.get(name) {
            entry.value().lock().record_prefill(latency);
        }
    }

    pub fn record_decode(&self, name: &str, latency: Duration, tokens: u32) {
        if let Some(entry) = self.profiles.get(name) {
            entry.value().lock().record_decode(latency, tokens);
        }
    }

    pub fn snapshot(&self, name: &str) -> Option<BackendLatencyProfile> {
        self.profiles
            .get(name)
            .map(|entry| entry.value().lock().clone())
    }
}

impl Default for LatencyProfileStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_profile_returns_none() {
        let profile = BackendLatencyProfile::new();
        assert!(profile.prefill_ms().is_none());
        assert!(profile.decode_ms_per_token().is_none());
        assert!(profile.predict_total_ms(64).is_none());
        assert!(!profile.is_warm());
    }

    #[test]
    fn first_sample_seeds_ewma_to_sample_value() {
        let mut profile = BackendLatencyProfile::new();
        profile.record_prefill(Duration::from_millis(120));
        profile.record_decode(Duration::from_millis(40), 10);

        let prefill_ms = profile.prefill_ms().expect("prefill seeded");
        let per_token_ms = profile.decode_ms_per_token().expect("decode seeded");
        assert!((prefill_ms - 120.0).abs() < 1e-9);
        assert!((per_token_ms - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ewma_smooths_subsequent_samples() {
        let mut profile = BackendLatencyProfile::with_alpha(0.5);
        profile.record_prefill(Duration::from_millis(100));
        profile.record_prefill(Duration::from_millis(200));
        // 0.5 * 200 + 0.5 * 100 = 150
        let prefill_ms = profile.prefill_ms().expect("prefill seeded");
        assert!((prefill_ms - 150.0).abs() < 1e-9);
    }

    #[test]
    fn decode_ignores_zero_token_samples() {
        let mut profile = BackendLatencyProfile::new();
        profile.record_decode(Duration::from_millis(80), 0);
        assert!(profile.decode_ms_per_token().is_none());
        assert_eq!(profile.decode_samples(), 0);
    }

    #[test]
    fn predict_total_combines_prefill_and_decode() {
        let mut profile = BackendLatencyProfile::new();
        profile.record_prefill(Duration::from_millis(200));
        profile.record_decode(Duration::from_millis(50), 10);
        let predicted = profile
            .predict_total_ms(20)
            .expect("both components seeded");
        // 200 + 5 * 20 = 300
        assert!((predicted - 300.0).abs() < 1e-9);
    }

    #[test]
    fn warm_requires_min_samples_in_both_phases() {
        let mut profile = BackendLatencyProfile::new();
        for _ in 0..LATENCY_PROFILE_MIN_SAMPLES {
            profile.record_prefill(Duration::from_millis(100));
        }
        assert!(!profile.is_warm(), "no decode samples yet");
        for _ in 0..LATENCY_PROFILE_MIN_SAMPLES {
            profile.record_decode(Duration::from_millis(20), 5);
        }
        assert!(profile.is_warm());
    }

    #[test]
    fn store_isolates_per_backend_state() {
        let store = LatencyProfileStore::new();
        store.register_backend("alpha");
        store.register_backend("beta");
        store.record_prefill("alpha", Duration::from_millis(100));
        store.record_prefill("beta", Duration::from_millis(500));

        let alpha = store.snapshot("alpha").expect("alpha registered");
        let beta = store.snapshot("beta").expect("beta registered");
        assert!((alpha.prefill_ms().unwrap() - 100.0).abs() < 1e-9);
        assert!((beta.prefill_ms().unwrap() - 500.0).abs() < 1e-9);
    }
}
