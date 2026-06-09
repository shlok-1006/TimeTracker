//! In-memory ticket cache (per user) with a TTL. Serves stale entries when a
//! live fetch fails or is rate-limited (Requirement: cache + handle rate limits).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::Uuid;

/// A Linear ticket, shaped for the API response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub state: String,
    pub project: Option<String>,
    pub labels: Vec<String>,
    pub description_excerpt: String,
}

struct Entry {
    fetched_at: Instant,
    tickets: Vec<Ticket>,
}

pub struct TicketCache {
    inner: Mutex<HashMap<Uuid, Entry>>,
    ttl: Duration,
}

impl TicketCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Fresh tickets (within TTL), if any.
    pub fn fresh(&self, user_id: Uuid) -> Option<Vec<Ticket>> {
        let map = self.inner.lock().ok()?;
        map.get(&user_id)
            .filter(|e| e.fetched_at.elapsed() < self.ttl)
            .map(|e| e.tickets.clone())
    }

    /// Any cached tickets, regardless of age (stale fallback on fetch failure).
    pub fn stale(&self, user_id: Uuid) -> Option<Vec<Ticket>> {
        let map = self.inner.lock().ok()?;
        map.get(&user_id).map(|e| e.tickets.clone())
    }

    pub fn put(&self, user_id: Uuid, tickets: Vec<Ticket>) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(
                user_id,
                Entry {
                    fetched_at: Instant::now(),
                    tickets,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: &str) -> Ticket {
        Ticket {
            id: id.into(),
            title: "t".into(),
            state: "Todo".into(),
            project: None,
            labels: vec![],
            description_excerpt: String::new(),
        }
    }

    #[test]
    fn fresh_then_stale_after_ttl() {
        let cache = TicketCache::new(Duration::from_millis(50));
        let u = Uuid::new_v4();
        assert!(cache.fresh(u).is_none());
        cache.put(u, vec![ticket("a")]);
        assert_eq!(cache.fresh(u).unwrap().len(), 1); // fresh now
        std::thread::sleep(Duration::from_millis(80));
        assert!(cache.fresh(u).is_none()); // expired
        assert_eq!(cache.stale(u).unwrap().len(), 1); // still available as stale
    }
}
