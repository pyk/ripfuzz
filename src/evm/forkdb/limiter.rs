//! Token-bucket rate limiter for RPC requests.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::trace;

/// Optional token-bucket rate limiter.
///
/// State is protected by a `Mutex` so there are no atomic ordering concerns
/// and no background threads are spawned.
#[derive(Debug)]
pub struct RateLimiter {
    // `None` means unlimited (zero rate limit).
    inner: Option<Mutex<State>>,
    max_tokens: u64,
    interval_ms: u64,
}

#[derive(Debug)]
struct State {
    tokens: u64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: u64) -> Self {
        if requests_per_second == 0 {
            return Self {
                inner: None,
                max_tokens: 0,
                interval_ms: 1000,
            };
        }

        Self {
            inner: Some(Mutex::new(State {
                tokens: requests_per_second,
                last_refill: Instant::now(),
            })),
            max_tokens: requests_per_second,
            interval_ms: 1000,
        }
    }

    pub fn acquire(&self) {
        let Some(ref mutex) = self.inner else {
            return;
        };

        let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let now = Instant::now();
            let elapsed = now.duration_since(state.last_refill).as_millis() as u64;
            let tokens_to_add = (elapsed / self.interval_ms).saturating_mul(self.max_tokens);

            if tokens_to_add > 0 {
                state.tokens = (state.tokens + tokens_to_add).min(self.max_tokens);
                state.last_refill = now;
                trace!(tokens = state.tokens, "rate limit refilled");
            }

            if state.tokens >= 1 {
                state.tokens -= 1;
                trace!(tokens = state.tokens, "rate limit acquired");
                return;
            }

            let sleep_ms = self.interval_ms - (elapsed % self.interval_ms);
            trace!(sleep_ms, "rate limit sleep");
            drop(state);
            std::thread::sleep(Duration::from_millis(sleep_ms.max(1)));
            state = mutex.lock().unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;
    use std::time::Instant;

    use super::*;

    #[test]
    fn rate_limiter_allows_first_request() {
        let limiter = RateLimiter::new(10);
        let t0 = Instant::now();
        limiter.acquire();
        assert!(Instant::now().duration_since(t0).as_millis() < 50);
    }

    #[test]
    fn rate_limiter_throttles_when_bucket_exhausted() {
        let limiter = RateLimiter::new(2); // 2 req/sec
        limiter.acquire();
        limiter.acquire(); // exhaust bucket
        let t0 = Instant::now();
        limiter.acquire(); // must wait for refill
        let elapsed = Instant::now().duration_since(t0).as_millis();
        assert!(
            elapsed >= 400,
            "rate limiter should have throttled, elapsed: {elapsed}ms"
        );
    }

    /// Regression: a rate limit of 0 must mean "unlimited" (return
    /// immediately) rather than deadlocking in an infinite sleep loop.
    #[test]
    fn rate_limiter_zero_rps_does_not_deadlock() {
        let limiter = RateLimiter::new(0);
        let t0 = Instant::now();
        limiter.acquire();
        assert!(
            Instant::now().duration_since(t0).as_millis() < 50,
            "zero-rps limiter must not deadlock"
        );
    }

    /// Regression: the full bucket capacity must be available concurrently
    /// without any thread spuriously sleeping because of stale atomic reads.
    #[test]
    fn rate_limiter_burst_capacity_under_contention() {
        let limiter = Arc::new(RateLimiter::new(10));
        let barrier = Arc::new(Barrier::new(10));

        let handles: Vec<thread::JoinHandle<u128>> = (0..10)
            .map(|_| {
                let l = Arc::clone(&limiter);
                let b = Arc::clone(&barrier);
                thread::spawn(move || {
                    b.wait();
                    let t0 = Instant::now();
                    l.acquire();
                    t0.elapsed().as_millis()
                })
            })
            .collect();

        let mut times: Vec<u128> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        times.sort();

        // Every thread should acquire a token from the pre-filled bucket
        // within 100 ms; none should block waiting for a refill.
        assert!(
            times.last().copied().unwrap_or(0) < 100,
            "slowest thread in burst took {}ms",
            times.last().copied().unwrap_or(0)
        );
    }
}
