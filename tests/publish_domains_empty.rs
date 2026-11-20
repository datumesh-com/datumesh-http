//! D41 (QUESTIONS-build2): production stays closed to other namespaces. `DATUMESH_PUBLISH_DOMAINS` set
//! explicitly to the empty string behaves exactly as unset: the mesh's own namespaces only. A regression
//! test (it passes before 0.4.8 and must keep passing).

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use common::lite::*;
use datumesh_http::config::PublishDomains;
use publish4::*;

#[test]
fn an_empty_setting_parses_as_unset() {
    let cfg = |v: Option<&'static str>| {
        datumesh_http::Config::from_vars(move |k| match k {
            "DATUMESH_HOST" => Some(HOST.to_string()),
            "DATUMESH_PUBLISH_DOMAINS" => v.map(str::to_string),
            _ => None,
        })
        .unwrap()
    };
    let unset = cfg(None).publish_domains;
    assert_eq!(unset, PublishDomains::Own);
    for v in ["", " ", "  ", ",", " , "] {
        assert_eq!(cfg(Some(v)).publish_domains, unset, "{v:?}");
    }
    assert!(!unset.allows(PUB));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_setting_behaves_as_unset() {
    let stub = Stub::start(PUB).await;
    let mut answers = Vec::new();
    for (label, domains) in [("pde-unset", None), ("pde-empty", Some(""))] {
        let p = public(label, &[stub.peer()], domains).await;
        // another domain with a valid key document is refused, nothing written
        let r = p.post(simple_bundle(PUB)).await;
        assert_eq!(r.status, 403, "{label}: {}", r.text());
        p.refused(&r, "E_DOMAIN_NOT_ALLOWED");
        p.nothing_of(PUB);
        let refused = (r.status, r.json()["error"].clone());
        // the mesh's own namespace is accepted
        let own = p.post(simple_bundle(HOST)).await;
        assert_eq!(own.status, 201, "{label}: {}", own.text());
        answers.push((refused, own.json()["names"].clone()));
    }
    assert_eq!(answers[0], answers[1], "empty is exactly unset");
}
