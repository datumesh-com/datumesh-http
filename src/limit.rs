//! Rate limits: token buckets per client IP, per API key and, for `POST /verify`, per IP.
//! A refused request is answered at once (`rate_limited`, `retry_after_seconds`,
//! `Retry-After`): nothing waits for a token, and the check takes no pool permit.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::config::RateLimits;

struct Bucket {
    tokens: f64,
    last: Instant,
}

struct Buckets<K> {
    map: Mutex<HashMap<K, Bucket>>,
}

impl<K: Hash + Eq + Clone> Buckets<K> {
    fn new() -> Buckets<K> {
        Buckets { map: Mutex::new(HashMap::new()) }
    }

    /// Takes `cost` tokens from `k`'s bucket (capacity `burst`, refilled at `per_min` a
    /// minute), or the seconds until they would be there.
    fn take(&self, k: &K, cost: f64, per_min: u32, burst: u32) -> Result<(), u64> {
        let now = Instant::now();
        let rate = per_min as f64 / 60.0;
        let cap = burst.max(1) as f64;
        let mut m = self.map.lock().unwrap_or_else(|p| p.into_inner());
        if m.len() > 100_000 {
            // Forget buckets that are full again.
            m.retain(|_, b| b.tokens + now.duration_since(b.last).as_secs_f64() * rate < cap);
        }
        let b = m.entry(k.clone()).or_insert(Bucket { tokens: cap, last: now });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * rate).min(cap);
        b.last = now;
        if b.tokens >= cost {
            b.tokens -= cost;
            Ok(())
        } else if rate <= 0.0 {
            Err(60)
        } else {
            Err(((cost - b.tokens) / rate).ceil().max(1.0) as u64)
        }
    }
}

/// Who is asking.
#[derive(Clone, Debug)]
pub(crate) enum Client {
    Ip(IpAddr),
    Key(String, Option<u32>),
}

pub(crate) struct Limiter {
    cfg: RateLimits,
    ip: Buckets<IpAddr>,
    key: Buckets<String>,
    verify: Buckets<IpAddr>,
    pub(crate) refused: AtomicU64,
}

impl Limiter {
    pub(crate) fn new(cfg: RateLimits) -> Limiter {
        Limiter { cfg, ip: Buckets::new(), key: Buckets::new(), verify: Buckets::new(), refused: AtomicU64::new(0) }
    }

    /// Admits a request costing `cost` requests, or refuses it with the seconds to wait.
    pub(crate) fn admit(&self, client: &Client, cost: u32) -> Result<(), u64> {
        if !self.cfg.enabled {
            return Ok(());
        }
        let r = match client {
            Client::Ip(ip) => self.ip.take(ip, cost as f64, self.cfg.per_ip, self.cfg.ip_burst),
            Client::Key(id, own) => {
                let n = own.unwrap_or(self.cfg.per_key);
                self.key.take(id, cost as f64, n, n)
            }
        };
        if r.is_err() {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        r
    }

    /// Admits a `POST /verify` from `ip`.
    pub(crate) fn admit_verify(&self, ip: IpAddr) -> Result<(), u64> {
        if !self.cfg.enabled {
            return Ok(());
        }
        let r = self.verify.take(&ip, 1.0, self.cfg.verify_per_ip, self.cfg.verify_per_ip);
        if r.is_err() {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        r
    }

    /// The tokens a batch of `n` cells costs.
    pub(crate) fn batch_cost(&self, n: usize) -> u32 {
        let per = self.cfg.batch_cells_per_token.max(1) as usize;
        n.div_ceil(per).max(1) as u32
    }
}
