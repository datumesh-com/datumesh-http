//! Build 2 Part 2, R2 and R3: the vector side of `/find`, on PostgreSQL with pgvector. These tests
//! run when `DATUMESH_PGVECTOR_URL` names a server of the `pgvector/pgvector:pg16` image whose role may
//! create databases (each test makes a database of its own); otherwise each one returns at once.
//! The embed service is the fake of `common/fake_embed.rs` (384-d hash vectors, `query: ` and
//! `passage: ` prefixes enforced).

mod common;

use common::fake_embed::FakeEmbed;
use common::readpath::*;
use serde_json::json;

macro_rules! need_pgvector {
    () => {
        match pgvector_url() {
            Some(u) => u,
            None => {
                eprintln!("DATUMESH_PGVECTOR_URL is not set: skipped");
                return;
            }
        }
    };
}

fn vec_hits(v: &serde_json::Value) -> bool {
    v["vector"] == json!(true) && results(v).iter().any(|r| r["match"] == "vector")
}

/// Exact, then text, then vector. `kestrel` is the label of `bird_a` (exact), a word of the
/// description of `bird_b` (text); `bird_c` (falcon sightings) shares no word, but the fake embeds
/// `falcon` as `kestrel`: vector. `vector` is true.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_then_text_then_vector() {
    let admin = need_pgvector!();
    let fake = FakeEmbed::start_with(&[("falcon", "kestrel")]);
    let m = serve_pg(&admin, &lexicon_and_zoo(), Some(config_with_embed(&fake.url()))).await;
    let v = m.find_until("kestrel", vec_hits).await;
    let kind = |n: &str| result(&v, n)["match"].as_str().unwrap().to_string();
    assert_eq!((kind(BIRD_A).as_str(), kind(BIRD_B).as_str(), kind(BIRD_C).as_str()), ("exact", "text", "vector"));
    assert!(pos(&v, BIRD_A) < pos(&v, BIRD_B) && pos(&v, BIRD_B) < pos(&v, BIRD_C), "{:?}", names(&v));
    // the ranks are never mixed: no exact after a text, no text after a vector
    let order: Vec<&str> = results(&v).iter().map(|r| r["match"].as_str().unwrap()).collect();
    let rank = |m: &str| ["exact", "text", "vector"].iter().position(|x| *x == m).unwrap();
    assert!(order.windows(2).all(|w| rank(w[0]) <= rank(w[1])), "{order:?}");
    for r in results(&v) {
        assert!(r["call"].is_string() && r["label"].is_string() && r["score"].is_number(), "{r}");
    }
    // R2: the query was embedded with the `query: ` prefix; indexed text with `passage: `.
    assert!(fake.queries().iter().any(|q| q == "query: kestrel"), "{:?}", fake.queries());
    assert!(!fake.passages().is_empty());
    assert!(fake.inputs().iter().all(|i| i.starts_with("query: ") || i.starts_with("passage: ")));
}

/// Vector fills the remaining places: with `limit=2` the vector result is cut, with 10 it is there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vector_fills_the_places_that_remain() {
    let admin = need_pgvector!();
    let fake = FakeEmbed::start_with(&[("falcon", "kestrel")]);
    let m = serve_pg(&admin, &lexicon_and_zoo(), Some(config_with_embed(&fake.url()))).await;
    m.find_until("kestrel", vec_hits).await;
    let v = m.get("/find?q=kestrel&limit=2").await;
    assert_eq!(names(&v), [BIRD_A, BIRD_B], "{v}");
}

/// Indexing is idempotent: a second mesh on the same database embeds nothing that already has an
/// embedding for the model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn indexing_is_idempotent() {
    let admin = need_pgvector!();
    let fake = FakeEmbed::start_with(&[("falcon", "kestrel")]);
    let m = serve_pg(&admin, &lexicon_and_zoo(), Some(config_with_embed(&fake.url()))).await;
    m.find_until("kestrel", vec_hits).await;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let first = fake.passages().len();
    assert!(first > 0);
    // a second mesh over the same store: same entries, same model
    let shared: std::sync::Arc<dyn datumesh_store::Store + Send + Sync> = m.store.clone();
    let mesh2 = datumesh_http::Mesh::builder(config_with_embed(&fake.url()), shared)
        .signing_key(common::key_for(common::lite::HOST))
        .clock(std::sync::Arc::new(common::lite::FixedClock(MESH_NOW)))
        .build()
        .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let t = tokio::spawn(async move {
        let _ = mesh2.serve(l, std::future::pending::<()>()).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    let v: serde_json::Value =
        serde_json::from_slice(&reqwest::get(format!("{base}/find?q=kestrel")).await.unwrap().bytes().await.unwrap()).unwrap();
    assert_eq!(v["vector"], json!(true), "{v}");
    assert_eq!(fake.passages().len(), first, "the second start embedded again");
    t.abort();
}

/// An embedding match alone never fills an argument: `iohannes` (the fake embeds it as `john`) is
/// near `John`, `1 John`... but is no dictionary value, label or alias. The resolution is not exact,
/// no `book` is filled, and the `book` key carries at most 3 labelled candidates, each with a value,
/// a label and a score.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_embedding_match_alone_gives_at_most_three_candidates() {
    let admin = need_pgvector!();
    let mut lines = lexicon_and_zoo();
    // four more books near John (each label has the word `john`): five matches by vector, three shown
    let dict = json!({"book": [
        {"value": "John", "label": "John"}, {"value": "1Jn", "label": "1 John"},
        {"value": "2Jn", "label": "2 John"}, {"value": "3Jn", "label": "3 John"},
        {"value": "Gen", "label": "Genesis"},
    ]});
    lines.retain(|l| !l.contains(r#""catalog":"mesh-test.datumesh.com/scratch_b2/lexicon/verse""#));
    lines.insert(
        1,
        catalog_line(VERSE, "function", "Synthetic Verses", VERSE_DESC, json!({"book": "the book code"}), dict),
    );
    let fake = FakeEmbed::start_with(&[("iohannes", "john")]);
    let m = serve_pg(&admin, &lines, Some(config_with_embed(&fake.url()))).await;
    let v = m
        .find_until("verse iohannes 3:16", |v| {
            v["vector"] == json!(true) && v["resolution"]["keys"].as_array().is_some_and(|k| !k.is_empty())
        })
        .await;
    assert_ne!(status(&v), "exact", "{v}");
    let keys = v["resolution"]["keys"].as_array().unwrap();
    let book = keys.iter().find(|k| k["key"] == "book").unwrap_or_else(|| panic!("no book key in {v}"));
    let cands = book["candidates"].as_array().unwrap();
    assert!((1..=3).contains(&cands.len()), "{book}");
    for c in cands {
        assert!(c["value"].is_string() && c["label"].is_string() && c["score"].is_number(), "{c}");
        assert!(c["label"].as_str().unwrap().to_lowercase().contains("john"), "{c}");
    }
    for k in keys {
        assert!(k["candidates"].as_array().unwrap().len() <= 3, "{k}");
    }
    // nothing was filled from it: no call with a book, in the resolution or the results
    let all = norm(&v.to_string().replace("\\\"", "\""));
    for b in ["John", "1Jn", "2Jn", "3Jn"] {
        assert!(!all.contains(&format!("book:\"{b}\"")), "{b} was filled: {all}");
    }
}

/// The embed service going away after start: `/find` still answers, `vector` false.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_service_stopping_later_gives_vector_false() {
    let admin = need_pgvector!();
    let fake = FakeEmbed::start_with(&[("falcon", "kestrel")]);
    let m = serve_pg(&admin, &lexicon_and_zoo(), Some(config_with_embed(&fake.url()))).await;
    m.find_until("kestrel", vec_hits).await;
    fake.set_fail(true);
    let v = m.find("kestrel").await;
    assert_eq!(v["vector"], json!(false), "{v}");
    assert!(results(&v).iter().all(|r| r["match"] != "vector"));
    assert_eq!(names(&v)[0], BIRD_A);
}
