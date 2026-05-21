//! HTTP agent pool and URL pool for round-robin selection.

use std::sync::atomic::{AtomicUsize, Ordering};

use ureq::Agent;

/// Round-robin pool of `ureq::Agent` instances.
#[derive(Debug)]
pub struct AgentPool {
    agents: Vec<Agent>,
    idx: AtomicUsize,
}

impl AgentPool {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self {
            agents,
            idx: AtomicUsize::new(0),
        }
    }

    pub fn next(&self) -> &Agent {
        let idx = self.idx.fetch_add(1, Ordering::Relaxed) % self.agents.len().max(1);
        &self.agents[idx]
    }
}

/// Round-robin pool of JSON-RPC endpoint URLs.
#[derive(Debug)]
pub struct UrlPool {
    urls: Vec<String>,
    idx: AtomicUsize,
}

impl UrlPool {
    pub fn new(urls: Vec<String>) -> Self {
        Self {
            urls,
            idx: AtomicUsize::new(0),
        }
    }

    pub fn next(&self) -> &str {
        let idx = self.idx.fetch_add(1, Ordering::Relaxed) % self.urls.len().max(1);
        &self.urls[idx]
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_pool_round_robin() {
        let a1 = ureq::Agent::new_with_defaults();
        let a2 = ureq::Agent::new_with_defaults();
        let pool = AgentPool::new(vec![a1, a2]);

        let first = pool.next() as *const Agent;
        let second = pool.next() as *const Agent;
        let third = pool.next() as *const Agent;

        assert_ne!(first, second);
        assert_eq!(first, third);
    }

    #[test]
    fn url_pool_round_robin() {
        let pool = UrlPool::new(vec![
            "https://a.example.com".into(),
            "https://b.example.com".into(),
        ]);

        assert_eq!(pool.next(), "https://a.example.com");
        assert_eq!(pool.next(), "https://b.example.com");
        assert_eq!(pool.next(), "https://a.example.com");
    }
}
