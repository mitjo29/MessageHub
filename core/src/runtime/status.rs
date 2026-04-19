use serde::{Deserialize, Serialize};

/// Per-channel health state. Persisted to `channels.status` as a lowercase string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelStatus {
    #[default]
    Healthy,
    /// Channel has failed 1..=3 times in a row.
    ///
    /// `attempt` mirrors `channels.consecutive_failures` in the DB and is
    /// populated from that column on load — it is not a separately-persisted
    /// field. Callers persisting `Degraded` must pass the matching
    /// `consecutive_failures` value to `Store::update_channel_status`.
    Degraded { attempt: u32 },
    Failed { last_error: String },
}

impl ChannelStatus {
    pub fn db_str(&self) -> &'static str {
        match self {
            ChannelStatus::Healthy => "healthy",
            ChannelStatus::Degraded { .. } => "degraded",
            ChannelStatus::Failed { .. } => "failed",
        }
    }
}

/// Threshold at which Degraded transitions to Failed.
pub const FAIL_THRESHOLD: u32 = 4;
/// Hard ceiling on backoff delay.
pub const MAX_BACKOFF_SECS: u64 = 600;

/// Tracks consecutive failures and derives the next poll delay.
///
/// Exponential base-2 with ±20% jitter, clamped to `MAX_BACKOFF_SECS`.
#[derive(Debug, Clone, Default)]
pub struct BackoffState {
    pub consecutive_failures: u32,
}

impl BackoffState {
    pub fn new() -> Self { Self { consecutive_failures: 0 } }

    /// Classify the current state into a `ChannelStatus`.
    pub fn status(&self, last_error: Option<&str>) -> ChannelStatus {
        if self.consecutive_failures == 0 {
            ChannelStatus::Healthy
        } else if self.consecutive_failures < FAIL_THRESHOLD {
            ChannelStatus::Degraded { attempt: self.consecutive_failures }
        } else {
            ChannelStatus::Failed {
                last_error: last_error.unwrap_or("unknown").to_string(),
            }
        }
    }

    /// Reset to Healthy after a successful fetch.
    pub fn record_success(&mut self) { self.consecutive_failures = 0; }

    /// Increment failure counter.
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Deterministic delay (seconds) before jitter. Used by callers that want
    /// testability; production code uses `next_delay`.
    pub fn base_delay_secs(&self, poll_interval_secs: u32) -> u64 {
        if self.consecutive_failures == 0 {
            return poll_interval_secs as u64;
        }
        let exp = self.consecutive_failures.min(16); // avoid shift overflow
        let raw = (poll_interval_secs as u64).saturating_mul(1u64 << exp);
        raw.min(MAX_BACKOFF_SECS)
    }

    /// Actual next delay with ±20% jitter applied via the injected RNG.
    pub fn next_delay_secs<R: rand::Rng>(&self, poll_interval_secs: u32, rng: &mut R) -> u64 {
        let base = self.base_delay_secs(poll_interval_secs);
        if base == 0 { return 0; }
        let jitter: f64 = rng.gen_range(-0.2..=0.2);
        let delayed = (base as f64 * (1.0 + jitter)).round() as i64;
        delayed.max(0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn fresh_state_is_healthy() {
        let s = BackoffState::new();
        assert_eq!(s.status(None), ChannelStatus::Healthy);
        assert_eq!(s.base_delay_secs(30), 30);
    }

    #[test]
    fn failures_progress_through_degraded_to_failed() {
        let mut s = BackoffState::new();
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 1 });
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 2 });
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 3 });
        s.record_failure();
        assert_eq!(
            s.status(Some("x")),
            ChannelStatus::Failed { last_error: "x".to_string() },
        );
    }

    #[test]
    fn success_resets_state() {
        let mut s = BackoffState::new();
        for _ in 0..5 { s.record_failure(); }
        s.record_success();
        assert_eq!(s.status(None), ChannelStatus::Healthy);
        assert_eq!(s.base_delay_secs(30), 30);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut s = BackoffState::new();
        let base = 30u32;
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 60);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 120);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 240);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 480);
        s.record_failure();
        // 30 * 32 = 960 → clamped to 600
        assert_eq!(s.base_delay_secs(base), MAX_BACKOFF_SECS);
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        let mut s = BackoffState::new();
        s.record_failure();
        let base = s.base_delay_secs(30);
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        for _ in 0..1000 {
            let d = s.next_delay_secs(30, &mut rng);
            let lo = (base as f64 * 0.8).floor() as u64;
            let hi = (base as f64 * 1.2).ceil()  as u64;
            assert!(d >= lo && d <= hi, "delay {} outside [{}, {}]", d, lo, hi);
        }
    }

    #[test]
    fn db_str_roundtrip() {
        assert_eq!(ChannelStatus::Healthy.db_str(), "healthy");
        assert_eq!(ChannelStatus::Degraded { attempt: 1 }.db_str(), "degraded");
        assert_eq!(
            ChannelStatus::Failed { last_error: "x".to_string() }.db_str(),
            "failed",
        );
    }
}
