//! `publish_stored` (the `publish-stored` subcommand): stored functions and their cells signed
//! by the mesh, blobs included, idempotently; a formula function (a `formula` line) over a blob
//! (`text_of`, `remove_chars`) evaluates and is signed by the mesh, trust `computed`.

mod common;

use std::io::Write;

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publish_stored_cells() {
    let a = Node::start(A, |_| {}).await;
    let dir = tempdir();
    let text = "בְּרֵאשִׁ֖ית בָּרָ֣א אֱלֹהִ֑ים אֵ֥ת הַשָּׁמַ֖יִם וְאֵ֥ת הָאָֽרֶץ׃".as_bytes();
    std::fs::write(dir.join("gen-1-1.txt"), text).unwrap();
    let lines = format!(
        r#"{{"function":"{A}/scripture/verse_text","signature":{{"args":{{"book":{{"type":"string","valid":false}},"chapter":{{"type":"number","unit":"1","valid":false}},"corpus":{{"type":"string","valid":false}},"edition":{{"type":"string","valid":false}},"verse":{{"type":"number","unit":"1","valid":false}}}},"order":["corpus","book","chapter","verse","edition"],"returns":"blob"}},"bound":"2020-01-01T00:00:00.000000Z"}}
{{"function":"{A}/ecb/fx_rate","signature":{{"args":{{"base":{{"type":"string","valid":false}},"date":{{"type":"date","valid":true}},"quote":{{"type":"string","valid":false}}}},"order":["base","quote","date"],"returns":{{"type":"number","unit":{{"of":{{"arg":"quote"}},"per":{{"arg":"base"}}}}}}}},"bound":"2020-01-01T00:00:00.000000Z"}}
{{"cell":{{"fn":"{A}/scripture/verse_text","args":{{"book":{{"type":"string","value":"Gen"}},"chapter":{{"type":"number","value":"1"}},"corpus":{{"type":"string","value":"tanakh"}},"edition":{{"type":"string","value":"uxlc"}},"verse":{{"type":"number","value":"1"}}}}}},"value":{{"blob":"gen-1-1.txt","media_type":"text/plain"}},"tx":"2026-03-01T00:00:00.000000Z","asof":"2026-03-01T00:00:00.000000Z","issued":"2026-03-01T00:00:00.000000Z","licence":"https://tanach.us/License.html","summary":"Test: Genesis 1:1, UXLC 2.6."}}
{{"cell":{{"fn":"{A}/ecb/fx_rate","args":{{"base":{{"type":"string","value":"EUR"}},"date":{{"type":"date","value":"2026-03-02"}},"quote":{{"type":"string","value":"USD"}}}}}},"value":{{"type":"number","value":"1.0832"}},"tx":"2026-03-02T16:00:00.000000Z","asof":"2026-03-02T16:00:00.000000Z","issued":"2026-03-02T16:00:00.000000Z","licence":"CC0-1.0","summary":"Synthetic test data."}}
"#
    );
    let path = dir.join("cells.jsonl");
    std::fs::File::create(&path).unwrap().write_all(lines.as_bytes()).unwrap();
    let key = a.key.clone();
    let first = bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &path).unwrap());
    assert_eq!((first.functions, first.receipts, first.versions, first.blobs), (2, 2, 2, 1), "{first:?}");
    let again = bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &path).unwrap());
    assert_eq!((again.functions, again.versions), (0, 0), "idempotent: {again:?}");

    // The verse by name: primary, a blob value whose bytes are the file's.
    let r =
        a.get(&format!("/cell/{A}/scripture/verse_text?book=Gen&chapter=1&corpus=tanakh&edition=uxlc&verse=1")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("primary"));
    let blob = r.json().get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str).unwrap().to_string();
    assert_eq!(blob, format!("{}:{}:text/plain", datumesh_core::sha256_hex(text), text.len()));
    let got = a.get(&format!("/blob/{}", &blob[..64])).await;
    assert_eq!(got.body, text);
    // Verified with the blob content (10.1 step 7.7).
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    let v = datumesh_core::verify(&receipt, &a.keys(), None, Some(text));
    assert!(v.ok && v.blob_checked, "{v:?}");

    // A derived edition: a formula over the blob, defined by a `formula` line.
    let sig = r#"{"args":{"book":{"type":"string","valid":false},"chapter":{"type":"number","unit":"1","valid":false},"verse":{"type":"number","unit":"1","valid":false}},"order":["book","chapter","verse"],"returns":"string"}"#;
    // The characters of the spec vector formula-eval/hebrew-consonantal (U+0591-U+05C7 but
    // maqaf, paseq, sof pasuq and nun hafukha), in its NFC order.
    let vector = datumesh_core::json::parse(
        &std::fs::read(spec_dir().join("vectors/formula-eval/hebrew-consonantal.json")).unwrap(),
    )
    .unwrap();
    let body = vector.get("world").and_then(|w| w.get("functions")).and_then(Value::as_object).unwrap()[0]
        .1
        .get("descriptor")
        .and_then(|d| d.get("code"))
        .and_then(|c| c.get("body"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let strip = body.rsplit_once(", \"").unwrap().1.split('"').next().unwrap().to_string();
    assert_eq!(strip.chars().count(), 51);
    let source = format!(
        r#"remove_chars(text_of({A}/scripture/verse_text(corpus: "tanakh", book: book, chapter: chapter, verse: verse, edition: "uxlc")), "{strip}")"#
    );
    let line = Value::Object(vec![
        ("bound".into(), Value::String("2020-01-01T00:00:00.000000Z".into())),
        ("formula".into(), Value::String(format!("{A}/scripture/strip_niqqud"))),
        ("signature".into(), datumesh_core::json::parse(sig.as_bytes()).unwrap()),
        ("source".into(), Value::String(source)),
    ]);
    let fpath = dir.join("formulas.jsonl");
    std::fs::write(&fpath, [line.to_jcs(), b"\n".to_vec()].concat()).unwrap();
    let f1 = bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &fpath).unwrap());
    assert_eq!((f1.functions, f1.cells), (1, 0), "{f1:?}");
    let f2 = bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &fpath).unwrap());
    assert_eq!(f2.functions, 0, "idempotent: {f2:?}");
    let r = a.get(&format!("/cell/{A}/scripture/strip_niqqud?book=Gen&chapter=1&verse=1")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("computed"));
    assert_eq!(
        r.json().get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str),
        Some("בראשית ברא אלהים את השמים ואת הארץ׃")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn tempdir() -> std::path::PathBuf {
    let d = out_dir().join(format!("publish-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
