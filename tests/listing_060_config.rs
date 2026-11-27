//! http 0.6.0, the listing release (CONTRACT-http-listing.md L1): the setting `DATUMESH_LISTED_MESHES`, empty by default, and
//! what is refused at start. Unit tests on `Config::from_vars` and `Config::check`; failing on the test commit's stubs (the
//! fields exist and are neither read nor checked).
//!
//! Refused at start (`Config::from_vars`, and `Config::check` for a configuration built in code), with an error that names
//! `DATUMESH_LISTED_MESHES`: a host that is also in `DATUMESH_NAMESPACES` (the mesh's own host is one by default), a host
//! that is not a valid domain name, more than 64 hosts. `DATUMESH_LISTING_INTERVAL_SECS` (default 21,600) at least 1.

use std::time::Duration;

use datumesh_http::listing::{self, DEFAULT_INTERVAL, MAX_HOSTS};
use datumesh_http::Config;

const HOST: &str = "mesh-test.datumesh.com";

fn from(vars: &[(&str, &str)]) -> Result<Config, String> {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).map_err(|e| e.to_string())
}

#[track_caller]
fn refused(vars: &[(&str, &str)], what: &str) {
    match from(vars) {
        Ok(c) => panic!("{what}: accepted, listed {:?}", c.listed_meshes),
        Err(e) => assert!(e.contains("DATUMESH_LISTED_MESHES"), "{what}: the error names the setting: {e}"),
    }
}

fn hosts(n: usize) -> String {
    (0..n).map(|i| format!("m{i}.example")).collect::<Vec<_>>().join(",")
}

#[test]
fn l1_the_list_is_empty_by_default() {
    let c = from(&[]).unwrap();
    assert!(c.listed_meshes.is_empty());
    assert_eq!(c.listing_interval, Duration::from_secs(21_600));
    assert_eq!(DEFAULT_INTERVAL, Duration::from_secs(21_600));
    assert_eq!(c.listing_first_delay, Duration::ZERO);
    assert_eq!(MAX_HOSTS, 64);
    let empty = from(&[("DATUMESH_LISTED_MESHES", "")]).unwrap();
    assert!(empty.listed_meshes.is_empty(), "an empty value is the empty list");
}

#[test]
fn l1_the_list_is_comma_separated_hosts() {
    let c = from(&[("DATUMESH_LISTED_MESHES", " source.example, stats.other.example ,")]).unwrap();
    assert_eq!(c.listed_meshes, ["source.example", "stats.other.example"]);
    let c = from(&[
        ("DATUMESH_LISTED_MESHES", "source.example"),
        ("DATUMESH_LISTING_INTERVAL_SECS", "60"),
        ("DATUMESH_LISTING_FIRST_DELAY_SECS", "5"),
    ])
    .unwrap();
    assert_eq!(c.listing_interval, Duration::from_secs(60));
    assert_eq!(c.listing_first_delay, Duration::from_secs(5));
}

#[test]
fn l1_a_hosted_namespace_is_refused() {
    refused(&[("DATUMESH_LISTED_MESHES", HOST)], "the mesh's own host (a namespace by default)");
    refused(
        &[
            ("DATUMESH_NAMESPACES", &format!("{HOST},bankofengland.co.uk")),
            ("DATUMESH_LISTED_MESHES", "bankofengland.co.uk"),
        ],
        "a host of DATUMESH_NAMESPACES",
    );
    refused(
        &[("DATUMESH_NAMESPACES", &format!("{HOST},a.example")), ("DATUMESH_LISTED_MESHES", "b.example,a.example")],
        "one host of several",
    );
    // A host that is not hosted is fine, a sub-domain of a hosted one too (it is another namespace).
    let c = from(&[("DATUMESH_LISTED_MESHES", &format!("stats.{HOST}"))]).unwrap();
    assert_eq!(c.listed_meshes, [format!("stats.{HOST}")]);
}

#[test]
fn l1_an_invalid_host_is_refused() {
    for bad in
        ["Not A Host", "bad_host!.example", "https://source.example", "source.example/path", "-x.example", ".example"]
    {
        refused(&[("DATUMESH_LISTED_MESHES", bad)], bad);
    }
}

#[test]
fn l1_more_than_64_hosts_are_refused() {
    let c = from(&[("DATUMESH_LISTED_MESHES", &hosts(64))]).unwrap();
    assert_eq!(c.listed_meshes.len(), 64, "64 is the most, and allowed");
    refused(&[("DATUMESH_LISTED_MESHES", &hosts(65))], "65 hosts");
}

#[test]
fn l1_the_interval_is_at_least_one_second() {
    match from(&[("DATUMESH_LISTED_MESHES", "source.example"), ("DATUMESH_LISTING_INTERVAL_SECS", "0")]) {
        Ok(_) => panic!("an interval of 0 is accepted"),
        Err(e) => assert!(e.contains("DATUMESH_LISTING_INTERVAL_SECS"), "{e}"),
    }
}

/// `Config::check` refuses the same for a configuration built in code (the mesh builder checks it).
#[test]
fn l1_check_refuses_the_same_in_code() {
    let mut c = Config::new(HOST);
    c.listed_meshes = vec![HOST.to_string()];
    assert!(c.check().is_err(), "own host");
    c.listed_meshes = vec!["Not A Host".to_string()];
    assert!(c.check().is_err(), "invalid host");
    c.listed_meshes = hosts(65).split(',').map(str::to_string).collect();
    assert!(c.check().is_err(), "65 hosts");
    c.listed_meshes = vec!["source.example".to_string()];
    c.check().expect("a listed host that is not hosted");
}

/// How a host is reached: `https://<host>/`, unless `DATUMESH_PEERS` maps it to another origin.
#[test]
fn l1_a_host_is_reached_at_its_origin_or_its_peer() {
    let c = from(&[
        ("DATUMESH_LISTED_MESHES", "source.example,other.example"),
        ("DATUMESH_PEERS", "source.example=http://127.0.0.1:9"),
    ])
    .unwrap();
    assert_eq!(listing::origin_of(&c, "source.example"), "http://127.0.0.1:9");
    assert_eq!(listing::origin_of(&c, "other.example"), "https://other.example");
}
