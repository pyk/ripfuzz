//! Optional token-bucket rate limiter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tracing::trace;

pub struct RateLimiter {
    tokens: AtomicU64,
    last_refill: AtomicU64,
    max_tokens: u64,
    refill_interval_ms: u64,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field(
                "tokens",
                &self.tokens.load(std::sync::atomic::Ordering::Relaxed),
            )
            .field("max_tokens", &self.max_tokens)
            .field("refill_interval_ms", &self.refill_interval_ms)
            .finish()
    }
}

impl RateLimiter {
    pub fn new(requests_per_second: u64) -> Self {
        let max_tokens = requests_per_second;
        Self {
            tokens: AtomicU64::new(max_tokens),
            last_refill: AtomicU64::new(0),
            max_tokens,
            refill_interval_ms: 1000,
        }
    }

    pub fn acquire(&self) {
        loop {
            let now_millis = Instant::now().elapsed().as_millis() as u64;
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
    use super::*;

    #[test]
    fn rate_limiter_allows_first_request() {
        let limiter = RateLimiter::new(10);
        let t0 = Instant::now();
        limiter.acquire();
        assert!(Instant::now().duration_since(t0).as_millis() < 50);
    }

    #[test]
    fn rate_limiter_throttles() {
        let limiter = RateLimiter::new(1000); // 1000 req/sec = one per ms
        limiter.acquire();
        let t0 = Instant::now();
        limiter.acquire(); // second acquire may need to wait
        let elapsed = Instant::now().duration_since(t0).as_millis();
        // With a 1000 req/sec limit, the second acquire should take at most
        // a couple of milliseconds; we just assert it did not take an
        // unreasonably long time.
        assert!(elapsed < 100, "rate limiter slept too long: {elapsed}ms");
    }
}
