//! The test classes of the 0.3.0 tests (spec-0.5.0, section 16), modelled on the live check of the
//! test mesh (the seed's descriptors carry no key rule):
//!
//! * `<h>/scripture/verse`: keys `corpus`, `book`, `chapter`, `verse`, all `none`; methods `text`
//!   (a stored function like the seed's `verse_text`, with an `edition` argument and a blob value)
//!   and `consonantal` (a formula, the keys first);
//! * `<h>/ecb/fx_pair`: keys `base`, `quote`, `none`; methods `rate` (stored like `fx_rate`) and
//!   `cross` (a formula whose `date` has `"default": "asof"`);
//!
//! and, for what those cannot show (a rule on a key, a label, a dispatch):
//!
//! * `<h>/ecb/fx_upper`: keys `base`, `quote` under `upper_ascii`; method `rate` (stored; its key
//!   arguments carry the rule, 16.2) and the formulas `upper_rate(pair, date)`, which dispatches
//!   `rate` on the key `pair` of a class of the set `[fx_upper]` (16.6), and `lower_rate(x, date)`,
//!   which reads `rate` with a key it computes (`lower(x)`: the fault `E_FORMULA_KEY`);
//! * `<h>/ecb/fx_named`: keys `base`, `quote`, `none`; methods `rate` and `named`, the class's
//!   `lookup` (a label to a key, 16.7).
//!
//! [`install`] builds the world straight in the node's store (the method functions bound from
//! `BIND_TX`, the classes from [`TC`], the dispatch formulas from [`TD`], the cells at [`TV`]),
//! with `formula` and the store's own API, not through `publish-stored`.
#![allow(dead_code)]

use datumesh_core::json::Value;
use datumesh_core::{ClassDescriptor, FunctionDescriptor, Scalar, Type};
use datumesh_formula::definition::{define_class, define_function, publish, publish_class};

use super::*;

/// The classes are bound from here (after the method functions, bound at `BIND_TX`).
pub const TC: &str = "2020-01-02T00:00:00.000000Z";
/// The formulas that dispatch are bound from here (after the classes).
pub const TD: &str = "2020-01-03T00:00:00.000000Z";
/// The tx and asof of every stored cell of the world.
pub const TV: &str = "2026-03-02T16:00:00.000000Z";
/// The date of the world's FX cells.
pub const DAY: &str = "2026-03-02";
/// An asof after the cells, whose date is [`DAY`].
pub const ASOF: &str = "2026-03-02T18:00:00.000000Z";
/// Genesis 1:1, as the seed has it.
pub const GEN_1_1: &str = "בְּרֵאשִׁ֖ית בָּרָ֣א אֱלֹהִ֑ים אֵ֥ת הַשָּׁמַ֖יִם וְאֵ֥ת הָאָֽרֶץ׃";
/// Its consonants, as `consonantal` returns them.
pub const GEN_1_1_CONSONANTAL: &str = "בראשית ברא אלהים את השמים ואת הארץ׃";

/// Percent-encodes a call for a URL path as a client sends it (CONTRACT-round-0.7 R4): `[`, `]`, `(`, `)`, `,`, `:`, `"` and
/// every other printable ASCII byte go raw, as `curl` and the libraries send them; only what SPEC 5.1 requires is encoded:
/// a space, `%`, `?`, `#`, and bytes that are not ASCII.
pub fn call_path(call: &str) -> String {
    let mut out = String::new();
    for b in call.bytes() {
        if b.is_ascii_graphic() && !b"%?#".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encodes a value of a cell URI's query (SPEC 5.1): unreserved characters and `:` as they
/// are, every other byte `%XX` in uppercase.
pub fn qenc(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The world's names and class hashes.
pub struct World {
    pub host: String,
    pub verse: String,
    pub pair: String,
    pub upper: String,
    pub named: String,
    pub verse_hash: String,
    pub pair_hash: String,
    pub upper_hash: String,
    pub named_hash: String,
}

/// The 51 characters `consonantal` removes: the spec vector's `formula-eval/hebrew-consonantal`.
pub fn strip_chars() -> String {
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
    body.rsplit_once(", \"").unwrap().1.split('"').next().unwrap().to_string()
}

const STR: &str = r#"{"type":"string","valid":false}"#;
const NUM: &str = r#"{"type":"number","unit":"1","valid":false}"#;
const UPPER: &str = r#"{"key_normalisation":"upper_ascii","type":"string","valid":false}"#;

/// `verse/text`: the seed's `verse_text`.
pub fn verse_text_sig() -> String {
    format!(
        r#"{{"args":{{"book":{STR},"chapter":{NUM},"corpus":{STR},"edition":{STR},"verse":{NUM}}},"order":["corpus","book","chapter","verse","edition"],"returns":"blob"}}"#
    )
}

/// `verse/consonantal`: the keys of the class, first.
pub fn verse_consonantal_sig() -> String {
    format!(
        r#"{{"args":{{"book":{STR},"chapter":{NUM},"corpus":{STR},"verse":{NUM}}},"order":["corpus","book","chapter","verse"],"returns":"string"}}"#
    )
}

const FX_UNIT: &str = r#"{"type":"number","unit":{"of":{"arg":"quote"},"per":{"arg":"base"}}}"#;

/// `fx_pair/rate`: the seed's `fx_rate`.
pub fn fx_rate_sig() -> String {
    format!(
        r#"{{"args":{{"base":{STR},"date":{{"type":"date","valid":true}},"quote":{STR}}},"order":["base","quote","date"],"returns":{FX_UNIT}}}"#
    )
}

/// `fx_pair/cross`: `date` defaults to the asof (16.7).
pub fn fx_cross_sig() -> String {
    format!(
        r#"{{"args":{{"base":{STR},"date":{{"default":"asof","type":"date","valid":true}},"quote":{STR}}},"order":["base","quote","date"],"returns":{FX_UNIT}}}"#
    )
}

/// `fx_upper/rate`: the class's rule on its key arguments.
pub fn upper_rate_sig() -> String {
    format!(
        r#"{{"args":{{"base":{UPPER},"date":{{"type":"date","valid":true}},"quote":{UPPER}}},"order":["base","quote","date"],"returns":{{"type":"number","unit":"1"}}}}"#
    )
}

/// `upper_rate(pair, date)`: dispatches over `[fx_upper]`.
pub fn dispatch_sig(upper: &str) -> String {
    format!(
        r#"{{"args":{{"date":{{"type":"date","valid":true}},"pair":{{"classes":["{upper}"],"type":"string","valid":false}}}},"order":["pair","date"],"returns":{{"type":"number","unit":"1"}}}}"#
    )
}

/// `lower_rate(x, date)`.
pub fn lower_sig() -> String {
    format!(
        r#"{{"args":{{"date":{{"type":"date","valid":true}},"x":{STR}}},"order":["x","date"],"returns":{{"type":"number","unit":"1"}}}}"#
    )
}

/// `fx_named/named`: the lookup, a label to a key of the class.
pub fn named_sig(named: &str) -> String {
    format!(r#"{{"args":{{"label":{STR}}},"order":["label"],"returns":{{"classes":["{named}"],"type":"string"}}}}"#)
}

/// The class descriptor as an author writes it (16.1); `methods` are `(name, fn hash)`.
pub fn class_json(keys: &str, order: &[&str], methods: &[(&str, &str)], host: &str, extra: &str) -> String {
    let ms: Vec<String> = methods.iter().map(|(m, h)| format!("\"{m}\":\"{h}\"")).collect();
    let os: Vec<String> = order.iter().map(|o| format!("\"{o}\"")).collect();
    format!(
        r#"{{"keys":{keys},"methods":{{{}}},{extra}"order":[{}],"steward":{{"namespace":"{host}","process":"https://{host}/classes/change"}}}}"#,
        ms.join(","),
        os.join(",")
    )
}

pub const NONE_KEYS_VERSE: &str = r#"{"book":{"key_normalisation":"none","type":"string"},"chapter":{"type":"number","unit":"1"},"corpus":{"key_normalisation":"none","type":"string"},"verse":{"type":"number","unit":"1"}}"#;
pub const NONE_KEYS_PAIR: &str =
    r#"{"base":{"key_normalisation":"none","type":"string"},"quote":{"key_normalisation":"none","type":"string"}}"#;
pub const UPPER_KEYS_PAIR: &str = r#"{"base":{"key_normalisation":"upper_ascii","type":"string"},"quote":{"key_normalisation":"upper_ascii","type":"string"}}"#;

impl World {
    pub fn new(host: &str) -> World {
        World {
            host: host.to_string(),
            verse: format!("{host}/scripture/verse"),
            pair: format!("{host}/ecb/fx_pair"),
            upper: format!("{host}/ecb/fx_upper"),
            named: format!("{host}/ecb/fx_named"),
            verse_hash: String::new(),
            pair_hash: String::new(),
            upper_hash: String::new(),
            named_hash: String::new(),
        }
    }

    /// The class descriptors as authors write them, given the method functions' hashes bound by
    /// `at` in `store`: `(class name, descriptor JSON)` for verse, fx_pair, fx_upper, fx_named.
    pub fn class_sources(&self, store: &PgStore, at: &str) -> Vec<(String, String)> {
        let h = |name: &str| -> String {
            store.resolve_name(name, at).unwrap().unwrap_or_else(|| panic!("{name} unbound"))
        };
        let (host, v, p, u, nm) = (&self.host, &self.verse, &self.pair, &self.upper, &self.named);
        vec![
            (
                v.clone(),
                class_json(
                    NONE_KEYS_VERSE,
                    &["corpus", "book", "chapter", "verse"],
                    &[("consonantal", &h(&format!("{v}/consonantal"))), ("text", &h(&format!("{v}/text")))],
                    host,
                    "",
                ),
            ),
            (
                p.clone(),
                class_json(
                    NONE_KEYS_PAIR,
                    &["base", "quote"],
                    &[("cross", &h(&format!("{p}/cross"))), ("rate", &h(&format!("{p}/rate")))],
                    host,
                    "",
                ),
            ),
            (
                u.clone(),
                class_json(UPPER_KEYS_PAIR, &["base", "quote"], &[("rate", &h(&format!("{u}/rate")))], host, ""),
            ),
            (
                nm.clone(),
                class_json(
                    NONE_KEYS_PAIR,
                    &["base", "quote"],
                    &[("named", &h(&format!("{nm}/named"))), ("rate", &h(&format!("{nm}/rate")))],
                    host,
                    r#""lookup":"named","#,
                ),
            ),
        ]
    }

    /// The flattened class descriptor of `name` as the store holds it.
    pub fn descriptor(&self, store: &PgStore, name: &str) -> (String, ClassDescriptor) {
        bl(|| {
            let hash = store.resolve_class(name, "9999-12-31T23:59:59.999999Z").unwrap().expect("a bound class");
            let bytes = store.class(&hash).unwrap().expect("a stored class");
            (hash, ClassDescriptor::from_json(&bytes).unwrap())
        })
    }

    /// The classes the store holds, by name (what a client of the mesh holds for the describe check).
    pub fn held(&self, store: &PgStore) -> std::collections::BTreeMap<String, ClassDescriptor> {
        [&self.verse, &self.pair, &self.upper, &self.named]
            .into_iter()
            .map(|n| (n.clone(), self.descriptor(store, n).1))
            .collect()
    }

    /// The function descriptors of every method of the held classes.
    pub fn functions(&self, store: &PgStore) -> Vec<FunctionDescriptor> {
        let mut out = Vec::new();
        for (_, c) in self.held(store) {
            for h in c.methods.values() {
                out.push(bl(|| FunctionDescriptor::from_json(&store.descriptor(h).unwrap()).unwrap()));
            }
        }
        out
    }
}

/// A formula defined against the store and bound from `at`.
pub fn formula_at(node: &Node, name: &str, source: &str, signature: &str, at: &str) -> String {
    bl(|| {
        let sig = datumesh_core::json::parse(signature.as_bytes()).unwrap();
        let s: &PgStore = &node.store;
        let def = define_function(name, source, &sig, &Default::default(), s, at)
            .unwrap_or_else(|e| panic!("define {name}: {e:?}"));
        publish(s, &def, at).unwrap()
    })
}

/// A class defined against the store from the descriptor an author writes, and bound from `at`.
/// Returns its hash.
pub fn class_at(node: &Node, name: &str, source: &str, at: &str) -> String {
    bl(|| {
        let s: &PgStore = &node.store;
        let definer = name.rsplit_once('/').unwrap().0;
        let def = define_class(name, source.as_bytes(), definer, s, at)
            .unwrap_or_else(|e| panic!("define class {name}: {e:?}"));
        publish_class(s, &def, at).unwrap()
    })
}

fn blob(node: &Node, text: &str) -> Scalar {
    let sha = bl(|| node.store.put_blob(text.as_bytes()).unwrap());
    Scalar::from_lexical(Type::Blob, &format!("{sha}:{}:text/plain", text.len())).unwrap()
}

/// Builds the world in `node`'s store (see the module documentation) and returns it.
pub fn install(node: &Node) -> World {
    let mut w = World::new(&node.host);
    let (v, p, u, nm) = (w.verse.clone(), w.pair.clone(), w.upper.clone(), w.named.clone());
    // The method functions, bound from BIND_TX.
    node.stored(&format!("{v}/text"), &verse_text_sig());
    node.formula(
        &format!("{v}/consonantal"),
        &format!(
            r#"remove_chars(text_of({v}/text(corpus: corpus, book: book, chapter: chapter, verse: verse, edition: "uxlc")), "{}")"#,
            strip_chars()
        ),
        &verse_consonantal_sig(),
    );
    node.stored(&format!("{p}/rate"), &fx_rate_sig());
    node.formula(
        &format!("{p}/cross"),
        &format!(r#"{p}/rate(base: "EUR", quote: quote, date: date) / {p}/rate(base: "EUR", quote: base, date: date)"#),
        &fx_cross_sig(),
    );
    node.stored(&format!("{u}/rate"), &upper_rate_sig());
    node.stored(&format!("{nm}/rate"), &fx_rate_sig());
    node.stored(&format!("{nm}/named"), &named_sig(&nm));
    // The classes, bound after them.
    let sources = bl(|| w.class_sources(&node.store, TC));
    let mut hashes = Vec::new();
    for (name, source) in &sources {
        hashes.push(class_at(node, name, source, TC));
    }
    (w.verse_hash, w.pair_hash, w.upper_hash, w.named_hash) =
        (hashes[0].clone(), hashes[1].clone(), hashes[2].clone(), hashes[3].clone());
    // The formulas over classes, bound after them.
    formula_at(node, &format!("{}/ecb/upper_rate", w.host), "pair.rate(date: date)", &dispatch_sig(&u), TD);
    formula_at(
        node,
        &format!("{}/ecb/lower_rate", w.host),
        &format!(r#"{u}/rate(base: lower(x), quote: "USD", date: date)"#),
        &lower_sig(),
        TD,
    );
    // The cells.
    node.put_value(
        &format!("{v}/text"),
        &args(&[
            ("book", s("Gen")),
            ("chapter", n("1")),
            ("corpus", s("tanakh")),
            ("edition", s("uxlc")),
            ("verse", n("1")),
        ]),
        blob(node, GEN_1_1),
        TV,
    );
    let rate = |name: &str, base: &str, quote: &str, value: &str| {
        node.put_value(name, &args(&[("base", s(base)), ("date", d(DAY)), ("quote", s(quote))]), n(value), TV);
    };
    rate(&format!("{p}/rate"), "EUR", "USD", "1.2");
    rate(&format!("{p}/rate"), "EUR", "GBP", "0.8");
    rate(&format!("{u}/rate"), "EUR", "USD", "1.2");
    rate(&format!("{nm}/rate"), "GBP", "USD", "1.25");
    node.put_value(
        &format!("{nm}/named"),
        &args(&[("label", s("cable"))]),
        s(&format!(r#"{nm}(base:"GBP",quote:"USD")"#)),
        TV,
    );
    w
}
