//! Retry logic with exponential backoff and error classification.
//!
//! Handles transient failures with intelligent retry strategies.

use rand::Rng;
use std::future::Future;
use std::time::Duration;

/// Retry strategy configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Backoff multiplier (usually 2.0)
    pub backoff_multiplier: f64,
    /// Whether to add jitter to delays
    pub jitter_enabled: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter_enabled: true,
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration.
    pub fn new(
        max_retries: usize,
        initial_delay: Duration,
        max_delay: Duration,
        backoff_multiplier: f64,
    ) -> Self {
        RetryConfig {
            max_retries,
            initial_delay,
            max_delay,
            backoff_multiplier,
            jitter_enabled: true,
        }
    }

    /// Disable jitter.
    pub fn without_jitter(mut self) -> Self {
        self.jitter_enabled = false;
        self
    }
}

/// Error classification for retry decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient error that should be retried (timeout, 5xx, rate limit)
    Retryable,
    /// Permanent error that should not be retried (auth, invalid request)
    Permanent,
    /// Unknown error classification
    Unknown,
}

/// Retry strategy executor.
#[derive(Clone)]
pub struct RetryStrategy {
    config: RetryConfig,
}

impl RetryStrategy {
    /// Create a new retry strategy with config.
    pub fn new(config: RetryConfig) -> Self {
        RetryStrategy { config }
    }

    /// Get the retry configuration.
    pub fn config(&self) -> &RetryConfig {
        &self.config
    }

    /// Classify an error for retry decisions.
    pub fn classify_error(&self, error: &anyhow::Error) -> ErrorClass {
        let error_str = error.to_string().to_lowercase();

        // Retryable errors
        if error_str.contains("timeout")
            || error_str.contains("timed out")
            || error_str.contains("connection")
            || error_str.contains("rate limit")
            || error_str.contains("429")
            || error_str.contains("500")
            || error_str.contains("502")
            || error_str.contains("503")
            || error_str.contains("504")
        {
            return ErrorClass::Retryable;
        }

        // Permanent errors
        if error_str.contains("unauthorized")
            || error_str.contains("forbidden")
            || error_str.contains("401")
            || error_str.contains("403")
            || error_str.contains("invalid")
            || error_str.contains("bad request")
            || error_str.contains("400")
            || error_str.contains("404")
        {
            return ErrorClass::Permanent;
        }

        // Default to unknown
        ErrorClass::Unknown
    }

    /// Determine if an error should be retried.
    pub fn should_retry(&self, error: &anyhow::Error, attempt: usize) -> bool {
        if attempt >= self.config.max_retries {
            return false;
        }

        match self.classify_error(error) {
            ErrorClass::Retryable => true,
            ErrorClass::Unknown => attempt < 1, // Retry once for unknown errors
            ErrorClass::Permanent => false,
        }
    }

    /// Calculate delay for next retry with exponential backoff and optional jitter.
    pub fn next_delay(&self, attempt: usize) -> Duration {
        let base_millis = self.config.initial_delay.as_millis() as f64
            * self.config.backoff_multiplier.powi(attempt as i32);

        let capped = base_millis.min(self.config.max_delay.as_millis() as f64);

        if self.config.jitter_enabled {
            // Add ±10% jitter
            let mut rng = rand::rng();
            let jitter_factor = rng.random_range(0.9..=1.1);
            let with_jitter = capped * jitter_factor;
            Duration::from_millis(with_jitter as u64)
        } else {
            Duration::from_millis(capped as u64)
        }
    }

    /// Execute an operation with automatic retry on failure.
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> anyhow::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let mut attempt = 0;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if !self.should_retry(&error, attempt) {
                        return Err(error);
                    }

                    let delay = self.next_delay(attempt);
                    tracing::debug!(
                        "Retry attempt {} after {:?} due to: {}",
                        attempt + 1,
                        delay,
                        error
                    );

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}

impl Default for RetryStrategy {
    fn default() -> Self {
        RetryStrategy::new(RetryConfig::default())
    }
}
