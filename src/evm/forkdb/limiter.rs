use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tracing::trace;

/// Optional token-bucket rate limiter.
#[derive(Debug)]
pub struct RateLimiter {
    tokens: AtomicU64,
    last_refill: AtomicU64,
    max_tokens: u64,
    refill_interval_ms: u64,
    baseline: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: u64) -> Self {
        let max_tokens = requests_per_second;
        Self {
            tokens: AtomicU64::new(max_tokens),
            last_refill: AtomicU64::new(0),
            max_tokens,
            refill_interval_ms: 1000,
            baseline: Instant::now(),
        }
    }

    pub fn acquire(&self) {
        loop {
            let now_millis = self.baseline.elapsed().as_millis() as u64;
            let last_refill = self.last_refill.load(Ordering::Relaxed);
            let elapsed = now_millis.saturating_sub(last_refill);
            let tokens_to_add = (elapsed / self.refill_interval_ms).saturating_mul(self.max_tokens);

            let current_tokens = self.tokens.load(Ordering::Relaxed);
            let new_tokens = (current_tokens + tokens_to_add).min(self.max_tokens);

            if tokens_to_add > 0
                && self
                    .last_refill
                    .compare_exchange(
                        last_refill,
                        now_millis,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
            {
                self.tokens.store(new_tokens, Ordering::Relaxed);
            }

            let tokens_before = self.tokens.load(Ordering::Relaxed);
            if tokens_before >= 1 {
                if self
                    .tokens
                    .compare_exchange(
                        tokens_before,
                        tokens_before - 1,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    trace!(
                        tokens_before,
                        tokens_after = tokens_before - 1,
                        "rate limit acquired"
                    );
                    return;
                }
            } else {
                let sleep_ms = self
                    .refill_interval_ms
                    .saturating_sub(elapsed % self.refill_interval_ms);
                trace!(sleep_ms, "rate limit sleep");
                std::thread::sleep(Duration::from_millis(sleep_ms.max(1)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
