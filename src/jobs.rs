//! Jobs (SPEC 5.5): requests answered `pending`, held in memory.
//!
//! A job id is 32 random bytes from the operating system's CSPRNG, as 64 lowercase hex
//! digits (256 bits). A completed job's result is kept at least the configured TTL (300 s or
//! more) after it is first served, and the same after completion when it is never served;
//! then it is forgotten, and its id is unknown (`E_JOB_UNKNOWN`). A job's result is the
//! envelope its `/cell` request would have had, bound to the original request.
//!
//! Jobs live in the process: a restart forgets them, and a client then gets `E_JOB_UNKNOWN`
//! and sends the original request again, which the store (and, once warm, the memo) answers
//! at once, since the evaluation's receipt was stored when it completed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use datumesh_core::Request;

use crate::envelope::Reply;

struct Job {
    request: Request,
    created: Instant,
    result: Option<Reply>,
    completed: Option<Instant>,
    first_served: Option<Instant>,
}

/// What a poll finds.
pub(crate) enum Poll {
    /// Still running: the original request and the seconds it has run.
    Running(Request, u64),
    /// Done: its envelope.
    Done(Reply),
    /// Unknown or expired.
    Unknown,
}

pub(crate) struct Jobs {
    map: Mutex<HashMap<String, Job>>,
    ttl: Duration,
    max: usize,
    /// Mean duration of completed jobs, in milliseconds (an exponential moving average).
    ewma_ms: AtomicU64,
    pub(crate) created: AtomicU64,
    pub(crate) completed: AtomicU64,
}

impl Jobs {
    pub(crate) fn new(ttl: Duration, max: usize) -> Jobs {
        Jobs {
            map: Mutex::new(HashMap::new()),
            ttl,
            max,
            ewma_ms: AtomicU64::new(1000),
            created: AtomicU64::new(0),
            completed: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Job>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn expired(&self, j: &Job, now: Instant) -> bool {
        match (j.first_served, j.completed) {
            (Some(s), _) => now.duration_since(s) > self.ttl,
            (None, Some(c)) => now.duration_since(c) > self.ttl,
            (None, None) => false,
        }
    }

    /// A new job for `request`; `None` when the table is full (of running jobs).
    pub(crate) fn create(&self, request: Request) -> Option<String> {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).ok()?;
        let id = datumesh_core::lexical::to_hex(&b);
        let now = Instant::now();
        let mut m = self.lock();
        if m.len() >= self.max {
            m.retain(|_, j| !self.expired(j, now));
            if m.len() >= self.max {
                return None;
            }
        }
        m.insert(id.clone(), Job { request, created: now, result: None, completed: None, first_served: None });
        self.created.fetch_add(1, Ordering::Relaxed);
        Some(id)
    }

    /// Records a job's result.
    pub(crate) fn complete(&self, id: &str, reply: Reply) {
        let now = Instant::now();
        let mut m = self.lock();
        if let Some(j) = m.get_mut(id) {
            let ms = now.duration_since(j.created).as_millis() as u64;
            let old = self.ewma_ms.load(Ordering::Relaxed);
            self.ewma_ms.store((old * 3 + ms) / 4, Ordering::Relaxed);
            j.result = Some(reply);
            j.completed = Some(now);
            self.completed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The estimate for a job that has run `elapsed` seconds.
    pub(crate) fn estimate(&self, elapsed: u64) -> u64 {
        let mean = self.ewma_ms.load(Ordering::Relaxed).div_ceil(1000);
        mean.saturating_sub(elapsed).max(1)
    }

    /// A poll of `id`.
    pub(crate) fn poll(&self, id: &str) -> Poll {
        let now = Instant::now();
        let mut m = self.lock();
        let Some(j) = m.get_mut(id) else { return Poll::Unknown };
        if self.expired(j, now) {
            m.remove(id);
            return Poll::Unknown;
        }
        match &j.result {
            Some(r) => {
                j.first_served.get_or_insert(now);
                Poll::Done(r.clone())
            }
            None => Poll::Running(j.request.clone(), now.duration_since(j.created).as_secs()),
        }
    }

    /// A poll that does not count as serving the result.
    pub(crate) fn poll_peek(&self, id: &str) -> Poll {
        let m = self.lock();
        match m.get(id) {
            None => Poll::Unknown,
            Some(j) => match &j.result {
                Some(r) => Poll::Done(r.clone()),
                None => Poll::Running(j.request.clone(), j.created.elapsed().as_secs()),
            },
        }
    }

    /// Jobs held, and those still running.
    pub(crate) fn counts(&self) -> (usize, usize) {
        let m = self.lock();
        (m.len(), m.values().filter(|j| j.result.is_none()).count())
    }
}
