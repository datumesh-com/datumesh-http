//! SQLite stores and a fixed clock for the 1.8b tests (CONTRACT-1.8b H1, H2): a store in a
//! directory of its own, migrated, holding the key document of [`HOST`] (the test key from
//! 2026-01-01), removed when the value is dropped.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use datumesh_core::SigningKey;
use datumesh_engine::Clock;
use datumesh_store::{migrate, SqliteStore, Store};

use super::{key_for, out_dir};

/// The mesh host of the 1.8b tests.
pub const HOST: &str = "mesh-test.datumesh.com";
/// The fixed clock of the 1.8b tests.
pub const NOW: &str = "2026-11-15T12:00:00.000000Z";
/// The start of the test key of [`HOST`].
pub const K_FROM: &str = "2026-01-01T00:00:00.000000Z";
/// After every binding.
pub const END: &str = "9999-12-31T23:59:59.999999Z";

/// A clock that always reads the same instant.
pub struct FixedClock(pub &'static str);

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.0.to_string()
    }
}

pub struct Lite {
    pub store: Arc<SqliteStore>,
    pub key: SigningKey,
    pub dir: PathBuf,
}

impl Lite {
    pub fn new(label: &str) -> Lite {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = out_dir().join(format!("lite-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = SqliteStore::open(dir.join("store.db")).unwrap();
        migrate::sqlite::apply(&store).unwrap();
        let key = key_for(HOST);
        let doc = datumesh_http::keydoc::with_key(HOST, None, &key, K_FROM).unwrap();
        store.put_namespace(HOST, &doc, K_FROM).unwrap();
        Lite { store: Arc::new(store), key, dir }
    }

    /// A file in the store's directory.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The tx of `name`'s binding as of the end of time.
    pub fn bound_tx(&self, name: &str) -> Option<String> {
        self.store.resolve_binding(name, END).unwrap().map(|b| b.tx)
    }
}

impl Drop for Lite {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
