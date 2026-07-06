//! Per-backend EWMA latency profile.
//!
//! Tracks prefill base latency and per-token decode cost using an
//! exponentially-weighted moving average so the runtime can build a stable
//! cost surface for backend-aware dispatch without keeping per-request
//! histograms in hot memory.
//!
//! It also tracks a coarser end-to-end request latency EWMA
//! (`record_request` / `request_ms`) that `HealthMonitor::record_success`
//! feeds on every successful call. That signal is what
//! `ModelRouter::select_from_table` reads for `RoutingTarget::Latency` — the
//! finer-grained prefill/decode breakdown (`record_prefill` / `record_decode`)
//! remains available for callers on the streaming path that can supply that
//! split (e.g. the runtime's `timing_tracker`), but request-level EWMA
//! wiring no longer waits on it.

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
    /// EWMA of whole-request (round-trip) latency in milliseconds, fed by
    /// `HealthMonitor::record_success` on every successful call. This is the
    /// coarse signal `RoutingTarget::Latency` ranks backends by — it doesn't
    /// need the prefill/decode split, just "how long did the last few calls
    /// to this backend take".
    request_ms_ewma: Option<f64>,
    request_samples: usize,
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
            request_ms_ewma: None,
            request_samples: 0,
        }
    }

    /// Record one whole-request (round-trip) latency observation. Folds into
    /// the EWMA with this profile's `alpha`: the first sample seeds the
    /// estimate directly (no artificial warm-up bias), every subsequent
    /// sample nudges it toward the new observation by `alpha`.
    pub fn record_request(&mut self, latency: Duration) {
        let sample_ms = duration_to_ms(latency);
        self.request_ms_ewma = Some(merge(self.request_ms_ewma, sample_ms, self.alpha));
        self.request_samples += 1;
    }

    /// Smoothed whole-request latency in milliseconds, or `None` if no
    /// successful request has been recorded yet for this backend.
    pub fn request_ms(&self) -> Option<f64> {
        self.request_ms_ewma
    }

    /// Number of whole-request samples folded into the EWMA.
    pub fn request_samples(&self) -> usize {
        self.request_samples
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

/// Thread-safe per-backend EWMA store.
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

    /// Reset a backend's profile to fresh (no samples), if it's registered.
    /// No-op for unregistered backends.
    pub fn reset_backend(&self, name: &str) {
        if let Some(entry) = self.profiles.get(name) {
            *entry.value().lock() = BackendLatencyProfile::new();
        }
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

    /// Record a whole-request latency observation for an already-registered
    /// backend. Silently a no-op for unknown backends.
    pub fn record_request(&self, name: &str, latency: Duration) {
        if let Some(entry) = self.profiles.get(name) {
            entry.value().lock().record_request(latency);
        }
    }

    /// Smoothed whole-request latency in milliseconds for `name`, or `None`
    /// if the backend is unregistered or has no successful samples yet.
    pub fn request_ms(&self, name: &str) -> Option<f64> {
        self.profiles
            .get(name)
            .and_then(|entry| entry.value().lock().request_ms())
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
    fn first_sample_seeds_the_ewma_directly() {
        let mut profile = BackendLatencyProfile::new();
        assert_eq!(profile.request_ms(), None);
        profile.record_request(Duration::from_millis(100));
        assert_eq!(profile.request_ms(), Some(100.0));
        assert_eq!(profile.request_samples(), 1);
    }

    #[test]
    fn ewma_converges_toward_recent_values_not_all_time_average() {
        // alpha = 0.5 for a fast-reacting profile in this test.
        let mut profile = BackendLatencyProfile::with_alpha(0.5);
        profile.record_request(Duration::from_millis(100));
        // 0.5*100 + 0.5*100 = 100
        profile.record_request(Duration::from_millis(100));
        assert_eq!(profile.request_ms(), Some(100.0));

        // A long run of low-latency samples should pull the estimate close to
        // the recent value, not to the simple all-time average (which would
        // sit near 100ms given the initial samples above).
        for _ in 0..20 {
            profile.record_request(Duration::from_millis(20));
        }
        let ms = profile.request_ms().unwrap();
        assert!(
            ms < 25.0,
            "expected EWMA to converge near the recent 20ms samples, got {ms}"
        );

        // And it should be far below a naive all-time average of the mixed
        // 100ms/20ms samples, proving this isn't just accumulating.
        let naive_average = (100.0 * 2.0 + 20.0 * 20.0) / 22.0;
        assert!(ms < naive_average);
    }

    #[test]
    fn low_alpha_smooths_more_than_high_alpha() {
        let mut smooth = BackendLatencyProfile::with_alpha(0.1);
        let mut reactive = BackendLatencyProfile::with_alpha(0.9);

        for profile in [&mut smooth, &mut reactive] {
            profile.record_request(Duration::from_millis(100));
        }
        for profile in [&mut smooth, &mut reactive] {
            profile.record_request(Duration::from_millis(20));
        }

        let smooth_ms = smooth.request_ms().unwrap();
        let reactive_ms = reactive.request_ms().unwrap();
        // The reactive (high alpha) profile should have moved further toward
        // the new 20ms sample than the smooth (low alpha) profile.
        assert!(reactive_ms < smooth_ms);
    }

    #[test]
    fn store_record_request_is_a_noop_for_unregistered_backend() {
        let store = LatencyProfileStore::new();
        // No panic, no crash — just silently ignored.
        store.record_request("ghost", Duration::from_millis(10));
        assert_eq!(store.request_ms("ghost"), None);
    }

    #[test]
    fn store_tracks_request_latency_per_backend() {
        let store = LatencyProfileStore::new();
        store.register_backend("fast");
        store.register_backend("slow");

        assert_eq!(store.request_ms("fast"), None);
        assert_eq!(store.request_ms("slow"), None);

        store.record_request("fast", Duration::from_millis(10));
        store.record_request("slow", Duration::from_millis(500));

        assert_eq!(store.request_ms("fast"), Some(10.0));
        assert_eq!(store.request_ms("slow"), Some(500.0));
    }
}
