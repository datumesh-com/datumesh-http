//! Build 2 Part 2, R1/R3: the demand record. A `/find` with no results and no resolution writes
//! its query text to the store's `find_misses` (`query`, `at`; no IP, no key, no user). The
//! contract offers no read surface, so the table is read with SQL on the test database
//! (`DATABASE_URL`, plain PostgreSQL 16 is enough: a miss needs no vectors).

mod common;

use common::readpath::*;

async fn pg() -> PgServed {
    serve_pg(&common::admin_url(), &lexicon_and_zoo(), None).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_miss_is_logged_with_its_query_text_only() {
    let m = pg().await;
    let v = m.find("quokka migration").await;
    assert!(results(&v).is_empty() && status(&v) == "none", "{v}");
    let rows = m.column("SELECT query FROM find_misses");
    assert_eq!(rows, ["quokka migration"]);
    // `at` is a timestamp; the table has no other column of a person or an address
    let cols = m
        .column("SELECT column_name::text FROM information_schema.columns WHERE table_name = 'find_misses' ORDER BY 1");
    for c in &cols {
        assert!(["at", "query", "id"].contains(&c.as_str()), "find_misses has a column {c}: {cols:?}");
    }
    assert!(cols.contains(&"query".to_string()) && cols.contains(&"at".to_string()), "{cols:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answered_queries_are_not_logged() {
    let m = pg().await;
    m.find("kestrel").await; // results
    m.find("John 3:16").await; // a resolution
    m.find("John 3:16-18").await; // a refused range is a resolution too
    assert!(m.column("SELECT query FROM find_misses").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_query_is_kept_to_512_bytes() {
    let m = pg().await;
    let q = "zq".repeat(300);
    m.find(&q).await;
    let rows = m.column("SELECT query FROM find_misses");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].len() <= 512 && q.starts_with(&rows[0]), "{} bytes", rows[0].len());
}
