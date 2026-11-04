//! A configurable base URL (0.2.0): where the mesh is served, for the endpoints of its mesh
//! document (`datumesh serve` advertising `http://127.0.0.1:<port>`). `https://` for any
//! host, `http://` for the loopback hosts only; an optional port; no path, query, fragment or
//! trailing slash; a typed error when the mesh is built. Unset, the mesh document is 0.1.2's,
//! byte for byte. The key document and `host` never change.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use datumesh_core::json::Value;
use datumesh_http::config::BaseUrlError;
use datumesh_http::{BuildError, Config, Mesh};
use datumesh_store::Store;

const A: &str = "mesh-a.example";

/// The endpoints of the mesh document, `bindings` first.
fn urls(doc: &Value) -> Vec<String> {
    let mut out = vec![doc.get("bindings").and_then(Value::as_str).unwrap().to_string()];
    for (_, v) in doc.get("endpoints").and_then(Value::as_object).unwrap() {
        out.push(v.as_str().unwrap().to_string());
    }
    out
}

/// What building a mesh for `A` with `base` gives (the store holds A's key document).
fn build(db: &TestDb, base: &str) -> Result<Mesh, BuildError> {
    bl(|| {
        let store = Arc::new(db.store(2));
        if store.namespace(A).unwrap().is_none() {
            store.put_namespace(A, &key_doc(A, &key_for(A)), BIND_TX).unwrap();
        }
        let shared: Arc<dyn Store + Send + Sync> = store;
        Mesh::builder(Config::new(A).with_base_url(base), shared).signing_key(key_for(A)).build()
    })
}

fn refused(db: &TestDb, base: &str) -> BaseUrlError {
    match build(db, base) {
        Ok(_) => panic!("{base} accepted"),
        Err(BuildError::BaseUrl(e)) => e,
        Err(e) => panic!("{base}: not a base URL error: {e}"),
    }
}

/// The mesh document of 0.1.2 for `A`, byte for byte.
const DOC_0_1_2: &str = concat!(
    r#"{"bindings":"https://mesh-a.example/bindings","endpoints":{"#,
    r#""blob":"https://mesh-a.example/blob/{sha256}","#,
    r#""cell":"https://mesh-a.example/cell/{cell}","#,
    r#""cells":"https://mesh-a.example/cells","#,
    r#""descriptor":"https://mesh-a.example/descriptor/{hash}","#,
    r#""explain":"https://mesh-a.example/explain/{cell}","#,
    r#""find":"https://mesh-a.example/find","#,
    r#""jobs":"https://mesh-a.example/jobs/{job}","#,
    r#""manifest":"https://mesh-a.example/manifest/{slice_id}/{members_hash}","#,
    r#""receipt":"https://mesh-a.example/receipt/{hash}","#,
    r#""slice":"https://mesh-a.example/slice/{cell}","#,
    r#""verify":"https://mesh-a.example/verify","#,
    r#""versions":"https://mesh-a.example/versions/{cell}"},"#,
    r#""host":"mesh-a.example","proto":"datumesh/1"}"#
);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_unchanged() {
    assert_eq!(Config::new(A).base_url, None);
    let a = Node::start(A, |_| {}).await;
    let md = a.get("/.well-known/datumesh-mesh.json").await;
    assert_eq!(md.status, 200);
    assert_eq!(md.text(), DOC_0_1_2);
    assert_eq!(a.mesh.mesh_document(), DOC_0_1_2.as_bytes());
    assert_eq!(a.mesh.config().origin(), format!("https://{A}"));
    // From the environment: unset is unset.
    let c = Config::from_vars(|k| (k == "DATUMESH_HOST").then(|| A.to_string())).unwrap();
    assert_eq!(c.base_url, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_http_accepted() {
    let db = TestDb::migrated();
    for base in
        ["http://127.0.0.1:8787", "http://127.0.0.1", "http://[::1]:8787", "http://localhost:9", "http://localhost"]
    {
        let m = build(&db, base).unwrap_or_else(|e| panic!("{base} refused: {e}"));
        let doc = datumesh_core::json::parse(&m.mesh_document()).unwrap();
        for u in urls(&doc) {
            assert!(u.starts_with(&format!("{base}/")), "{u} not under {base}");
        }
        assert_eq!(doc.get("host").and_then(Value::as_str), Some(A), "host unchanged");
        assert_eq!(m.config().origin(), base);
    }

    // Served: the mesh document uses the base URL; the key document and host are unchanged.
    let base = "http://127.0.0.1:8787";
    let a = Node::start(A, |c| {
        *c = c.clone().with_base_url(base);
        c.budget = Duration::from_millis(200);
    })
    .await;
    let md = a.get("/.well-known/datumesh-mesh.json").await;
    assert_eq!(md.status, 200);
    let expected = DOC_0_1_2.replace(&format!("https://{A}/"), &format!("{base}/"));
    assert_eq!(md.text(), expected);
    let kd = a.get("/.well-known/datumesh.json").await;
    assert_eq!(kd.status, 200);
    assert_eq!(kd.body, key_doc(A, &a.key), "key document unchanged");

    // A pending envelope's poll is the jobs endpoint with its job (SPEC 9.6), on the base.
    a.function(&format!("{A}/slow"), "sleep", "2000", &[], TAG_SIG);
    let r = a.get(&format!("/cell/{A}/slow?tag=x")).await;
    assert_eq!(r.status, 202, "{}", r.text());
    let env = r.json();
    let job = env.get("job").and_then(Value::as_str).unwrap();
    assert_eq!(env.get("poll").and_then(Value::as_str), Some(format!("{base}/jobs/{job}").as_str()));

    // From the environment.
    let c = Config::from_vars(|k| match k {
        "DATUMESH_HOST" => Some(A.to_string()),
        "DATUMESH_BASE_URL" => Some(base.to_string()),
        _ => None,
    })
    .unwrap();
    assert_eq!(c.base_url.as_deref(), Some(base));
}

#[test]
fn non_loopback_http_refused() {
    let db = TestDb::migrated();
    for base in [
        "http://mesh-a.example",
        "http://mesh-a.example:8080",
        "http://10.0.0.1:8787",
        "http://127.0.0.2:8787",
        "http://0.0.0.0:8787",
        "http://localhost.example",
        "http://[::2]:8787",
    ] {
        assert_eq!(refused(&db, base), BaseUrlError::HttpNotLoopback, "{base}");
    }
    for base in ["ftp://mesh-a.example", "mesh-a.example", "HTTPS://mesh-a.example", "//mesh-a.example"] {
        assert_eq!(refused(&db, base), BaseUrlError::Scheme, "{base}");
    }
    // From the environment: a configuration error.
    let r = Config::from_vars(|k| match k {
        "DATUMESH_HOST" => Some(A.to_string()),
        "DATUMESH_BASE_URL" => Some("http://mesh-a.example".to_string()),
        _ => None,
    });
    assert!(r.is_err());
}

#[test]
fn path_or_trailing_slash_refused() {
    let db = TestDb::migrated();
    for base in ["https://mesh-a.example/", "http://127.0.0.1:8787/", "https://mesh-a.example:8443/"] {
        assert_eq!(refused(&db, base), BaseUrlError::TrailingSlash, "{base}");
    }
    for base in ["https://mesh-a.example/mesh", "http://127.0.0.1:8787/a/b", "https://mesh-a.example/mesh/"] {
        assert_eq!(refused(&db, base), BaseUrlError::Path, "{base}");
    }
    for base in ["https://mesh-a.example?x=1", "https://mesh-a.example/?x=1"] {
        assert_eq!(refused(&db, base), BaseUrlError::Query, "{base}");
    }
    for base in ["https://mesh-a.example#top", "http://localhost#x"] {
        assert_eq!(refused(&db, base), BaseUrlError::Fragment, "{base}");
    }
    for base in ["https://", "https://user@mesh-a.example", "https://mesh a.example", "https://-a.example"] {
        assert_eq!(refused(&db, base), BaseUrlError::Host, "{base}");
    }
    for base in [
        "https://mesh-a.example:",
        "https://mesh-a.example:x",
        "https://mesh-a.example:65536",
        "https://mesh-a.example:0",
    ] {
        assert_eq!(refused(&db, base), BaseUrlError::Port, "{base}");
    }
}

#[test]
fn https_with_port_accepted() {
    let db = TestDb::migrated();
    for base in ["https://mesh-a.example:8443", "https://other.example", "https://127.0.0.1:8443", "https://[::1]:8443"]
    {
        let m = build(&db, base).unwrap_or_else(|e| panic!("{base} refused: {e}"));
        let doc = datumesh_core::json::parse(&m.mesh_document()).unwrap();
        for u in urls(&doc) {
            assert!(u.starts_with(&format!("{base}/")), "{u} not under {base}");
        }
        assert_eq!(doc.get("host").and_then(Value::as_str), Some(A));
    }
}
