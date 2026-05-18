//! Time source abstraction for the facilitator.
//!
//! The R1.1 verifier-policy hardening adds a time-window check
//! (`Authorization.valid_after <= now <= Authorization.valid_before`) per
//! `06-facilitator.md §6.6`. The check is implemented against the [`Clock`]
//! trait so tests can drive [`MockClock`] freezes and assert boundary
//! behavior deterministically; production paths use [`SystemClock`].
//!
//! The trait surface is intentionally narrow — Unix-seconds resolution
//! matches `Authorization.valid_after`/`valid_before` on the wire, and
//! anything finer would require a wider type on the spec side.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// Time source returning Unix-epoch seconds.
///
/// `now()` returns the seconds since `1970-01-01T00:00:00Z`, matching the
/// type of `Authorization.valid_after`/`valid_before`. Implementations
/// MUST be monotonic across calls within a single process — the verifier
/// asserts no upper-bound clock travel between checks.
pub trait Clock: Send + Sync {
    /// Current Unix-epoch time, in seconds.
    fn now(&self) -> u64;
}

/// Reads the OS wall clock. The default for production [`AppState`].
///
/// [`AppState`]: crate::AppState
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Test clock that holds a frozen value; tests advance it explicitly.
///
/// Use [`MockClock::frozen_at`] to construct, [`MockClock::set`] /
/// [`MockClock::advance`] to drive forward. Cheap to clone — the
/// underlying state is shared via `Arc<AtomicI64>`.
#[derive(Debug, Clone)]
pub struct MockClock {
    inner: Arc<AtomicI64>,
}

impl MockClock {
    /// Construct a clock frozen at the given Unix-seconds value.
    pub fn frozen_at(now: u64) -> Self {
        Self {
            inner: Arc::new(AtomicI64::new(now as i64)),
        }
    }

    /// Overwrite the current time. Used by tests to step the clock.
    pub fn set(&self, now: u64) {
        self.inner.store(now as i64, Ordering::SeqCst);
    }

    /// Advance the clock by `delta` seconds. May be negative for
    /// rewind tests (intentionally permissive — the trait does not
    /// require monotonicity in test fixtures).
    pub fn advance(&self, delta: i64) {
        self.inner.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now(&self) -> u64 {
        let raw = self.inner.load(Ordering::SeqCst);
        if raw < 0 {
            0
        } else {
            raw as u64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_nonzero_recent_time() {
        let now = SystemClock.now();
        assert!(now > 1_700_000_000, "SystemClock returned suspiciously old value: {now}");
    }

    #[test]
    fn mock_clock_freezes_at_constructor_value() {
        let clock = MockClock::frozen_at(1_000);
        assert_eq!(clock.now(), 1_000);
        // Re-reading does not advance the clock.
        assert_eq!(clock.now(), 1_000);
    }

    #[test]
    fn mock_clock_set_overwrites() {
        let clock = MockClock::frozen_at(1_000);
        clock.set(2_500);
        assert_eq!(clock.now(), 2_500);
    }

    #[test]
    fn mock_clock_advance_steps_forward() {
        let clock = MockClock::frozen_at(1_000);
        clock.advance(60);
        assert_eq!(clock.now(), 1_060);
    }

    #[test]
    fn mock_clock_clone_shares_state() {
        let a = MockClock::frozen_at(1_000);
        let b = a.clone();
        a.set(2_000);
        assert_eq!(b.now(), 2_000, "Clone must share the underlying time");
    }
}
