//! http 0.6.0, the listing release (CONTRACT-http-listing.md L3, L5, *[25 Nov]*): on PostgreSQL with pgvector, the listing
//! mesh's store against the source mesh in process. An embedding is made only for a new name or a changed descriptor: the
//! embedding job embeds what `pending_embeddings` gives (catalogue rows with no vector), so a run with nothing changed
//! leaves nothing pending, writes no `listings` row and no catalogue cell. Listed names are given embeddings (L5), so the
//! vector match finds them. Runs when `DATUMESH_PGVECTOR_URL` names a server of the `pgvector/pgvector:pg16` image whose
//! role may create databases (skipped otherwise, as `find_vector.rs` is). Failing on the test commit (`run_host` is
//! `todo!()`).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::sync::Arc;

use catworld::LICENCE;
use common::lite::*;
use common::readpath::pgvector_url;
use common::*;
use datumesh_http::listing::{self, Limits};
use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};
use datumesh_store::{migrate, EmbedRef, Embedding, PgStore, PoolOptions, Store};
use listing060::*;

const MODEL: &str = "intfloat/multilingual-e5-small";

struct Db {
    admin: String,
    name: String,
    url: String,
}

impl Drop for Db {
    fn drop(&mut self) {
        let (admin, name) = (self.admin.clone(), self.name.clone());
        on_thread(move || {
            let cfg: PgConfig = admin.parse().unwrap();
            if let Ok(mut c) = cfg.connect(NoTls) {
                let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
            }
        });
    }
}

fn db(admin: &str) -> Db {
    let name = format!("dm_l060_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos());
    let (path, query) = admin.split_once('?').map(|(p, q)| (p, Some(q))).unwrap_or((admin, None));
    let base = path.rsplit_once('/').unwrap().0;
    let url = match query {
        Some(q) => format!("{base}/{name}?{q}"),
        None => format!("{base}/{name}"),
    };
    let (a, n) = (admin.to_string(), name.clone());
    on_thread(move || {
        let cfg: PgConfig = a.parse().unwrap();
        cfg.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {n}")).unwrap();
    });
    Db { admin: admin.to_string(), name, url }
}

fn count(url: &str, sql: &str) -> i64 {
    let (url, sql) = (url.to_string(), sql.to_string());
    on_thread(move || {
        let cfg: PgConfig = url.parse().unwrap();
        cfg.connect(NoTls).unwrap().query_one(sql.as_str(), &[]).unwrap().get(0)
    })
}

fn stamps(url: &str) -> Vec<String> {
    let url = url.to_string();
    on_thread(move || {
        let cfg: PgConfig = url.parse().unwrap();
        let mut c = cfg.connect(NoTls).unwrap();
        c.query("SELECT kind || ' ' || name || ' ' || updated_at::text FROM catalog_entries ORDER BY 1", &[])
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect()
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_a_quiet_run_makes_no_embedding_on_postgres() {
    let Some(admin) = pgvector_url() else {
        eprintln!("DATUMESH_PGVECTOR_URL is not set: skipped");
        return;
    };
    let d = db(&admin);
    let url = d.url.clone();
    let store: Arc<PgStore> = Arc::new(on_thread(move || {
        let first = PgStore::connect_with(&url, PoolOptions::new().max_size(4)).unwrap();
        migrate::apply(&first).unwrap();
        drop(first);
        let s = PgStore::connect_with(&url, PoolOptions::new().max_size(4)).unwrap();
        s.put_namespace(HOST, &key_doc(HOST, &key_for(HOST)), K_FROM).unwrap();
        s
    }));
    assert!(store.vector_available(), "the server has pgvector");
    let src = source("l060-pg", &[]).await;
    let origin = src.proxy.base.clone();
    let run = |now: &'static str| {
        let (s, o) = (store.clone(), origin.clone());
        on_thread(move || listing::run_host(&*s, SRC, &o, &Limits::default(), now).unwrap())
    };
    let build_at = |clock: &'static str| {
        let s = store.clone();
        on_thread(move || {
            datumesh_http::catalog_build::catalog_build_with(&*s, &key_for(HOST), HOST, LICENCE, &common::pub031::opts(clock))
                .unwrap()
                .cells
        })
    };
    let pending = || {
        let s = store.clone();
        on_thread(move || s.pending_embeddings(MODEL, 1000).unwrap())
    };

    // The first run: a new name, so one entry to embed (the listed name's text).
    let a = run(RUN1);
    assert_eq!(a.added, [bank_rate()]);
    let p = pending();
    assert_eq!(p.len(), 1, "one pending embedding, the new name's: {p:?}");
    assert!(p[0].text.contains(LABEL) && p[0].text.contains(DESCRIPTION), "{p:?}");
    // The embedding job embeds it (here: a fixed unit vector).
    let mut v = vec![0.0f32; 384];
    v[0] = 1.0;
    let items: Vec<Embedding> = p.iter().map(|x| Embedding { ref_kind: x.ref_kind, ref_id: x.ref_id, vec: v.clone() }).collect();
    assert!(items.iter().all(|e| e.ref_kind == EmbedRef::Entry));
    let s = store.clone();
    on_thread(move || s.put_embeddings(MODEL, &items).unwrap());
    assert!(build_at(catworld::T3) > 0);
    // The vector match finds the listed name.
    let s = store.clone();
    let hits = on_thread(move || s.vector_find(MODEL, &v, 5).unwrap());
    assert!(
        hits.iter().any(|h| matches!(h, datumesh_store::Hit::Entry { entry, .. } if entry.name == bank_rate())),
        "the listed name has its embedding: {hits:?}"
    );

    // A quiet run: nothing written, nothing pending, no catalogue cell.
    let before = (
        count(&d.url, "SELECT count(*) FROM listings"),
        count(&d.url, "SELECT count(*) FROM listed_hosts"),
        count(&d.url, "SELECT count(*) FROM embeddings"),
        count(&d.url, "SELECT count(*) FROM catalog_entries"),
        count(&d.url, "SELECT count(*) FROM receipts"),
        stamps(&d.url),
    );
    let a = run(RUN1B);
    assert!(!a.changed_any(), "{a:?}");
    assert_eq!(a.writes, 0, "{a:?}");
    assert_eq!(pending(), vec![], "no embedding to make");
    let after = (
        count(&d.url, "SELECT count(*) FROM listings"),
        count(&d.url, "SELECT count(*) FROM listed_hosts"),
        count(&d.url, "SELECT count(*) FROM embeddings"),
        count(&d.url, "SELECT count(*) FROM catalog_entries"),
        count(&d.url, "SELECT count(*) FROM receipts"),
        stamps(&d.url),
    );
    assert_eq!(after, before, "row counts and the catalogue's timestamps before and after");
    assert_eq!(build_at(catworld::T4), 0, "no catalogue cell");
    std::mem::forget(store);
}
