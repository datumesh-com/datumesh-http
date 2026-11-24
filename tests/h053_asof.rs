//! http 0.5.3, CONTRACT-protocol-part1 H3 (D7, D7.1): the forms of an instant.
//! Written first; FAIL until then.
//!
//! Wherever a request takes an instant (the query `asof` of every read endpoint, and `asof` in the `/x/calc` and
//! `/whatif` bodies) these are accepted: `YYYY-MM-DDTHH:MM:SSZ` with 0 to 6 fractional digits, and a date, meaning
//! `YYYY-MM-DDT23:59:59.999999Z` of that day. They are canonicalised to six digits before use, and the answer's own
//! `asof` is the canonical instant. An offset other than `Z` is `E_CANON_INSTANT` (400 bad_request).
//!
//! Fixture (`common/h053.rs`): `px(USD, 2026-05-01)` is 1 at 2026-05-10T12:00:00.000000Z, 2 half a second later and 3
//! the next day at 12:00. So the date 2026-05-10 sees 2 (a cell written at 12:00 that day), and 2026-05-10's date does not
//! see the write of 05-11.
//!
//! Not testable: `/verify` has no `asof` in its body (it takes a receipt); see the report.

mod common;
#[path = "common/h053.rs"]
mod h053;

use common::{bl, now};
use datumesh_store::Store;
use h053::*;
use serde_json::{json, Value};

const CELL: &str = "ccy=USD&date=2026-05-01";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_cell_in_every_form_answers_as_the_canonical_form_does() {
    let a = px_world().await;
    for (form, canon, want) in FORMS {
        let r = a.get(&format!("/cell/{A}/px?{CELL}&asof={form}")).await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let c = a.get(&format!("/cell/{A}/px?{CELL}&asof={canon}")).await;
        assert_eq!(c.status, 200, "{canon}: {}", c.text());
        let (v, cv) = (jv(&r), jv(&c));
        assert_eq!(v["receipt"]["value"], *want, "{form}: {v}");
        assert_eq!(v["receipt"], cv["receipt"], "{form} answers as {canon}");
        assert_eq!(v["receipt"]["asof"], cv["receipt"]["asof"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_a_date_is_the_end_of_its_day() {
    let a = px_world().await;
    // written at 12:00:00.5 on 05-10: visible on the date 05-10; the write of 05-11 is not
    let r = jv(&a.get(&format!("/cell/{A}/px?{CELL}&asof=2026-05-10")).await);
    assert_eq!(r["receipt"]["value"], "2", "{r}");
    let r = jv(&a.get(&format!("/cell/{A}/px?{CELL}&asof=2026-05-11")).await);
    assert_eq!(r["receipt"]["value"], "3", "{r}");
    // the day before the first write: nothing was known
    let r = a.get(&format!("/cell/{A}/px?{CELL}&asof=2026-05-09")).await;
    assert_ne!(jv(&r)["receipt"]["value"], "1", "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_an_offset_is_still_refused() {
    let a = px_world().await;
    for bad in [
        "2026-05-10T12:00:00%2B01:00",
        "2026-05-10T12:00:00.5%2B01:00",
        "2026-05-10T12:00:00-05:00",
        "2026-05-10T12:00:00.1234567Z",
        "2026-05-10T12:00Z",
        "2026-05-1",
    ] {
        for path in [format!("/cell/{A}/px?{CELL}&asof={bad}"), format!("/slice/{A}/px?ccy=USD&date=*&asof={bad}")] {
            let r = a.get(&path).await;
            let v = jv(&r);
            assert_eq!(
                (r.status, v["outcome"].as_str(), v["error"].as_str()),
                (400, Some("bad_request"), Some("E_CANON_INSTANT")),
                "{path}: {}",
                r.text()
            );
        }
    }
    for (path, body) in [
        ("/x/calc", json!({"formula": format!("{A}/px(\"USD\", 2026-05-01)"), "asof": "2026-05-10T12:00:00+01:00"})),
        (
            "/whatif",
            json!({"cell": format!("{A}/plus_off?{CELL}"), "overrides": [], "asof": "2026-05-10T12:00:00+01:00", "solve": null}),
        ),
    ] {
        let r = a.post(path, serde_json::to_vec(&body).unwrap()).await;
        let v = jv(&r);
        assert_eq!((r.status, v["error"].as_str()), (400, Some("E_CANON_INSTANT")), "{path}: {}", r.text());
    }
}

/// `/slice` in JSON: the same members whatever the form, and the `next` link pins the canonical instant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_slice_json_and_arrow_pin_the_canonical_instant() {
    let a = px_world().await;
    for (form, canon, want) in FORMS {
        let r = a.get(&format!("/slice/{A}/px?ccy=USD&date=*&limit=1&asof={form}")).await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let v = jv(&r);
        let m = &v["members"][0];
        assert_eq!(m["receipt"]["value"], *want, "{form}: {v}");
        let next = v["next"].as_str().unwrap_or_else(|| panic!("{form}: no next: {v}"));
        let pinned = next.split("asof=").nth(1).unwrap().split('&').next().unwrap();
        assert_eq!(pinned.replace("%3A", ":"), *canon, "{form}: the next link pins the canonical instant");
        // Arrow: the schema metadata says the instant
        let r = a
            .get_with(
                &format!("/slice/{A}/px?ccy=USD&date=*&asof={form}"),
                &[("accept", "application/vnd.apache.arrow.stream")],
            )
            .await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let reader = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(&r.body), None).unwrap();
        assert_eq!(reader.schema().metadata().get("asof").map(String::as_str), Some(*canon), "{form}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_calc_in_every_form_answers_as_the_canonical_form_does() {
    let a = px_world().await;
    for (form, canon, want) in FORMS {
        let body = json!({"formula": format!("{A}/px(\"USD\", 2026-05-01)"), "asof": form});
        let r = a.post("/x/calc", serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let v = jv(&r);
        let ans = v.get("calculation").unwrap_or(&v);
        assert_eq!(ans["value"], *want, "{form}: {v}");
        assert_eq!(ans["asof"], *canon, "{form}: the answer's asof is the canonical instant");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_whatif_in_every_form_answers_as_the_canonical_form_does() {
    let a = px_world().await;
    // plus_off = px + off("x"); off("x") is overridden to 1000, so the scenario is px as of the instant + 1000
    let off_hash = bl(|| a.store.resolve_name(&format!("{A}/off"), &now()).unwrap().unwrap());
    let ov = json!([{"cell": {"ns": A, "fn": "off", "fn_hash": off_hash, "args": {"k": {"type": "string", "value": "x"}}},
                     "value": {"type": "number", "value": "1000"}}]);
    for (form, canon, want) in FORMS {
        let body = json!({"cell": format!("{A}/plus_off?{CELL}"), "overrides": ov, "asof": form});
        let r = a.post("/whatif", serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let v = jv(&r);
        let sc = &v["scenario"];
        let expect = (want.parse::<i64>().unwrap() + 1000).to_string();
        assert_eq!(sc["value"].as_str().map(|x| x.trim_end_matches(".0").to_string()), Some(expect), "{form}: {v}");
        assert_eq!(sc["asof"], *canon, "{form}: the scenario's asof is the canonical instant");
    }
}

/// What is stored does not change form: the same read at the canonical instant and at a short form names one receipt.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_the_receipt_of_a_short_form_read_is_the_stored_one() {
    let a = px_world().await;
    let short = jv(&a.get(&format!("/cell/{A}/px?{CELL}&asof=2026-05-10T12:00:00.5Z")).await);
    let canon = jv(&a.get(&format!("/cell/{A}/px?{CELL}&asof={V2}")).await);
    assert_eq!(short["receipt"], canon["receipt"]);
    let _: &Value = &short;
}
