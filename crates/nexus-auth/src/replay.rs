use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct NonceReplayCache {
    entries: HashMap<Vec<u8>, Instant>,
    ttl: Duration,
    capacity: usize,
}

impl NonceReplayCache {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            capacity: capacity.max(1),
        }
    }

    /// Returns true only for the first use of a nonce within the TTL window.
    pub fn accept(&mut self, nonce: &[u8], now: Instant) -> bool {
        self.prune(now);
        if self.entries.contains_key(nonce) {
            return false;
        }
        if self.entries.len() >= self.capacity {
            return false;
        }
        self.entries.insert(nonce.to_vec(), now);
        true
    }

    fn prune(&mut self, now: Instant) {
        self.entries
            .retain(|_, seen| now.saturating_duration_since(*seen) < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_replay_and_allows_expiry() {
        let now = Instant::now();
        let mut cache = NonceReplayCache::new(Duration::from_secs(5), 8);
        assert!(cache.accept(b"nonce", now));
        assert!(!cache.accept(b"nonce", now + Duration::from_secs(1)));
        assert!(cache.accept(b"nonce", now + Duration::from_secs(6)));
    }

    #[test]
    fn rejects_new_nonce_when_capacity_is_reached() {
        let now = Instant::now();
        let mut cache = NonceReplayCache::new(Duration::from_secs(60), 2);
        assert!(cache.accept(b"a", now));
        assert!(cache.accept(b"b", now + Duration::from_secs(1)));
        assert!(!cache.accept(b"c", now + Duration::from_secs(2)));
        assert!(!cache.accept(b"a", now + Duration::from_secs(3)));
    }
}
