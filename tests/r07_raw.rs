//! The 0.7 round (CONTRACT-round-0.7.md), R4: **test helpers send addresses as clients do**. `curl` and the libraries send `[`,
//! `]`, `(`, `)`, `,` and `:` raw in a path; only what SPEC 5.1 requires is percent-encoded (a space, `%`, `?`, `#`, and bytes that
//! are not ASCII). The shared helper `common::world::call_path` (and the `enc` of `pins.rs` and `syntax.rs`) used to encode every
//! other byte, so no test ever sent a raw bracket.
//!
//! These tests send the request line themselves, byte for byte, over a TCP stream (no URL library between the test and the mesh),
//! and assert that the line sent holds no `%` where none is required; one request per address form: an object, a label, a chain
//! and a short call. They also pin the helper's own contract.
//!
//! A double quote is the one byte that cannot be sent raw to this server: the `http` crate refuses it in a request target (400
//! before the mesh sees it), and `reqwest` and `curl --globoff` send it as `%22`. The object form with a quoted key is sent through
//! the stream with the quotes as `%22`, and through the helper; the forms with bare keys are sent raw by the stream.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::world::call_path;
use crworld::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The helper's contract: everything printable is raw, except a space, `%`, `?`, `#`; non-ASCII bytes are encoded.
#[test]
fn r4_the_helper_sends_what_clients_send_raw() {
    let raw = "mesh-a.example/shop/product[apple].price(2026-03-16)";
    assert_eq!(call_path(raw), raw, "brackets, parentheses, colons and commas are raw");
    let call = r#"mesh-a.example/shop/product(sku:"A1",date:2026-03-16)"#;
    assert_eq!(call_path(call), call, "a colon, a comma and a double quote are raw");
    assert_eq!(call_path("a b"), "a%20b");
    assert_eq!(call_path("a%b"), "a%25b");
    assert_eq!(call_path("a?b#c"), "a%3Fb%23c");
    assert_eq!(call_path("Z\u{fc}rich"), "Z%C3%BCrich", "non-ASCII bytes are encoded");
    assert_eq!(call_path("(x)[y]:*,@"), "(x)[y]:*,@");
}

/// GET `path` over a TCP stream, the request line exactly as given. Returns the status, the body, and the request line sent.
async fn raw_get(m: &common::readpath::Served, path: &str) -> (u16, serde_json::Value, String) {
    let addr = m.base.trim_start_matches("http://").to_string();
    let line = format!("GET {path} HTTP/1.1");
    let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let req = format!("{line}\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or_else(|| panic!("no header end: {buf:?}"));
    let head = String::from_utf8_lossy(&buf[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = buf[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut rest = &body[..];
        loop {
            let eol = rest.windows(2).position(|w| w == b"\r\n").unwrap();
            let n = usize::from_str_radix(std::str::from_utf8(&rest[..eol]).unwrap().trim(), 16).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&rest[eol + 2..eol + 2 + n]);
            rest = &rest[eol + 2 + n + 2..];
        }
        body = out;
    }
    let v = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("{path}: not JSON ({e}): {}", String::from_utf8_lossy(&body)));
    (status, v, line)
}

fn no_escapes(line: &str) {
    assert!(!line.contains('%'), "a request with nothing that needs encoding must carry no escape: {line}");
}

/// The object form: the key encoding of a product, `/describe/N(sku:"A1")`, sent as a client sends it: parentheses and the colon
/// raw, the two quotes `%22` (see the module documentation). The same address through the helper names the same object.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_an_object_address_is_sent_raw() {
    let m = shop("r07-raw-object", false).await;
    let n = product();
    let (status, v, line) = raw_get(&m, &format!("/describe/{n}(sku:%22A1%22)")).await;
    assert_eq!(line.matches('%').count(), 2, "only the two quotes are escaped: {line}");
    assert_eq!((status, v["kind"].as_str()), (200, Some("object")), "{line}: {v}");
    let p = format!("/describe/{}", call_path(&pk("A1")));
    assert!(p.contains(r#"(sku:"A1")"#), "the helper leaves the key encoding raw: {p}");
    let a = ask(&m, &p).await;
    assert_eq!((a.status, a.kind()), (200, Some("object")), "{p}: {}", a.text);
    assert_eq!(a.v["object_id"], v["object_id"], "the raw and the quoted spelling name one object");
}

/// The label form: `N[apple].price(day)` and the bare `N[apple]` (a describe): brackets and parentheses raw.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_a_label_address_is_sent_raw() {
    let m = shop("r07-raw-label", false).await;
    let n = product();
    let (status, v, line) = raw_get(&m, &format!("/cell/{n}[apple].price({SHOP_DAY})")).await;
    no_escapes(&line);
    assert_eq!(
        (status, v["outcome"].as_str(), v["receipt"]["value"].as_str()),
        (200, Some("ok"), Some("1.5")),
        "{line}: {v}"
    );
    let (status, v, line) = raw_get(&m, &format!("/cell/{n}[apple]")).await;
    no_escapes(&line);
    assert_eq!((status, v["kind"].as_str()), (200, Some("object")), "{line}: {v}");
}

/// The chain form: `N(A1).successor().price(day)`, two links, every parenthesis raw: the price of A1's successor P1 (3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_a_chain_address_is_sent_raw() {
    let m = shop("r07-raw-chain", false).await;
    let n = product();
    let (status, v, line) = raw_get(&m, &format!("/cell/{n}(A1).successor().price({SHOP_DAY})")).await;
    no_escapes(&line);
    assert_eq!(
        (status, v["outcome"].as_str(), v["receipt"]["value"].as_str()),
        (200, Some("ok"), Some("3")),
        "{line}: {v}"
    );
    let (status, v, line) = raw_get(&m, &format!("/cell/{n}[apple].successor().price({SHOP_DAY})")).await;
    no_escapes(&line);
    assert_eq!((status, v["receipt"]["value"].as_str()), (200, Some("3")), "{line}: {v}");
}

/// The short call: `N/price(A1,2026-03-16)` (positional, a comma) and `N/price(sku:A1,date:2026-03-16)` (labelled, colons):
/// parentheses, commas and colons raw. (No space: a space is one of the bytes that is encoded.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_a_short_call_address_is_sent_raw() {
    let m = shop("r07-raw-short", false).await;
    let n = product();
    for call in [format!("{n}/price(A1,{SHOP_DAY})"), format!("{n}/price(sku:A1,date:{SHOP_DAY})")] {
        let (status, v, line) = raw_get(&m, &format!("/cell/{call}")).await;
        no_escapes(&line);
        assert_eq!(
            (status, v["outcome"].as_str(), v["receipt"]["value"].as_str()),
            (200, Some("ok"), Some("1.5")),
            "{line}: {v}"
        );
    }
}
