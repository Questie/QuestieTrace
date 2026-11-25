//! A limit on how fast a run downloads, shared by every thread of it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimit {
    bytes_per_second: f64,
    /// When everything taken so far will have been paid for. None before the first.
    paid_by: Mutex<Option<Instant>>,
}

impl RateLimit {
    pub fn new(bytes_per_second: u64) -> Self {
        RateLimit {
            bytes_per_second: bytes_per_second.max(1) as f64,
            paid_by: Mutex::new(None),
        }
    }

    /// Accounts for bytes that have arrived and waits until the limit allows reading on. A
    /// caller reads a download in small parts and calls this after each.
    ///
    /// That holds the average over a few seconds to the limit. A connection's buffer still
    /// takes in a few megabytes at the line's speed, so small objects arrive in short bursts
    /// with pauses between them.
    pub fn took(&self, bytes: u64) {
        std::thread::sleep(self.account(bytes, Instant::now()));
    }

    /// How long to wait at `now` after taking `bytes`. Time in which nothing was taken is not
    /// saved up: after a pause a run continues at the limit and not above it.
    fn account(&self, bytes: u64, now: Instant) -> Duration {
        let mut paid_by = self.paid_by.lock().expect("no holder panics");
        let from = paid_by.map_or(now, |paid_by| paid_by.max(now));
        let until = from + Duration::from_secs_f64(bytes as f64 / self.bytes_per_second);
        *paid_by = Some(until);
        until - now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_taken_is_paid_for_at_the_limit_by_everyone_together() {
        let limit = RateLimit::new(1000);
        let start = Instant::now();
        let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);

        // 500 bytes at 1,000 a second are half a second.
        assert_eq!(limit.account(500, at(0)), Duration::from_millis(500));
        // A second taker at the same moment pays after the first, not beside it.
        assert_eq!(limit.account(500, at(0)), Duration::from_millis(1000));
        // Part of what is owed has passed: only the rest is waited for.
        assert_eq!(limit.account(100, at(400)), Duration::from_millis(700));
        // All of it was paid at 1,100 ms. The nine seconds after that buy nothing.
        assert_eq!(limit.account(2000, at(10_000)), Duration::from_millis(2000));
    }
}
