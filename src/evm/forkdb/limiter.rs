//! Token-bucket rate limiter for RPC requests.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tracing::trace;

/// Optional token-bucket rate limiter implemented with a single atomic so
/// there is no mutex contention on the hot path.
#[derive(Debug)]
pub struct RateLimiter {
    // `None` means unlimited (zero rate limit).
    inner: Option<RateLimiterInner>,
    max_tokens: u64,
    interval_ms: u64,
}

#[derive(Debug)]
struct RateLimiterInner {
    state: AtomicU64,
    start: Instant,
}

impl RateLimiterInner {
    #[inline]
    fn pack(last_refill_ms: u32, tokens: u32) -> u64 {
        ((last_refill_ms as u64) << 32) | (tokens as u64)
    }

    #[inline]
    fn unpack(packed: u64) -> (u32, u32) {
        let last_refill_ms = (packed >> 32) as u32;
        let tokens = (packed & 0xFFFF_FFFF) as u32;
        (last_refill_ms, tokens)
    }
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

        let max_tokens = requests_per_second.min(u32::MAX as u64);
        let start = Instant::now();
        let state = AtomicU64::new(RateLimiterInner::pack(0, max_tokens as u32));

        Self {
            inner: Some(RateLimiterInner { state, start }),
            max_tokens,
            interval_ms: 1000,
        }
    }

    fn compute_refill(elapsed_ms: u64, max_tokens: u64, interval_ms: u64) -> u64 {
        ((elapsed_ms as u128).saturating_mul(max_tokens as u128) / (interval_ms as u128)) as u64
    }

    pub fn acquire(&self) {
        let Some(inner) = &self.inner else {
            return;
        };

        let max_tokens = self.max_tokens as u32;
        let interval_ms = self.interval_ms;

        loop {
            let result = inner
                .state
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |packed| {
                    let (last_refill_ms, tokens) = RateLimiterInner::unpack(packed);
                    let now_ms = inner.start.elapsed().as_millis() as u32;
                    let elapsed_ms = now_ms.wrapping_sub(last_refill_ms) as u64;
                    let add = Self::compute_refill(elapsed_ms, self.max_tokens, interval_ms) as u32;

                    let new_tokens = if add > 0 {
                        tokens.saturating_add(add).min(max_tokens)
                    } else {
                        tokens
                    };

                    if new_tokens >= 1 {
                        let new_tokens = new_tokens - 1;
                        let new_last_refill = if add > 0 { now_ms } else { last_refill_ms };
                        Some(RateLimiterInner::pack(new_last_refill, new_tokens))
                    } else {
                        None
                    }
                });

            match result {
                Ok(packed) => {
                    let (last_refill_ms, tokens) = RateLimiterInner::unpack(packed);
                    let now_ms = inner.start.elapsed().as_millis() as u32;
                    let elapsed_ms = now_ms.wrapping_sub(last_refill_ms) as u64;
                    let add = Self::compute_refill(elapsed_ms, self.max_tokens, interval_ms) as u32;
                    let new_tokens = if add > 0 {
                        tokens.saturating_add(add).min(max_tokens)
                    } else {
                        tokens
                    };
                    trace!(tokens = new_tokens - 1, "rate limit acquired");
                    return;
                }
                Err(packed) => {
                    let (last_refill_ms, _) = RateLimiterInner::unpack(packed);
                    let now_ms = inner.start.elapsed().as_millis() as u32;
                    let elapsed_ms = now_ms.wrapping_sub(last_refill_ms) as u64;
                    let n = elapsed_ms.saturating_mul(self.max_tokens);
                    let remainder = n % interval_ms;
                    let sleep_ms = (interval_ms - remainder).div_ceil(self.max_tokens);
                    trace!(sleep_ms, "rate limit sleep");
                    std::thread::sleep(Duration::from_millis(sleep_ms.max(1)));
                }
            }
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

    /// Regression: refill math must be smooth (multiply before divide) so that
    /// tokens accrue continuously rather than bursting at interval boundaries.
    #[test]
    fn rate_limiter_smooth_refill_regression() {
        // 100 req/sec: after 999 ms the buggy formula (elapsed/interval)*max_tokens
        // gives 0 tokens; the correct formula gives 99.
        assert_eq!(RateLimiter::compute_refill(999, 100, 1000), 99);
        // 2 req/sec: after 600 ms the buggy formula gives 0; correct gives 1.
        assert_eq!(RateLimiter::compute_refill(600, 2, 1000), 1);
        // 1000 req/sec: after 1 ms should give 1 token (per-ms refill).
        assert_eq!(RateLimiter::compute_refill(1, 1000, 1000), 1);
    }

    /// Regression: high-RPS rate limiter must sleep for interval/max_tokens
    /// (time per token) rather than the remainder of a 1-second window.
    #[test]
    fn rate_limiter_high_rps_sleep_duration() {
        let rps = 100;
        let limiter = RateLimiter::new(rps);
        // Exhaust the initial burst.
        for _ in 0..rps {
            limiter.acquire();
        }
        // The next acquire should wait ~10 ms (1000 ms / 100 rps), not ~1000 ms.
        let t0 = Instant::now();
        limiter.acquire();
        let elapsed = Instant::now().duration_since(t0).as_millis() as u64;
        assert!(
            elapsed < 200,
            "rate limiter slept {elapsed}ms, expected ~{}ms (time per token)",
            1000 / rps
        );
    }
}
