//! The bounded blocking pool: the engine and the store are synchronous, so every request's
//! work runs on a `spawn_blocking` thread holding one of `size` permits. The pool's size is
//! the mesh's concurrency limit.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::Semaphore;

/// The pool and its counters.
#[derive(Debug)]
pub(crate) struct Pool {
    sem: Arc<Semaphore>,
    pub(crate) size: usize,
    pub(crate) in_use: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    /// Tasks that found every permit taken and waited.
    pub(crate) waited: AtomicU64,
    pub(crate) runs: AtomicU64,
}

struct InUse<'a>(&'a AtomicUsize);

impl Drop for InUse<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Pool {
    pub(crate) fn new(size: usize) -> Pool {
        Pool {
            sem: Arc::new(Semaphore::new(size)),
            size,
            in_use: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            waited: AtomicU64::new(0),
            runs: AtomicU64::new(0),
        }
    }

    /// Runs `f` on a blocking thread once a permit is free. `None` if the task panicked.
    pub(crate) async fn run<T: Send + 'static>(self: &Arc<Self>, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let permit = match self.sem.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                self.waited.fetch_add(1, Ordering::Relaxed);
                self.sem.clone().acquire_owned().await.ok()?
            }
        };
        self.runs.fetch_add(1, Ordering::Relaxed);
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let n = me.in_use.fetch_add(1, Ordering::Relaxed) + 1;
            me.peak.fetch_max(n, Ordering::Relaxed);
            let _guard = InUse(&me.in_use);
            let r = f();
            drop(permit);
            r
        })
        .await
        .ok()
    }
}
