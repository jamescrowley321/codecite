//! A point in time a wait must end by, or none when the timeout is too
//! large for an `Instant`, as `Duration::MAX` is.

use std::time::{Duration, Instant};

/// When a wait must end; `None` waits without limit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline(Option<Instant>);

impl Deadline {
    /// `timeout` after `start`, or no deadline if that overflows.
    pub(crate) fn at(start: Instant, timeout: Duration) -> Self {
        Self(start.checked_add(timeout))
    }

    /// `timeout` from now, or no deadline if that overflows.
    pub(crate) fn after(timeout: Duration) -> Self {
        Self::at(Instant::now(), timeout)
    }

    /// The time left, zero once passed and `Duration::MAX` with no deadline.
    pub(crate) fn remaining(self) -> Duration {
        self.0.map_or(Duration::MAX, |d| {
            d.saturating_duration_since(Instant::now())
        })
    }

    /// Whether the deadline has passed. Never true with no deadline.
    pub(crate) fn passed(self) -> bool {
        self.0.is_some_and(|d| Instant::now() >= d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overflowing_timeout_is_no_deadline() {
        let deadline = Deadline::after(Duration::MAX);
        assert!(!deadline.passed());
        assert_eq!(deadline.remaining(), Duration::MAX);
    }

    #[test]
    fn a_zero_timeout_has_passed() {
        let deadline = Deadline::after(Duration::ZERO);
        assert!(deadline.passed());
        assert_eq!(deadline.remaining(), Duration::ZERO);
    }
}
