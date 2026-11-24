# Changelog

## 0.5.4

The next patch's http changes (CONTRACT-next-patch items 15, 16 and 33); requires datumesh-formula 0.6.2 (`read_names`). Item 17 is a test change (`batch::thousand_cells` made robust, its claim stated as the user decided it); item 33 (`/x/calc` result types, added by the user) is a code change. Each change, and what it costs:

- **The function page shows each test's inputs (item 15).** A case with `inputs` reads them beside its call: "with <slice call> given as empty" for an empty members map, "with N members given" for a members map of N, "with <call> given" for any other input. Two cases with the same call read as two lines. `?format=json` carries each case's `inputs` as the test gave them, and omits the key for a case without. Cost: the page and the JSON cases of tests with inputs are longer.
- **`/x/calc` returns text, date, instant and boolean results (item 33).** The body is defined against each result type in turn (a number first, as before), and the first the body satisfies gives `type`; `value` is its lexical form, `kind` is `calculation`. Before, any non-number result was refused `E_FORMULA_RETURN_TYPE`. Numbers are unchanged. Cost: a body that was refused for its result type now answers 200.
- **`batch::thousand_cells` states the batch claim the user decided (item 17).** A batch costs one round trip and one name lookup for any number of cells, and on loopback 1,000 cells take less than 50 single requests (the bound was ten). Test change only; no code change.
- **`/x/calc` builds its callee environment in one pass (item 16).** The names the body reads come from datumesh-formula 0.6.2's `read_names`; each name's descriptor as of the asof is looked up once, in place of the loop that matched the "has no binding" message. Behaviour is unchanged (`tests/h054_calc_unbound.rs`). Cost: none.

## 0.5.3

The protocol round's http changes (CONTRACT-protocol-part1, datumesh-http 0.5.3: H2 and H3). Additive: no stored, signed or hashed value changes. Each change, and what it costs:

- **CSV and Arrow slices obey `limit` (H2, D6).** At most `limit` rows (never above `DATUMESH_SLICE_MAX`); when more members follow and the request named a `limit`, the response carries `Link: <same URL with after=<cursor>>; rel="next"`, the cursor being the JSON `next`'s. Without `limit`, the whole slice up to `DATUMESH_SLICE_MAX`, with no Link, as in 0.5.1. The whole-slice body cache keys on `limit=` and `after=` (as before) and keeps the page's Link with its body. Cost: a CSV or Arrow request with `limit=` now returns that many rows, not the whole slice up to the bound.
- **Instants: 0 to 6 fractional digits and a bare date (H3, D7, D7.1).** Wherever a request takes an as-of (the query `asof` of the read endpoints, and `asof` in the `/x/calc` and `/whatif` bodies), `YYYY-MM-DDTHH:MM:SS[.f{1,6}]Z` and `YYYY-MM-DD` (the end of that day, UTC) are accepted, canonicalised to six digits before use; the answer's `asof` is the canonical instant. `/versions` `from=` and `to=` take the fractional forms. An offset other than `Z` is still `E_CANON_INSTANT`. Cost: a bare date, which was refused, now reads as the end of its day.
- **/x/calc reads positional calls in the callee's declared order.** The body of `POST /x/calc` is defined against the store as of its asof, as the publish path and the local resolver define one: `f(a, b)` binds by the callee's `signature.order`. A callee without an order still takes labels only (SPEC 15.5), refused as `E_FORMULA_POSITIONAL`. Cost: a body that positionally reads a function with an order now computes, where it was refused.

## 0.5.2

The held sentence on production's shape (CONTRACT-0.5.2, from the rehearsal of the restored copy). Two changes, each with its cost:

- **A class's cells count for the measure order.** A class entry's cells are the stored cells of its methods' functions (the sum of their `cell_count`, read once per build), so production's verse class (many cells) puts "texts" before "provenance": "scripture (texts, provenance)". Cost: a measure held by a class with cells can move up the order; the held text changes only where a class has cells.
- **A declared source is "computed from".** Any faceted entry that is not a relay and declares a `source_name` reads "computed from <name>", whether its code is stored or a formula (production's rainfall is a stored function with `source_name` "CHIRPS": "monthly rainfall computed from CHIRPS"). Own data without a `source_name` names none; a relay still reads "from <name>". This supersedes the 0.7 R8 rule that own data names no source, by the user's decision of 2026-11-23. Cost: an entry of the publisher's own data that declares a `source_name` is now named in `held`.

## 0.5.1

The round 0.7 slices and catalogue (CONTRACT-0.5.1 H1 to H5): requires datumesh-store 0.6.4 (unreleased: a stored sort key per cell, `slice_page`, class bindings by hash), datumesh-core 0.3.14 and datumesh-formula 0.6.0. Each change, and what it costs:

- **Paged slices (H1).** `/slice` reads one page at a time by key (`slice_page`), at `limit=` (default `DATUMESH_SLICE_PAGE`, at most 10,000); `next` is still the last member's key, now turned into the store's sort key. The in-memory member cache is gone: a latest first page pins its own instant in `next`, and nothing is reused between requests. CSV and Arrow take at most `DATUMESH_SLICE_MAX` members, one read. A slice whose fixed or range arguments do not form a sort-key prefix costs the members it names, not the function. `/x/interval` and the series period pages read `slice_page` as well (their cap is `DATUMESH_SLICE_MAX`). Cost: a slice is no longer cut at one total of `DATUMESH_SLICE_MAX` members across pages; pages walk on by key.
- **The object index pages by key (H1, H2).** The index of a class walks its extent in pages of 1,000 by sort key (was 10,000 by cell id). When a class is cut at `DATUMESH_INDEX_CLASS_MAX`, the objects kept are the first in sort-key order. Cost: a class cut at its cap keeps a different set of objects than before.
- **Paged catalogue reads (H2).** The catalogue job and the catalogue thread list subjects from the `exists` extent in pages of 1,000 and read `label`, `sources`, `source(rank)` and `from_label` by key per page; no read is larger than a page. `DATUMESH_CATALOG_MAX` (default 100,000) caps the subjects listed; a cut is logged and counted in `datumesh_catalog_truncated_total`. `STORED_CELLS_READ` is gone. Cost: a catalogue with more subjects than the cap lists the cap's worth.
- **Find drops one-letter words (H3).** A query's alphabetic one-letter words are dropped before matching (digits and other single characters stay); a query of only such words is the empty query. Cost: a query of single letters returns the listing of the empty query, not a text match on them.
- **The held sentence (H4).** A computed entry (a function whose code is not stored) with a `source_name` is named "computed from <name>"; a relay still reads "from <name>". A measure's order is by its rank-one entries' cells, descending, then by name. Own data names no source. Cost: `held` text changes for measures with cells; production's rainfall entry reads "monthly rainfall computed from CHIRPS".
- **Whole-slice bodies at a past asof are cached (H1, bounded).** A CSV or Arrow answer of a slice at an explicit `asof` strictly before now is kept encoded, by form, slice, asof, `limit=` and `after=`, in an LRU of 64 MB (a larger body is not kept); such an answer is immutable. Latest answers and JSON pages are not cached. A warm 100,000-member Arrow stream takes about 70 ms (cold 8 s, release build). Cost: up to 64 MB of memory for the cache.
- **One parent walk per class describe (H5).** `describe` of a class walks its parent chain once and names each defining class once (class bindings by hash, store 0.6.4). Cost: none (the same answer, fewer reads).
- **Publish prefetch in pages.** The publish path's version and receipt reads are a page of ids at a time. Cost: none.

## 0.5.0

The classes round (CONTRACT-classes-round K1, K3, K5, K7 to K13): requires datumesh-core 0.3.13, datumesh-store 0.6.0 and datumesh-formula 0.6.0 (unreleased, on the classes-round branches). **BREAKING: K5, an inherited method reads the defining class's cells.** The other changes are additive. Each change, and what it costs:

- **BREAKING (K5): an inherited method reads the cells of the class that defines it.** A subclass object's read of an inherited method (in a request, a body, a dispatch and through `this`) is the cell of `D/m`, D the nearest ancestor whose own method gave `m` that function hash, named as of the request's asof (`resolved` says `method` -> `D/m`). An override stays `S/m`. Cost: a cell that was written under `S/m` for an inherited method is no longer read; the subclass's object reads the parent's cells, as the spec says. Describe of a class names `D/m` for an inherited method. A library before this release resolves the old way. Production's scripture classes have no subclasses, so no production cell changes.
- **Test inputs override a slice (K1).** A test's `inputs` entry `"<slice call>": {"add": {...}, "remove": [...]}` changes what a slice read sees in that test run (the store's members, minus `remove`, plus `add`). Cost: none for existing tests (the entry is new); the case hash covers it through `inputs`.
- **Boolean test expectations and inputs are JSON booleans (K3).** `expect` and `inputs` of type `boolean` take `true` or `false`; the string `"true"` is refused, as a cell line refuses it. Cost: a test file that wrote a boolean as a string is refused; the other types keep their lexical strings.
- **Inline class methods (K7).** A class line's `methods` may hold `{"stored": <signature>}` or `{"formula": <source>, "signature": <signature>}`; the methods are defined, and the class and their names bound in one transaction (`bind_class`). A formula method reads `this` as its class; mutual reads are refused. Variable-key receivers are read in bodies. Cost: the class line's validation runs the formula's checks of its methods; a class with only function hashes is unchanged.
- **Folded and withdrawn labels (K8).** A request's label is folded (case, spaces, NFC) where the lookup declares `label_fold`; a stored label must be folded (`E_KEY_NORMALISATION` at publish otherwise); `""` as a lookup's value withdraws a label (`E_NAME_UNBOUND`). `describe` answers `N[label]`, and `/cell` of `N[label]` with no method answers that object's describe. Cost: a stored label that is not folded is now refused at publish (no existing data does this).
- **Raw brackets at `/cell` (K8 fix).** `/cell/N[label]` with raw, unencoded brackets and no method answers the object's describe as the percent-encoded form does (the raw `[`/`]` was refused as `E_CANON_NAME` before the label form was read). Cost: none; a malformed class part is still `E_CANON_NAME`.
- **`extends` in the catalogue (K9).** A function's catalogue entry may name the class it extends; publish checks the keys and their types, and refuses it otherwise. `describe` of an object lists `extensions` (`fn`, `fn_hash`, `publisher`), only when there are some. Cost: a bundle's catalogue refuses `extends` for now (publish accepts it); the store's `extensions_of` is read on each object describe.
- **Chained requests (K10).** `N(k).m(a).n(b)`: each method after the first is called on the key the previous one returns, read as of the request's asof; `resolved` gains one `chain` resolution per link. A key outside the declared classes is `E_FORMULA_DISPATCH_KEY` (`not_covered`, with that fault). Cost: a new `Rejection::Fault` variant in `call` (the public enum), and the `Bindings` trait gains provided methods (`class_name_of`, `class_by_hash`, `chain`) that default to none.
- **Unquoted digit runs in string positions (K11).** A CIK or an accession number may be written unquoted in a string position of a request. Cost: none for other positions (they keep their codes).
- **Literal defaults (K13).** An argument may declare `"default"` as a literal; a request, a body and publish take it. Cost: only new functions have it (it is hashed).
- **The mesh's formula runtime names classes (K5, internal).** The runtime is built with `FormulaRuntime::new` on the store, which forwards the class names (formula c7fea2f); the mesh's own `MeshCallees` is removed. Cost: none outside the mesh.
- **Catalogue phase 1 (classes round C1 to C10, CONTRACT-classes-round Part C).** Requires datumesh-store 0.5.3 (facets and migration 0012, read through `catalog_entry_ext` and `catalog_entries_with_facets`) and datumesh-formula 0.6.0 (its `FormulaRuntime::new` names the classes). Every change is additive. Each one, and what it costs:
  - **The reserved `catalog` namespace (C1).** `<host>/catalog` and below is reserved like `series` and `fn`: `publish-stored`, `import-signed` and `POST /bundles` refuse a name there with `E_NAME_RESERVED`, the file or bundle whole. Only the catalogue job's own run writes there (the flag is for that one run). Cost: a name under `<host>/catalog/…` on this mesh is refused from this release.
  - **Facets on catalogue entries (C2).** The `catalog` line and bundle 1's `catalog` member may carry `facets` (five keys, bounded lists of names). `describe` (functions and classes) and `find` carry `facets` when the entry has them. A line without `facets` removes them, since the line is the publisher's whole statement of the entry. Signed with the bundle, outside every hash. Cost: a republished entry without facets loses them; the store's `catalog_entry_ext`, `upsert_catalog_ext` and `catalog_entries_with_facets` are used (store 0.5.3+).
  - **The catalogue job (C3 to C5), `datumesh-mesh catalog-build`.** Writes the classes `<host>/catalog/subject` (keys domain, thing, measure, region; `label`, `source(rank)`, `sources`, `exists`, `from_label`) and `<host>/catalog/facet` (`label`, `subjects`, `exists`) with their methods inline, their cells and the rule `<host>/catalog/rule` (a blob, also the page `/catalog/rule`), signed with the mesh's key under `DATUMESH_LICENCE`. A rerun over an unchanged catalogue writes no cell (`cells` 0, the store unchanged). A subject that disappears gets `exists()` false and its labels withdrawn (`""`); a facet value that disappears gets `exists()` false. Cost: one run reads the faceted entries and the cells of the subjects and facets (the store's `stored_receipts_of`), and writes only what changed; the job needs the store and the key, not a request. Level, binding time and name: the rank rule in the README.
  - **The describe root (C7).** `describe/<host>/catalog` answers `{kind: "catalog", host, domains, rule}` in one call: domains by subject count, each with its catalogue-name count and up to five subjects with their label and rank-one source. Cost: a new `describe` kind, read from the last catalogue build (`503` with `Retry-After` before the first build).
  - **The `held` text (C8).** `find?summary=1` gains `held`, at most 60 words: domains by size, each with up to three measures and a rank-one source. Absent with no facets. Cost: one string on the find summary, built with the find thread from the catalogue's last build.
  - **The pages (C9).** `/catalog`, `/catalog/<domain>`, `/catalog/<domain>/subject?key=…` and `/catalog/rule` (HTML, at most three steps deep), built on a thread (`DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS`, `DATUMESH_CATALOG_REFRESH_SECS`, default 300 s) and kept in memory; a request reads the last build (`503` before it), and no request does work that grows with the data. Metric `datumesh_catalog_builds_total{by}`. Cost: the pages' HTML is held in memory and rebuilt every refresh interval even when nothing changed.
  - **The addresses (C10).** No name, page path or link contains a facet value or a rank as a segment; a subject is addressed by its key encoding in the query; sources are linked by their full names.
  - **`catalog-build` in the README** and the knobs in its table.
- **Internal: the formula runtime (K5 follow-up).** `MeshCallees` is removed; the runtime is `FormulaRuntime::new(store)`. Cost: none.
- **`check_stewards.py` (K12, deploy).** Before a production publish of a class, `deploy/prod/feeds/check_stewards.py` fetches each class line's `steward.process` and requires 200 `text/html`. The SEC feeds spell `https://datumesh.com/classes/change` for production; the rehearsal addresses are noted as not real pages. Cost: an operator step before each production class publish.

## 0.4.10 (unreleased)

Public paths do no work that grows with the data (CONTRACT-round-0.7-P HP1 to HP8, from the sweep SWEEP-0.4.10 rows A1 to A20). Requires datumesh-store 0.5.3 (additive: counters, bounded reads, migration 0012). The public API gains the `DATUMESH_INDEX_SECS`, `DATUMESH_INDEX_CLASS_MAX` and `DATUMESH_INDEX_MAX` configuration fields; the rest is unchanged.

- **`describe` of a class reads no cell (HP1).** A class answer reads the descriptor only and hears of the class for the next index tick. An object named by its key encoding is answered without the index. A hex id the last build does not hold is `404` with the reason "not in the object index yet" (`503` with `Retry-After` before the first build). A cell id names no object (`404`).
- **The object index is built from the declared list (HP2).** The build reads, for each heard-of class, the stored cells of its `extent.keys` function (or of its first method in name order, when it declares none) in pages of 10,000 by cell id; computed cells are not read. A class holds at most `DATUMESH_INDEX_CLASS_MAX` objects (default 50,000) and the index at most `DATUMESH_INDEX_MAX` (default 200,000). A cut class keeps its first objects in cell-id order, is logged, and is counted in `datumesh_object_index_truncated_total`; its other objects are reached by key encoding. The per-class cap applies at build time only: a computed cell or a stored version added between builds is put into the index subject to the total cap alone.
- **No rebuild per write (HP3).** A stored version or a computed cell is added to the last build at once. A full build runs at start and then every `DATUMESH_INDEX_SECS` (default 300). Between full builds the index thread, on each tick (500 ms), indexes each heard-of class the last build does not hold (that class only, with the same caps, the total cap counting the objects already in the build) and merges it into the last build; a class with no objects is recorded as held too, so it is not read again. Nothing is indexed on a request.
- **Short ids (HP4).** A 12-hex id is a range read of the sorted index, the first two matches.
- **The function page and `describe` of a function (HP5).** Counts come from the store's test counter. Counterexamples: the stored failing and erroring tests (at most 100 listed) and the foreign `false` test cells (at most 100 read); the JSON gains `counterexamples_more`, and the HTML says "100 or more". Dependants are at most 100, with `lineage.dependants_more` and "and more" in the HTML. A test cell is read by its key.
- **`find` does bounded work (HP6).** A query is cut to its first 8 words and its spans to 4 words. Generic words come from one read of the catalogue, on the find thread's cycle. The summary's cell counts are counters. The value counts (`refresh_value_counts`, which read the cells' arguments) are no longer part of the first build: they run first `DATUMESH_VALUE_COUNTS_SECS` after start, then every `DATUMESH_VALUE_COUNTS_SECS`, and their result persists in the store. A dictionary value on a request is matched by `has_key_value`; the publish path's value lookup (`value_row`) still uses the store's text search, as it runs on a write, not a request.
- **Background threads are bounded (HP7).** The takeover lists and the series overview read names with `catalog_entries_all`, dictionary values in insertion order with `dictionary_values_ordered`, and fold the overview's members in pages.
- **Other public reads are bounded (HP8).** A series period page reads at most `DATUMESH_SLICE_MAX + 1` members and shows at most `DATUMESH_SLICE_MAX`. A formula's stored slice read (`/x/calc`, `/whatif`, a computed `/cell`, a test run) reads at most the remaining fuel plus one; more is the evaluation budget.

## Named test cells (spec-0.6.2 P3, carried on this branch; 0.5.0)

Named test cells (spec-0.6.2 P3, CONTRACT-spec-0.6.2 addendum). Requires datumesh-core 0.3.11 and datumesh-formula 0.5.2 (spec-0.6.2 round): through the libraries, the public items they add (licences, the shares and pure units, `last_by`) reach the mesh, and this crate's public API does not change.

- **The named test cell (spec 7.3, 16.8).** With `DATUMESH_TEST_CELLS` on, a name `<host>/test_case` that is unbound binds the named form `test_case(case, function, function_hash) -> boolean` (`function` the test's function name, the name stored with the row). A name already bound to the two-argument form keeps it (no rebind). `TestCases` computes the cells of whichever form is bound; in the named form only the tests of the function `function` names are its tests.
- **Counterexamples (spec 7.3).** `describe` and the function page count a foreign `false` two-argument cell for every function of the hash, and a foreign `false` named cell (of the stored kind) only for the function it names.
- **Tests.** The tests that called the two-argument `test_case` on a fresh mesh call the named form now.
## 0.4.9 (unreleased)

Public paths do no work that grows with the data (the sweep of 0.4.8: CONTRACT-0.4.9 H1 to H8, decision D43), and the dictionary rule. Requires datumesh-store 0.5.2 (additive, migration 0011: `function_tests.fn_name`, the table `open_keys`; every existing receipt verifies, no row is rewritten but the new nullable column). Two public items change: `BundleError` and `DeclareError` gain a `Reserved` variant (both are `#[non_exhaustive]`). The rest of the public API is unchanged.

- **The function page and `describe` never run tests (H1).** `GET /fn/<name>` and `GET /describe/<function>` read the stored results only: the counts, the worst result and the latest run (S2), the first 100 cases (S3), and the counterexamples (the stored `fail` and `error` tests of the name, plus the `false` test cells of other hosts). No test is run, no coverage is computed and nothing is evaluated on a request. The function page's `tests` gains `last_run` as `{result, at, pass, fail, error, not_run}` (`result` the worst stored result: `error`, `fail`, `not run` while any test has not run, else `pass`; `at` the latest stored run, `null` when none ran); `tests.derived` is removed (per-case coverage needs a run). `branches` and `covered` are the stored counts. The HTML says "last run <at>" and the worst result; "Showing the first 100 of N tests" is kept. `describe`'s answer keeps its shape.
- **Tests are named when they are stored (H1, S1).** `publish-stored`, `import-signed`, `POST /bundles` and the rebinds name each test by the function its call names. A thread of the mesh names the tests written before 0.4.9 once, in batches of 1,000. Until a function's old tests are named its page counts them by call and says that the counts may include tests of other functions of the same signature.
- **Relays and the takeover page (H2).** `relay_of` reads the first 8 receipts of a function (S4), keeps a found source for the life of the process and a missing one for 300 s. `GET /takeover/<domain>` reads the lists of a thread that builds them from the bound names (with the series' first delay and refresh interval); before its first build it answers `503` with `Retry-After` (1 to 300 s) and `no-store`. The policy and the contact are unchanged.
- **The object index is built in the background (H3).** The index of `GET /describe/<object id>` (64 or 12 hex digits) is built on a thread (at start after `DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS`, then when due: every 300 s, or when the mesh learns of a class or a version; an object this mesh writes or computes is indexed at once). A request reads the last build; before it, `503` with `Retry-After` and `no-store`. Class names, key encodings and function names do not wait for it.
- **Find (H4).** `GET /find?summary=1` and the empty query are read from a build of a thread (after `DATUMESH_FIND_FIRST_BUILD_DELAY_SECS`, then every 300 s), `503` before the first build. The summary counts the cells of names that share a hash by one indexed count (S6), not by a full slice. `entry_by_name` and `has_value` are exact lookups (S8 `catalog_entry`, S7 `has_key_value`), not `text_find`. `reindex` refreshes the value counts at most once per `DATUMESH_VALUE_COUNTS_SECS` (3600) and once at start. `GET /find?q=` is unchanged (it is reported, not fixed).
- **Bounded reads on the public paths (H5).** `/slice` reads at most `DATUMESH_SLICE_MAX + 1` members (S5), so a slice past the cap is cut to its first `slice_max` members as before, with no `next` past them. `/x/interval` reads the same bound, and a year series may now span 50 years (`E_RANGE_TOO_LARGE` beyond, for JSON, CSV and Arrow alike; a month series keeps its 10). `/versions/<cell>` reads 1,001 versions (S5). The dictionary order and the entry text of the series pages are part of the background overview, not computed on a request.
- **Publish does work in proportion to what it writes (H6).** A publish reads a function's tests once per hash, not once per row; it runs the held tests of a stored function only where they read a cell the file writes, and a rebind reruns all of a name's tests. The old tests keep their `last_run_at` when a file adds cells and tests to a function that has them.
- **Dictionary values: the rule (H7).** Only a `string` key takes dictionary values from cells. A publish whose new values would take a key past `DATUMESH_DICTIONARY_MAX` (default 1000; the process variable is read by the publish path, and `Config` reads it too) adds none of them and marks the key open (S7); an open key takes no automatic values, and nothing is deleted. A catalogue entry's `open_keys` opens its keys from the first cell. `find` fills an open key only from an exact cell value. `GET /metrics` gains `datumesh_dictionary_open_keys_total`. The README has a section "Dictionary values" with the rule.
- **Reserved paths (H8, D43).** Under the mesh's own host, a name whose first segment after the host is `series`, `fn`, `takeover` or `x` is refused with `E_NAME_RESERVED` at `publish-stored`, `import-signed`, `POST /bundles`, `declare` and bind (`422` for `POST /bundles`, the file or bundle refused whole, nothing written). A `<host>/key/…` name is accepted only as a key-derived name from the registered key it derives from; the mesh's own key and every other `key/…` name are refused. Names under other hosts are not reserved. The README has a section "Reserved paths".
- **Configuration.** New: `DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS`, `DATUMESH_FIND_FIRST_BUILD_DELAY_SECS`, `DATUMESH_VALUE_COUNTS_SECS`, `DATUMESH_DICTIONARY_MAX` (README rows). `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS` and `DATUMESH_SERIES_REFRESH_SECS` also govern the takeover lists.

## 0.4.8 (unreleased)

Build 2 Part 6 follow-up and the key-derived names (QUESTIONS-build2 D39 to D41, G5). Additive: the new configuration (below) and `RateLimits::keys_per_ip` are new public items; the public API otherwise does not change. Requires the same crates as 0.4.7.

- **Key-derived hosted names (D39, `DATUMESH_KEY_NAMES=1`, mesh-test only).** `GET /x/keys/challenge` issues a challenge, `POST /x/keys` registers an Ed25519 public key by its proof of possession (the signature of the challenge), and the key is added to the host's key document, valid from the registration, without rotating or closing any other key. Its namespace is `<host>/key/k<first 24 hex digits of SHA-256 of the raw public key>`. The registration is kept in the key document in the store (no store change). `POST /bundles` signed by a registered key is accepted only when every name is under that key's namespace (`E_NOT_OWNED`, 422 otherwise); the mesh's own key and the allowed domains are unchanged. Off by default, with `endpoints.x_keys` in the mesh document only when on; its own budget `DATUMESH_RATE_KEYS_PER_IP` (5 a minute, every POST counts).
- **The takeover page states the policy (D40).** `GET /takeover/<domain>` says that no permission is needed (the source proves control with a key document on its own domain), that its relay name then stops taking new stored cells, is rebound to read the source's cells, and keeps its history as the mesh's signed statements. The JSON form gains `policy` (those sentences) and `contact` (null unless `DATUMESH_CONTACT` is set; then the page and the JSON name the address).
- **The series overview is built in the background (G5).** The sitemap and the series indexes are read from an overview that a thread of the mesh builds at start (or after `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS`) and then every `DATUMESH_SERIES_REFRESH_SECS` (300). A request never builds it: before the first build the sitemap (and its parts) and the indexes answer `503` with `Retry-After` and `no-store`. Month and year pages and CSV do not wait for it (their previous and next links are missing until the first build). `GET /metrics` gains `datumesh_series_overview_builds_total{by="background"}` and `{by="request"}` (always 0).
- **The function page is bounded.** `GET /fn/<name>` lists at most the first 100 tests (HTML "Each test" and the JSON `tests.cases`), with a note and the total (`tests.cases_total`).
- **Production stays closed to other namespaces (D41).** `DATUMESH_PUBLISH_DOMAINS` set to the empty string behaves as unset; a regression test holds it.

## 0.4.7 (unreleased)

Fix of a panic in 0.4.6. Requires the same crates as 0.4.6; the public API does not change.

- **Panic on the call form of a slice address, on PostgreSQL.** On mesh-test (PostgreSQL store), `GET /slice/<fn>(date: *)` with `Accept: text/html` made the request's task panic (`postgres-0.19.14/src/connection.rs:66`, "Cannot start a runtime from within a runtime") and the proxy answered 502. Cause: 0.4.6's call-form redirect resolved the call with `call::resolve`, which reads the store (it looks the name up), directly on the async thread, from the `Accept` middleware (`negotiation_free`) and again from the slice handler; the synchronous PostgreSQL client may not run on a runtime thread. The query form (`?date=`) reads no store, so it never panicked, and the SQLite store used by the series tests does not mind, which is why the tests did not see it. Now the handler decides a browser's call form on the pool (`Pool::run`), as every other store read; the middleware no longer reads the store for it and leaves the `Accept` check to the handler, which answers the same 406 when the call is no redirect. Behaviour is otherwise identical: `303` `no-store` to the page or index, JSON for a JSON client, 406 for a browser on a call that is no listed series'.
- **Audit of the 0.4.5 and 0.4.6 async-path code.** The series pages and sitemap, `/x/interval`, `/x/calc`, the takeover page, the function page, `/find` (the by-name summary count), `/slice` and the middleware were read for store use on an async thread: all of them already run on the pool; the call-form redirect was the only direct use. New tests (`tests/pg_async_paths.rs`) run every route that reads the store on a PostgreSQL store (the series pages, CSV, index and sitemap, `/x/interval` in JSON, CSV and Arrow, `/x/calc`, `/fn`, `/takeover`, `/find?summary=1`) and the call-form redirect (listed, keyed and unkeyed, percent-encoded or not, two browser `Accept` values), checking that the mesh keeps serving afterwards and that a JSON client on the call form still gets JSON.

## 0.4.6 (unreleased)

Fixes from mesh-test and datumesh.com, and the interval call for agents (QUESTIONS-build2 D23, D32). Requires the same crates as 0.4.4 (datumesh-store 0.5.1 is unchanged).

- **Summary counts by name (D23).** `GET /find?summary=1` took each function's `cells` from the store's catalogue summary, which counts by function hash; stored functions of one signature share a hash, so `bank_rate` (13,116 cells) and `estr` (1,829) both showed 14,945. Where a hash is bound to more than one listed function the count is now made by name (the stored cells of `ns/fn`, cached five minutes); a function with its own hash keeps the store's count. The series pages, the index, the function page and `describe` were checked: they already read by name (a slice names `ns` and `fn`) and are covered by tests with two functions of one descriptor.
- **The call form of a slice address, for browsers (D32).** `/slice/<function>(<args>)`, percent-encoded or not, with `Accept` led by `text/html`, is now sent `303` (`no-store`) to the page or the index of a listed series like the query form; before, only `?date=` was read and the `Accept` check answered 406. The call is read by the crate's own call parser (`call::resolve`), so aliases and unquoted values work as at `/slice`.
- **`GET /x/interval/<function>` (D32: an extension).** `?<key>=<value|*>...&from=YYYY-MM-DD&to=YYYY-MM-DD[&format=csv|arrow]` answers the stored cells of a function listed in `DATUMESH_SERIES_FILE` with a date in `[from, to]` (inclusive), keys bound or open (an omitted key is open; a key the series file fixes may only be given its value), each with its receipt. JSON by default: `{function, from, to, asof, count, members: [{keys, value, type, tx, asof, receipt_hash, receipt}], truncated?}` (at most `DATUMESH_SLICE_MAX` members). CSV is long form (D33): one row a cell, the open keys as columns (the series file's fixed arguments are omitted), then `date`, `value` and `receipt` (the receipt hash), by date then key in dictionary order; the page CSV stays exactly the table, with no receipt column; Arrow is the slice endpoint's stream. Read from the store only, as the pages are. A range of more than ten years for a month series is `400` `E_RANGE_TOO_LARGE`; a missing or malformed `from` or `to`, `from` after `to`, an unknown argument or an unknown `format` is `400` `E_PARSE_URI`; an unlisted function is `404`. `Cache-Control` as the pages: `s-maxage=86400` for a range wholly before the current period, `600` for one that touches it; `ETag` and `304`. It takes the series bucket (`DATUMESH_RATE_SERIES_PER_IP`) after the per-IP limit. The mesh document gains `endpoints.x_interval` (`https://<host>/x/interval/{function}`), OpenAPI gains the path, and the agents block of a page and of an index gives the call for the page's interval in JSON, CSV and Arrow and says it takes any interval; the `/slice` lines stay.

## 0.4.5 (unreleased)

For CONTRACT-build2-part6-pages (G1 to G6; QUESTIONS-build2 D25, D26, D31): month pages for relayed series, the year pages of monthly ones, their index and sitemap. Additive: `RateLimits` gains `series_per_ip` (a public field), and the public API otherwise does not change. Requires the same crates as 0.4.4.

- **The series file (G2).** `DATUMESH_SERIES_FILE` names a JSON array, one entry per series: `function`, `granularity` (`month` for a daily series, `year` for a monthly or event one), `date_arg`, `column_arg` (optional: the keyed argument whose dictionary values become columns), `fixed` (optional: arguments held fixed, as strings), `unit`, `kind` (`relayed` or `computed`), and `source` (optional: the name a computed series' page gives, else read from the function's description after ` from `). Without it there are no pages, and a function it does not list has none.
- **The pages (G1, G3).** `/series/<function>/<YYYY-MM>` and `<YYYY>`, each with a `.csv` twin, and `/series/<function>/` as the index. A page is one `<table>` with one header row (`date`, then one column per dictionary value of the column argument, in the dictionary's order, each headed `value (unit)`; an unkeyed series has one `value (unit)` column), ISO dates, the stored decimal strings, empty cells where there is none. It carries the source line (relayed: `datumesh.com`'s relay of the source, with the link to `/takeover/<domain>`; computed: "Computed by <host> from <source>; not <owner>'s own values."), the licence of its cells, its as-of (the latest `tx` among its cells), a block for agents (JSON, CSV and Arrow slice addresses and the MCP `get` request), the verify link, previous and next links to the nearest periods with data, the index and the function page, and one schema.org `Dataset` JSON-LD script with no `<` in it.
- **Redirect (G1).** A browser (`Accept` led by `text/html`) on `/slice/<call>` of a listed series gets `303` to the month or year page of a single date, or to the index for `date=*`, with `Cache-Control: no-store`. A JSON client gets JSON as before. `?format=csv` (RFC 4180, `text/csv`, the same cache as JSON) and `?format=arrow` are addresses of their own; an Arrow answer chosen by `Accept` is `no-store`.
- **Rendering from stored cells (G5).** Pages, CSV, index and sitemap read the store only: the binding as of now, the descriptor, slice reads by a date range (`slice_values`), the receipts of the members (`receipts_many`, for licences) and the catalogue (`catalog_summary`, `text_find`). No evaluation, no fetch from another mesh; a page with no stored cell, a malformed period or an unlisted function is `404` (cacheable 600 s). The index and the sitemap, and the neighbours of a page, come from an overview of the series' cells kept for five minutes.
- **Caching (G5).** `Cache-Control: public, max-age=300, s-maxage=86400` for a past period, `public, max-age=300, s-maxage=600` for the current one, the index, the sitemap and a 404. `ETag` is a hash of the page's receipt hashes, its neighbours and its form; `304` on `If-None-Match`.
- **Sitemap (G4).** `/series/sitemap.xml` lists every index and page with `lastmod` the as-of; past 50,000 URLs it is a sitemap index of `/series/sitemap-<n>.xml` files of 50,000.
- **Rate limit (G5).** `/series/*` takes one token of the per-IP limit and one of `DATUMESH_RATE_SERIES_PER_IP` (default 30 a minute, its own bucket).
- **Documented.** OpenAPI paths for `/series/{page}` and `/series/sitemap.xml`; README rows for the routes and the two variables.

Build 2 Part 3, the `calc` addition (CONTRACT-build2-part3-calc C1, C3; QUESTIONS-build2 D27, D30). Additive: `Config` gains `rate.calc_per_ip`, the mesh document gains `endpoints.x_calc`, and the OpenAPI document gains `/x/calc`. Requires the same crates as 0.4.4.

- **`POST /x/calc` (D27: an extension, not in the spec).** The body is `{"formula": "<body>", "asof"?: <instant>}`. The formula is evaluated in a scope, as `whatif` evaluates: nothing is written, no receipt is signed, no name is bound. The answer is `kind: "calculation"` with `value`, `type` (and `unit`), `formula` as given, `canonical` (the printed canonical AST), `asof`, and `lineage`: each cell read, with its `call`, the `receipt` hash that answers it (`null` for a computed cell this mesh has not stored), its `publisher` (the host of its namespace) and its `trust` (as the envelope labels it). A computed cell is evaluated in the scope and never stored. Calls, slice aggregates and constant formulas are answered.
- **Limits.** A formula of more than 4096 bytes is `413` `E_TOO_LARGE`. The mesh's evaluation budget (`DATUMESH_BUDGET_MS`) gives `E_EVAL_BUDGET` at once, with no pending job. `DATUMESH_RATE_CALC_PER_IP` (default 30 a minute) is a bucket of its own, taken after the per-IP limit.
- **Errors.** A formula's codes are carried as written (`E_FORMULA_SYNTAX`, `E_FORMULA_TYPE`, ...): `bad_request` (400) for a formula that does not define, `not_covered` (404) with `fault` for an evaluation fault (`E_FORMULA_DIV_ZERO`, `E_FORMULA_ABSENT`, `E_FORMULA_EMPTY`, ...), and `error` with `cycle` (the 5.5 envelope, `E_EVAL_UNAVAILABLE`) for a cycle of computed cells. The core's error vocabulary has none of the formula's or the budget's codes; they are written as strings in this crate's envelopes.
- **Access.** A licence or payment rule on a function the calculation reads answers as it does for `whatif`: `unlicensed`, or `payment_required` with its quote.
- **Not done.** Class dispatch in a body, a range over a stored slice, and the versions of a computed function are refused (`E_EVAL_UNAVAILABLE`); `calc` on a foreign (relayed) function is not read from its source.

- **Feed jobs (deploy only, not in the crate).** `series.py` writes the series file of the month pages from `names.toml` (`run_feed.sh series`); `http_get` retries transient network failures (4 times, after 5, 15, 45 and 120 s), while 404 and 429/503 keep their own paths.

## 0.4.4 (2026-11-20)

Fixes the tests of stored functions that share a signature (QUESTIONS-build2 D23) and serves the takeover page (D24). No API change: `testrun` is private, and no public item changes. Requires the same crates as 0.4.3.

- **The bug (found on mesh-test with 0.4.3).** A stored function's descriptor is only its signature, so `relay/ecb.europa.eu/estr(date)` and `relay/bankofengland.co.uk/bank_rate(date)` have one `fn_hash`, and `function_tests`, keyed by hash, merged their tests: `describe` of `estr` counted the tests of `bank_rate` (21 instead of 1) and its function page listed their calls, results and counterexamples. Computed functions with identical bodies under two names shared a hash in the same way.
- **The fix (D23).** A function's own tests are the held tests whose `call` names that function. `describe` (`tests.count`, `branches`, `covered`, `counterexamples`) and the function page (`tests.count`, `cases`, `last_run`, `open_counterexamples`) read only those; a rerun on a new version of a cell or a rebind runs only the tests naming the function whose name changed, and two functions with one hash are both rerun.
- **Unchanged.** Foreign `<host>/test_case(function_hash, case)` cells stay per hash (spec 7.3 gives them no function name), so for stored functions of one signature a foreign counterexample applies to each; this is for a later spec round (D23). Test cells (`DATUMESH_TEST_CELLS=1`) are unaffected: the case id covers the call, so it differs per function.
- **The takeover page (D24).** `GET /takeover/<domain>`, the URL that relayed receipts and `describe` give as `source.takeover`, answered 404 up to 0.4.3. HTML (no script, everything escaped) or, by `Accept: application/json` or `?format=json`, `{domain, host, functions: [{name, page, fn_hash}], steps}`; cached like the function page. It lists the functions bound on this mesh whose relay source name is the domain (the detection `describe` uses for `source`), each linked to its `/fn` page, and says this mesh relays these values from the domain unchanged and that the domain remains their source. The steps: serve a key document at `https://<domain>/.well-known/datumesh.json` (spec 9); publish the same functions under `<domain>/...`, on its own mesh or hosted on this mesh. Hosting here says `POST /bundles` is open to the domain when `DATUMESH_PUBLISH_DOMAINS` admits it, else that the operator of the mesh must allow it (D11). It names no contact address and does not say the relay is withdrawn. A domain that is not a DNS name is `E_CANON_NAME` (400); a domain with no relayed function here is `not_covered` (404). The mesh document gains no member (the function page has none); the OpenAPI document gains the path. The functions are those with a catalogue entry (the page reads `catalog_summary`).

## 0.4.3 (2026-11-20)

For CONTRACT-build2-part5-feeds F2 (source-conformance tests of stored functions; QUESTIONS-build2 D16, D20, D21). Additive: no signature of the public API changes. Requires datumesh-core 0.3.10, datumesh-formula 0.5.1 and datumesh-store 0.5.1, as 0.4.2 does.

- **Tests of a stored function (spec 7.2, 7.3).** A `test` line of a publish-stored file (with the cells, in a tests-only file, or after them), and a bundle's signed `tests` member, for a stored function are stored in `function_tests` like a computed function's tests: the case is the first 16 hex digits of the SHA-256 of the test's JCS, and the same test is held once.
- **Running them.** A stored function's test reads the stored cell its call names, at the test's asof or else now, and compares its value with `expect` as typed values (numbers as numbers): equal is `pass`, different is `fail`, and a cell that is not stored, or cannot be read, is `error`. The result is recorded as before.
- **Never a refusal.** A failing or erroring test of a stored function never refuses a file or a bundle (stored functions stay exempt from the public rule); computed functions are unchanged.
- **Reruns.** A new version of a stored cell reruns the tests of its function, as does a rebind of its name, so the result follows the latest value.
- **describe.** A stored function's `tests` counts its stored tests, with `branches` and `covered` 0; its `counterexamples` counts its failing and erroring tests and the `false` `<host>/test_case` cells, as before.
- **Function page.** Each case of `tests.cases` gains `result` (`pass`, `fail` or `error`; null before its first run), additively; the HTML shows it beside each case. `tests.last_run.result` is the worst stored result (as before).
- **Test cells (`DATUMESH_TEST_CELLS=1`).** Each stored-function test is also the boolean cell `<host>/test_case(function_hash, case)`, trust `primary`, with `true` for a pass and `false` for a fail or error.
- **Feed jobs (deploy only, not in the crate), from the mesh-test rehearsal:** source-conformance tests on every run from an independent second copy, with a test window read from the first copy when needed (QUESTIONS-build2 D21); Treasury history from 1990, one year per call, 2 s between requests; a Treasury day without tenor values is a day without data; CHIRPS reads the LZW GeoTIFF (`imagecodecs` pinned), takes -9999 as nodata when the file has no tag, spaces rasters 120 s apart and backs off on 429 and 503; one stderr line per run with the cells and tests written.

## 0.4.2 (2026-11-20)

For spec-0.6.1 (CONTRACT-build2-spec-0.6.1 S1 to S3, S5 item 4). Requires datumesh-core 0.3.10 (`FunctionAnswer::with_publisher`, `Described::publisher`) and datumesh-formula 0.5.1. Additive: no signature of the public API changes.

- **DNS segments in names (S1, spec 2.3).** A namespace segment after the host may be a DNS name, as core 0.3.10 and formula 0.5.1 read it. A stored or computed function under `…/relay/ecb.europa.eu/fx_reference` publishes, is served by URI and by call, has its slices, its function page and its receipts verified; a formula reads it beside a divide or a minus. A DNS name in the last segment, or one that breaks 2.3's rules, is refused at publish with `E_CANON_NAME`.
- **`describe` names its publisher (S3, spec 16.8).** A function's and a class's answer gains `publisher`, the host of the namespace. A relayed function's `source` is its source domain, as before; a domain inside a name is never the publisher.
- **`find` names its publisher and source (S3, spec 9.6).** Each result gains `publisher` (the host), and a relayed function's result gains `source` (the domain), as separate members.
- **Counterexamples (S2, spec 7.3).** `tests.counterexamples` in `describe` and on the function page counts this mesh's own failing or erroring tests and the `false` `<host>/test_case` cells held for the function's hash, from any host: the latest version of each cell, each cell once. The function page's JSON gains `tests.open_counterexamples` (`signer`, `case`) and its HTML lists each one as an open counterexample with its signer. The store is not changed: the cells are read as a slice per held `<host>/test_case` binding, through the store's typed-argument index on `function_hash`. Counting does not depend on `DATUMESH_TEST_CELLS`.
- **`POST /bundles` and bundle import: the bare host for test cases (D20).** A bundle whose namespace is a bare host is accepted when it has at least one name and every name and catalogue entry is `<host>/test_case`; any other bundle of a bare host is refused as before (`E_BUNDLE_FORMAT`, 400). The domain proof, the signature and the rate limits are unchanged.
- **URI error code (spec 5.4).** A cell URI whose function name core refuses (a segment over 64 characters, a non-canonical name) is answered with core's canonical code (`E_CANON_LIMIT`, `E_CANON_NAME`), not `not_covered`.
- **Feed jobs under the final names (deploy only, not in the crate).** `deploy/prod/feeds/`: every name is spelled with the host, which must be given as `mesh-test.datumesh.com` or `datumesh.com` (no default). Relays at `<host>/relay/<domain>/<function>` (ecb.europa.eu, treasury.gov, bankofengland.co.uk); the licence register as the class `<host>/register/dataset(source, code)` with eight entries, verbatim licence clauses (`licences.toml`); rainfall as `<host>/earth/monthly_rainfall(dataset, region, month)` for 22 wheat regions from Natural Earth 10m admin-1, cells without `source` (QUESTIONS-build2 D17). State and output are kept per host.

## 0.4.1 (2026-11-19)

For CONTRACT-build2-part4-publish (datumesh-http part: P1, P2) and CONTRACT-build2-part5-feeds (F5, the http needs). Additive: `Config` gains `publish_domains` and `log_dir`, `PublishDomains` is new, `Client::key_document_live` is new, and the mesh document gains `endpoints.x_bundles`. A function's describe answer names its page in a `Link: <https://<host>/fn/<name>>; rel="describedby"` header, not in the answer: the answer is the spec's (16.8) and core checks it strictly.

- **`POST /bundles` (P1).** The body is a bundle (Content-Type `application/json`, at most 16 MiB and 50,000 receipts; an oversize body is read to its end and answered `413` `E_TOO_LARGE`). The checks run in this order and nothing is written unless all pass: the bundle parses (`E_BUNDLE_FORMAT`, 400); every name and catalogue entry is under the namespace (`E_NOT_OWNED`, 422); the host is allowed (`DATUMESH_PUBLISH_DOMAINS`, `E_DOMAIN_NOT_ALLOWED`, 403; a host that may not publish causes no outbound request); the rate limits (10 an hour per host, 60 per mesh; `rate_limited`, 429 with `Retry-After`); the domain proof, with the publisher's key document fetched now from `https://<host>/.well-known/datumesh.json` within 5 s (no answer is `E_KEY_UNAVAILABLE`, 422; a document that is not one, or holds no key of the manifest valid now, is `E_KEY_DOC`, 422) and the manifest signature (`E_BUNDLE_SIGNATURE`, 422); `import-signed`'s rules (`E_BACKDATED`, 422) and the public test rule (`E_TESTS_MISSING`, `E_TESTS_FAILING`, 422, with the lists). A `201` answers `{"imported", "names", "functions": [{name, fn_hash, page}]}`. The publisher's key document becomes the namespace's record, and the host becomes **hosted** on this mesh: its names and cells are served from the store from then on, and never fetched from the publisher again (the mark is a store blob holding a fixed per-host marker, until the store has a table for it).
- **`DATUMESH_PUBLISH_DOMAINS`** is a comma-separated list of hosts (spaces allowed), or `*` for any host with a valid key document. Empty or unset: the mesh's own namespaces only (D11). **`DATUMESH_LOG_DIR`**: every attempt is one line of `bundles.log` in it (the time, the host, the result, the counts; never the contents).
- **The function page (P2).** `GET /fn/<function name>`: server-rendered HTML with no script (publisher text escaped; sections `description`, `signature`, `body`, `tests`, `lineage`, `examples`, `licence`), or the same data as JSON by `Accept: application/json` or `?format=json` (members `name`, `fn_hash`, `description`, `args`, `signature`, `body`, `tests`, `lineage`, `examples`, `licence`). An unbound name is `404` `not_covered` (`E_NAME_UNBOUND`). The tests section gives the counts, the derived cases each covered or not, each test's call and expectation, and the last run. `lineage` gives the inputs and the dependants on this mesh, and for a relayed function its `source`. The description, signature and tests are the store's and are read at the request.
- **`describe` of a function** answers `source` for a relayed function, and its function page is the `Link: <...>; rel="describedby"` header of the response (not a member: the answer is the spec's, 16.8) `{name, relayed_by, takeover}` (core 0.3.9's `DescribeSource`). A function is relayed when one of its first recorded cells (up to 8, in slice order) has a receipt with a `source`; that is cached for 5 minutes per name and hash.
- **`publish-stored` `source` (F5, spec-0.6.0 C2).** A cell line may carry `source` `{name, url, retrieved, sha256?, takeover?}`; the receipt signs it (core's `ReceiptBuilder::source`), and a line that breaks the receipt's rules (`retrieved` after `issued`, a publisher that relays itself, an unknown member, a takeover that is not `https://`) is refused at its line with nothing written. A receipt with a `source` has the trust `relayed` (`TrustLabel::Relayed`).
- **Bundle import** (`bundle::import_signed` and its siblings) gives a failed test run the code as the text's prefix: `E_TESTS_MISSING: ...` or `E_TESTS_FAILING: ...` (`BundleError::Tests`). The variants and signatures are unchanged.

- **Feed jobs (Part 5, deploy only, not in the crate).** `deploy/prod/feeds/`: one fetch script per relayed source (ECB reference rates, €STR, ECB policy rates, US Treasury par yields, Bank of England Bank Rate), the CHIRPS regional summaries and the licence register. Each writes a `publish-stored` file of new values only, with `source` on every line, and runs it with the lanes from `.env`. Relay names come from one table, `names.toml`.
- **Fix: the embedding index gets a time limit of its own (found on mesh-test).** A batch of 32 passages took 2.1 s on the embed service's one CPU, past the 2 s query limit, so 0.4.0 never wrote an embedding and logged nothing. Indexing now sends 16 entries a call under a 60 s limit (`embed::INDEX_TIMEOUT`, `EmbedClient::embed_passages`), and a failed batch is logged as a warning and retried in 30 s. A query keeps its 2 s limit.
- **CI runs the pgvector tests:** the test job's database is the pgvector image the meshes run, with `DATUMESH_PGVECTOR_URL` set. Before, `tests/find_vector.rs` skipped itself in CI.

## 0.4.0 (2026-11-19)

For CONTRACT-build2-part3-calculus (datumesh-http part: E1 to E5 at the mesh). Requires datumesh-engine 0.5.0, datumesh-formula 0.5.0, datumesh-core 0.3.9 and datumesh-store 0.5.1 (its `function_tests`, the descriptor inputs index and the bound names). The public API is additive: `publish::publish_stored_checked`, `publish::publish_stored_checked_lanes`, `bundle::build_with_tests`, `BundleError::Tests`, `PublishErrorKind::TestsMissing` and `TestsFailing`, and `Config::test_cells`. `publish_stored*` and `build*` keep their behaviour and signatures.

- **`POST /whatif` (E1, E2; spec-0.6.0 5.9).** The body `{"cell", "asof"?, "overrides", "solve"?}`: value overrides `{cell, value}` and slice overrides `{slice, add, remove}` (at most 64), and `solve` with `vary`, `target`, `equals`, `lo` and `hi`. Cells are calls, cell URI rests, or cell objects. The answer is a `scenario` `{asof, cell, overrides, type, value[, solved]}`, labelled a scenario: no receipt, no trust, never stored and never served at `/cell`. Bad requests answer `bad_request` with the codes of 5.9; a scenario with no solution, or a fault in scope, is `not_covered`. The mesh document lists `endpoints.whatif`, and the OpenAPI document has `/whatif`.
- **Cycles (E3).** A cycle answers `error` `E_EVAL_UNAVAILABLE` with `cycle: {path, fixes}`: the path of canonical calls in read order, and the fixes `asof` and `solver`, plus `lag` through a `date` or `instant` argument and `step` through a `number` argument.
- **Tests for public functions (E5).** A `test` line in `publish-stored` (`{"test", "call", "asof"?, "inputs"?, "expect"}`), a bundle's signed `tests` member, and `publish-stored` (the binary) uses `publish_stored_checked`. A computed function is refused with `E_TESTS_MISSING` unless its tests cover the derived edge cases (zero divisors, absent inputs, negative numbers, declared extents and empty slices) and every branch arm; a failing test is refused with `E_TESTS_FAILING`. Stored functions are exempt. Refusals write nothing. Bundle import (`import_signed`) applies the same rule. The tests of a function are held in the store (`function_tests`), with their results.
- **`describe` of a function (16.8).** `GET /describe/<function name>` answers `kind` `function` with `tests`: `count`, `branches`, `covered` and `counterexamples` (failing or erroring tests). A class method's name is not described.
- **Rerun on rebind.** Binding a name reruns the tests of every function that reads it, transitively; the results are recorded, and failures never block the rebind.
- **Tests as cells (mesh-test).** With `DATUMESH_TEST_CELLS=1`, each held test is a boolean cell `<host>/test_case(function_hash, case)`, signed by the mesh, with trust `primary` (spec 5.5: host-signed, no inputs). Production names no test cells.

For CONTRACT-build2-part2-read (datumesh-http part: R2, R3, R5). `GET /find` changes shape, and the rest is additive: `Config` gains `embed_url`, `bundle::build_with_catalog` is new, and `build`, `manifest_bytes`, `import_signed` keep their signatures. Requires datumesh-store 0.5.0 (the catalogue, the dictionaries, the embeddings and the find misses), and `datumesh-engine` and `datumesh-formula` built against it.

- **Hybrid `GET /find?q=` (R3).** The answer is `{"query", "vector": bool, "results": [...], "resolution"?: {...}}`. Results are ranked exact, then text, then vector, then by score and kind. Each result keeps its members (`descriptor`, `fn_hash`, `since`, `class_hash`, `methods`, `class`) and gains `call` (the template with typed placeholders, or the filled call of a resolved request), `label`, `match` (`exact`, `text` or `vector`) and `score`. The default `limit` is 10 (was 20) and the most is 50 (was 100). A blank query lists the bound entries by name, as before.
- **Value resolution.** A query that reads as a request (a function plus values for its keys) gets `resolution`: `exact` (the filled `call` and a human `label`, such as `John 3:16`), `ambiguous` (`candidates`, and `keys` with at most three labelled embedding candidates per key, which fill nothing), or `range` (`E_RANGE_REFUSED` for `3:16-18`, `16–18`, `1-5`). A string key is filled only by an exact or labelled dictionary match (value, label or alias, case-insensitive after NFC). A number key takes the query's numbers in signature order. Words of the catalogue and the fixed list in `src/find_stopwords.txt` never take part in matching a value.
- **Embedding client (R2).** `DATUMESH_EMBED_URL` (absent: vector `find` is off). `POST <url>/embed` with `query: ` prefixes for queries and `passage: ` for indexed text, 2 s timeout. Any failure answers `"vector": false` with the text results, and no error. The mesh embeds the catalogue entries and dictionary values that have no embedding for `intfloat/multilingual-e5-small` at start and every 30 s (idempotent).
- **`GET /find?summary=1` (R2).** The catalogue summary for the MCP tool descriptions: `{"summary": true, "mesh", "classes", "functions"}`, each entry with its label, description, cell count and small dictionaries (at most 50 values per key, with `distinct`).
- **Demand record.** A query with no result and no resolution is written to the store's miss log (its text only, at most 512 bytes).
- **Catalogue in `publish-stored` (R5).** A `{"catalog": <name>, "kind", "label", "description", "args", "examples", "dictionary"}` line, checked before any write (the label at most 120 bytes, the description 1 to 1024, a dictionary value with a label). It is stored with `source=publisher`, signed by the run's key; a second run writes nothing. The string arguments of the file's cells are stored as dictionary values with an empty label, where no dictionary has them.
- **Catalogue in bundle 1 (R5).** An optional `catalog` member (entries sorted by name, in JCS), covered by the manifest signature: a changed entry fails import with `E_SIGNATURE`. `bundle::build_with_catalog` makes one; `build` makes a bundle without the member, as before.
- **The operator's catalogue file** (`DATUMESH_CATALOG_FILE`) is stored as `source=operator` entries at start. It never replaces a publisher's entry.

## 0.3.3 (2026-11-19)

Loading is batched; no change to receipts. `publish-stored` makes its reads in chunks of up to 1,000 cell lines (one `latest_many` and one `receipts_many` per chunk, the store's binding, descriptor and namespace answers kept for the run) and writes consecutive cells with `put_cells` in calls of up to 1,000 (datumesh-store 0.4.3). The receipts, versions, counts and errors are those of 0.3.2: the golden files of `tests/perf_load.rs` were recorded with 0.3.2 and still match byte for byte. Signing runs on up to `available_parallelism()` threads. The earliest failing line still wins, with its line number and text.

The cells of the second pass are written on parallel lanes by the new `publish::publish_stored_lanes` (added, `S: Store + Sync`, the arguments of `publish_stored_with`) and by the `datumesh-mesh publish-stored` command, which calls it. The lanes are `DATUMESH_PUBLISH_LANES` (a positive integer), default `min(4, available_parallelism)`; a cell's lane is its id's first hex digit mod the lane count, and a failing call stops the other lanes from starting new ones. `publish_stored`, `publish_stored_in` and `publish_stored_with` keep their 0.3.2 signatures (no `Sync` bound) and always write on one lane, in file order, as 0.3.2 did. `public-api.txt` gains that one line.

A latest slice first page (no `asof`) reuses the cached members of its slice when no member changed since the cached instant (`slice_changed_after`), so a repeated first page is a cache hit; its `next` links pin that instant. The page contents are unchanged. The Arrow stream's schema metadata `asof` is the instant the members answer as of: on a reused first page, the reused `at`, the same instant its JSON `next` pins, not the request's now. Otherwise the Arrow stream is unchanged.

## 0.3.2 (2026-11-18)

For CONTRACT-1.7a-text (datumesh-http part): describe gives each method its description. Additive: no
signature changes, no change to the hash, the other members, or the cache header.

- **Method descriptions (H1).** `GET /describe/{class}` gives each method of the class a `description`
  taken from the catalog entry `{class}/{method}` when that entry is 1 to 1,024 bytes (UTF-8 bytes,
  never truncated). An entry that is absent, empty or longer than that gives no `description` member.
  The class itself still has none (16.8). `/find` is unchanged.

## 0.3.1 (2026-11-16)

For CONTRACT-1.7a (datumesh-http part): descriptor lines, no partial writes, `/find` returns classes
and methods, and cells on the run's clock. Additive: the public API gains members and no signature
changes; `publish::Published` gains the public field `descriptors`, which `Published`'s `#[non_exhaustive]`
allows. A file that was accepted in 0.3.0 and is refused now is one whose writes would have failed
part way (see below).

- **Descriptor lines (H1).** `publish-stored` takes `{"descriptor": <function descriptor>}`: the
  descriptor is put (`put_descriptor`, its hash is the core's `FunctionDescriptor::hash`) and nothing
  is bound. Only a stored function is accepted (`code.lang` `"stored"`, `code.body` `""`, `inputs`
  `[]`); any other is refused at its line, in the first pass. A class later in the file, or in a later
  run, may name the hash as a method. `Published::descriptors` counts the descriptors newly put by
  descriptor lines (a re-run counts 0). A line with both `class` and `descriptor` is a class line.
- **No partial writes (H2).** The first pass now plans every write the second makes, and refuses the
  file before any write when one would fail: a bind whose start is not later than the latest binding of
  its name (the store's, or one planned earlier in the file, so a function line and a class line binding
  one name at one instant are refused, W1b); a name bound as the other kind (a class as a function, or the
  reverse); a suspended namespace; a cell whose function is not bound as of its `tx` (counting the file's
  earlier binds); a blob file that is missing or unreadable; `tx <= asof <= issued`; and an existing
  version at or after the cell's `tx` that is not this receipt. Receipts are signed in the first pass
  (signing is deterministic), so their hashes are known there. The store is compared before and after
  in the tests (`tests/publish_atomic.rs`).
- **Found by the production rehearsal of step 1.7a.** A class whose steward namespace has a path
  (`datumesh.com/scripture`) is accepted: the first pass asks about the steward namespace's host,
  which is what a suspension row is. A cell line without `tx` (it takes the run's clock) whose
  cell's latest version states the same thing (type, value, valid, licence, summary) is skipped, so
  a re-run at a later clock changes nothing; a changed value is still a new version.
  `datumesh-mesh publish-stored` prints the `descriptors` count.
- **Cells on the run's clock (H4).** A cell line may leave out `tx` and `asof`; each defaults to the run's
  clock now, independently, the same instant as the default binding start. `issued` defaults as before
  (the later of `asof` and the key's start). One file can bind and publish with one clock reading.
- **`/find` returns classes and methods (H3).** The catalog may list class names and method names
  (`N/m`). Each result has a `kind`: `function` (as before, plus `kind`), `class` (`class_hash`, the
  flattened `descriptor`, `methods` with each method's `fn`, `fn_hash`, `args` (the arguments after the
  keys) and `returns`, and `description` where the catalog has one, and `since`), or `method` (`class`,
  `fn_hash`, `descriptor`, `since`). A class is matched by its own name and description and by its
  methods' names and catalog descriptions. Order and `limit` are as before; a catalog name that is
  unbound as of now is skipped. `/openapi.json` documents the members.
- **Version 0.3.1 (H5).** The README and the `lib` endpoint table describe the new members.

## 0.3.0 (2026-11-16)

For CONTRACT-1.8b (item 9): rule A, `--backfill` with its operator log, `import-signed` and bundle 1, the
describe index over computed cells, and Windows resolver errors. `cargo public-api` against 0.2.2 shows additions only (`public-api.txt`),
but `publish::PublishError` gains the public field `kind`, and a binding or declaration that starts
before the clock is now refused where it used to bind (see below): a minor release, not a patch.

- **Rule A (CONTRACT-1.8b H1).** No binding (a name's, a class's or a class method's) and no
  declaration (`valid_from`, `valid_to`) starts earlier than the mesh clock. A start left out is
  now. `publish::publish_stored_with(store, key, host, owned, path, &PublishOptions)`, where
  `PublishOptions { clock, backfill }`; `PublishOptions::system()`. The run reads the file in two
  passes: the first works out the binds and checks their starts, and nothing is written unless
  every check passes. `PublishError` gains `kind: PublishErrorKind` (`Line`, `StartBeforeNow`,
  `Backfill`, `OwnedVectorName`). `keydoc::declare_at` (typed `DeclareError`). `datumesh-mesh
  declare`: `--valid-from` is optional.
- **`--backfill` (H1).** `publish-stored <file> --backfill` with `DATUMESH_BACKFILL_LOG` (mode 0600,
  one JCS line per run that writes: `{"at","namespaces","starts"}`). Starts are not checked, but
  a backfill is refused when a name it would bind has a binding, or a namespace it would write
  under has any binding (store 0.4.2's `has_bindings_under`, CONTRACT-1.8b R2; the log records the
  run and is no longer read for the check).
- **`import-vectors` (H1).** `vectors::import_in(store, dir, now, host, owned)`: refused as a whole,
  before any write, when a name it would bind is under the mesh's own namespaces
  (`PublishErrorKind::OwnedVectorName`). `vectors::import` is unchanged.
- **Bundle 1 and `import-signed` (H2).** `datumesh_http::bundle`: `build` (signed with the
  32-byte seed: RFC 8032 Ed25519 with curve25519-dalek and sha2, now direct dependencies),
  `manifest_bytes`, `import_signed` (checks in contract order, `BundleError`, `Imported`). Writes
  run descriptors, then receipts and versions, then the bindings last, so a crash leaves nothing
  answerable and a re-run completes the import. `docs/bundle-1.md`. The `datumesh-mesh
  import-signed <file>` command prints the result as one JCS line.
- **The describe index covers computed cells (H4).** `GET /describe/{id}` names the objects of
  every recorded cell of each method of a class, stored (`slice_values`) and computed
  (`computed_cells`, store 0.4.2; at most 1,000,000 per method, one warning when reached). A formula
  cell that is computed is announced to the index, so a short id of its object resolves. Nothing is
  evaluated to build the index. Store 0.4.2 is required (`datumesh-store` 0.4.2, with `sqlite` in
  the dev-dependencies).
- **Windows resolver errors are `dns` (H3).** `client::transport_detail`: the Windows resolver codes
  11001 to 11004, and the text "No such host is known", are `dns` (they were `network`).

## 0.2.2 (unreleased; no release yet)

For spec-0.5.0 (`c8896ad`): classes, describe, `resolved`, `E_NAMESPACE_SUSPENDED`; with the
domain validation (step 1.5) and the `E_FOREIGN_UNAVAILABLE` fetch codes of this branch.
On `datumesh-core` 0.3.8, `datumesh-store` 0.4.1 (the namespace checks and classes),
`datumesh-engine` 0.4.2 and `datumesh-formula` 0.3.0 (all four unpublished: the branch is built
against local patches; `Cargo.toml` names the minima). **Additions only** (`cargo public-api`
against 0.2.1: 59 lines added, none removed or changed: `API-CHANGES-0.2.2.md`), so a patch
release; none of formula's types is in the public API.

- **Classes in the mesh (16).** `publish-stored` takes a class line, `{"class": "<ns>/<name>",
  "descriptor": {...}, "bound": "<instant>"}`: defined with formula's `define_class` (the
  owner rule: the class is under one of the mesh's own namespaces, else `E_CLASS_OWNER`),
  then `put_class` and `bind_class` from `bound` (the methods are bound with it), unless the
  name is already bound to that class. Function lines take `classes`, `default` and
  `key_normalisation` in their signatures; the order is method functions, classes, the
  formulas that dispatch. `publish::publish_stored_in(store, key, host, owned, path)`;
  `Published::classes`. `vectors::import` imports a vector world's classes (and skips
  `invalid/formula-eval-class-dispatch-rebound`, which no store can load).
- **`/bindings` (9.7)** answers a class name with `class_hash` in place of `fn_hash`; a method
  `N/m` answers as a function.
- **Requests by call form (5.8, 16.3, 16.7).** At `/cell` and `/cells` a call is a method call
  (`N(keys).m(args)`, the same cell and receipt as the plain read), takes unquoted values by
  the position's type, labels (`N[label]`, through the class's `lookup`), the `asof` default
  (the call's, or the mesh's now, written in the cell URI) and bare names through the alias
  map. `call::Bindings` gains the provided methods `class`, `lookup` and `aliases_tx`;
  `Target::resolved`. A key not canonical under its rule is `bad_request`
  `E_KEY_NORMALISATION`, a malformed encoding `E_KEY_ENCODING` (`envelope::CALL_CODES`;
  `BAD_REQUEST_CODES` stays the 5.4 set).
- **`resolved` (5.5)** is in the envelope of every outcome that may carry it (`ok`, `stale`,
  `not_covered`, `unlicensed`, `payment_required`, `error`) whenever something was resolved
  (`envelope::with_resolved`); a plain or pinned call of a full name and a cell URI have none.
  An `@alias` call or a pinned alias now carries it. `aliases_tx` is absent on the mesh (its
  alias map has no snapshot).
- **describe (16.8, 9.6).** `GET /describe/{id}`, listed in the mesh document: `{id}` is a class
  name (`describe-class`: the flattened descriptor and the method table, `args` and `returns`
  from the method functions; no `description`, a function descriptor has none), an object's key
  encoding (`describe-object`, the keys checked against the class), an object id, a cell id (the
  object whose method cell it is) or a short id; never a class hash (404). Known objects are
  those of a class bound now that have a recorded cell of one of its stored methods (the
  store's slice reads with every argument open), kept in an in-memory index built on the first
  describe or short-id request and again when a stored version is announced
  (`Mesh::stored_version_added`), when a class is heard of, or after 30 s. A short id matching
  more than one object or cell is `bad_request` `E_SHORT_ID_AMBIGUOUS` (400). `Mesh::describe`.
  **Limits:** the store cannot list its classes or report its latest transaction time, so the
  index covers the classes the mesh has heard of (a stored version announced under them, a
  request or describe that resolved them, the classes of the configured aliases) and is
  rebuilt on a timer for versions another process adds; a class published by another process
  and never named is not covered until then. `same_as` is not emitted. `/cell/{id}` by cell id
  is not served.
- **`E_NAMESPACE_SUSPENDED` across meshes (9.9, 5.5).** A cross-mesh import the store refuses
  because the namespace is suspended is `not_covered` with that `error` (404, never an `error`
  envelope); `is_error_code` refuses it, the class codes and `CALL_CODES`.
- **Faults.** `E_FORMULA_KEY` and `E_FORMULA_DISPATCH_KEY` are `not_covered` with `fault`
  (engine 0.4.2; a guard test). `super` reads go through the engine's pinned reads
  (`foreign::Through` forwards `Inputs::read_pinned`).
- **`shadows` (16.9)** is never emitted: the test mesh has no reference classes.

Domain validation (step 1.5), as before:

- **`domain` module.** `Validator::validate(store, host, now)` fetches
  `https://<host>/.well-known/datumesh.json` and checks it with core 0.3.7's rules
  (`key_document_from_response`, declarations included). Valid: `put_namespace` (so
  `validated_at` is recorded and a rotation replaces the stored key document through the store's
  normal path), then `record_namespace_success` (the failure count back to zero, a suspension
  lifted). Invalid: `record_namespace_failure` with `SUSPEND_AFTER` = 2, so the second
  consecutive failure suspends the namespace. `validate_all` does every namespace in the store;
  the mesh's own namespaces are skipped (never suspended). `Outcome`, `Validation`.
- **Suspension** refuses new writes under the namespace with the store's
  `Error::NamespaceSuspended`: `put_receipt` of a receipt signed by the host, `put_version` of
  such a receipt, `bind_name` under the host (so `define`, `publish-stored` and the import of a
  foreign host's bindings and cells too). Reads, verification, the key document the mesh serves,
  and receipts already stored are not affected. A cross-mesh import the store refuses is
  `not_covered` with `E_NAMESPACE_SUSPENDED` (spec-0.5.0).
- **`datumesh-mesh validate <host> | --all`**: one JSON line per host (`host`, `outcome`
  `valid`/`invalid`/`skipped`, `failures`, `suspended`, `suspended_now`, `reinstated`,
  `rotated`, and for an invalid host `code` and `detail`); exit 1 when any host is invalid.
  `DATUMESH_PEERS` maps a host to another origin, as for cross-mesh resolution.
- **Every cross-mesh fetch that gets no HTTP response is `E_FOREIGN_UNAVAILABLE`** (502, with
  core 0.3.7's `ForeignDetail`: `network`, `dns`, `tls`, `timeout`, `refused`), not only the
  publisher's key-document refresh: key and mesh documents, bindings, descriptors, receipts,
  manifests, blobs, cells, slices, batches and versions fetched from another mesh
  (`Client`, so `foreign.rs`), and a name that cannot be resolved because its host cannot be
  reached. A mesh that answers keeps its own outcome (`E_FN_UNAVAILABLE`, a bad status, ...).
  A cell served before is still `stale`. The system resolver's "failed to lookup address
  information" is `dns`. Changed: `Client::key_document` of an unreachable host was
  `E_KEY_UNAVAILABLE`; `Client` methods that were `E_INPUT_UNAVAILABLE` for it are
  `E_FOREIGN_UNAVAILABLE` (the error's `foreign_detail()` says why).
- **A suspended namespace across meshes** is `E_NAMESPACE_SUSPENDED` (see above; it was
  `E_INPUT_UNAVAILABLE` behind `foreign::NAMESPACE_SUSPENDED` until core 0.3.8).
- **Weekly re-check**: `deploy/validate.sh`, a cron entry (Sundays 04:30 UTC) installed by
  `deploy/install_cron.sh`.

## 0.2.1 (unreleased)

For spec-0.4.3: licence inheritance and `declarations` in the key document. On
`datumesh-engine` 0.4.1 and `datumesh-core` 0.3.7 (were 0.4.0 and 0.3.6). Additive.

**A mesh that serves `declarations` is unverifiable by clients on core < 0.3.7 or datumesh <
0.4.3; publishers should wait for their consumers to upgrade before declaring.** The same holds
for @datumesh/verify < 0.4.3.

- **Licence inheritance (SPEC 8.5).** The engine (0.4.1) signs a computed receipt under the
  licence its input receipts give; where the rule refuses (`EngineError::Unlicensed`), the mesh
  answers `unlicensed` (403, no new code, the function hash echoed; `public, max-age` of
  `DATUMESH_LATEST_MAX_AGE`, never immutable: a declaration may come later). A cell that reads
  a refused cell is refused too. `DATUMESH_LICENCE` is now the licence of a computed receipt
  with no inputs. Computed receipts signed before 0.2.1 are signed again under the inherited
  licence (or refused) on their next request; nothing needs migrating.
- **Declarations (SPEC 9.2).** `/.well-known/datumesh.json` is the store's key document, as
  before, so the declarations it holds are served; without any, the bytes are 0.2.0's.
  `keydoc::with_declaration(host, existing, fn_name, licence, valid_from, valid_to)` adds an
  entry, leaves an identical one unchanged, or closes an open one (`keydoc::Declared`); entries
  are never removed. The binary's `datumesh-mesh declare --fn <name> --licence <licence>
  --valid-from <instant> [--valid-to <instant>]` applies it to `DATUMESH_HOST`'s document in the
  store. `keydoc::with_key` (key rotation, `keygen --publish`) keeps the declarations.
- **Another mesh's function, computed here.** Before evaluating a function under a host it does
  not own, the mesh puts that publisher's key document in the store (the key document cache's
  TTL, `DATUMESH_TTL_KEY_DOC`), where the engine reads its declarations
  (`foreign::Foreign::ensure_publisher`, also in the cross-mesh runtime layer). A publisher that
  gives no HTTP response then is `error` `E_FOREIGN_UNAVAILABLE` (spec-0.4.3: 502, with
  `detail` `dns`, `tls`, `timeout`, `refused` or `network`), or `stale` with a receipt served
  before. `client::Client::publisher_key_document` and `ClientError::foreign_detail` (the
  detail is the start of `ClientError::detail`). The other cross-mesh failures keep their
  0.2.0 codes.
- **Deployment (`deploy/`).** The seed binds `ecb/fx_cross_undeclared` (defined like
  `fx_cross`, not declared); `seed/declarations.py` lists the test mesh's two declarations
  (`ecb/fx_cross` over the restricted test licence, `scripture/strip_niqqud` over the UXLC
  licence); `licences.sh` applies both once after the image upgrade (idempotent); `smoke.sh`
  checks fx_cross over NOK (restricted licence, entitled), fx_cross_undeclared over NOK
  (`unlicensed`) and strip_niqqud (UXLC licence, entitled). The smoke, seed and vector checks
  use datumesh 0.4.3 and @datumesh/verify 0.4.3. The spec submodule is at `spec-0.4.3`.
- Public API: additions only (`public-api.txt`).

## 0.2.0 (unreleased)

On `datumesh-store` 0.4.0, `datumesh-engine` 0.4.0 and `datumesh-formula` 0.2.0 (were 0.3.2,
0.3.2 and 0.1.1), and a configurable base URL for the mesh document.

- **The store 0.4 move.** Breaking only because the public API names the store's, engine's
  and formula's types (`Mesh::builder` takes an `Arc<dyn Store + Send + Sync>`, `Mesh::engine`
  gives the engine, `BuildError::Engine` carries its error), which are now store 0.4's, engine
  0.4's and formula 0.2's: users move to the four together. No code change was needed for it.
  Store 0.4.0 adds a SQLite backend (`SqliteStore`), which a `Mesh` can run on (the store's
  `Store` is the same trait) when the application enables it (`datumesh-store`'s default
  feature `sqlite`, or `datumesh-engine/sqlite`). **The image's backend is unchanged:
  PostgreSQL** (`DATABASE_URL` names a PostgreSQL 16 database, as before): this crate depends on
  the store with `default-features = false`, and on engine and formula without their `sqlite`
  features, so the mesh binary and the image compile no SQLite (`cargo tree -e features` has no
  `rusqlite` or `libsqlite3-sys`; `Cargo.lock` holds neither).
- **A base URL for the mesh document** (`Config::base_url`, `Config::with_base_url`,
  `DATUMESH_BASE_URL`): when set, every URL of `/.well-known/datumesh-mesh.json` (`bindings`
  and the endpoints) is on it instead of `https://<host>`, so a mesh on this machine (`datumesh
  serve`) can advertise `http://127.0.0.1:<port>`. `https://` with any host. `http://` only
  when the mesh's host is unrooted (under `.invalid`, SPEC 2.5, e.g. `local.invalid`), and
  then only on `127.0.0.1`, `[::1]` or `localhost`; a rooted host is never served over
  `http://`. An optional port; no path, query, fragment or trailing slash. Another is refused
  when the mesh is built, `BuildError::BaseUrl` with a `config::BaseUrlError` (`Scheme`,
  `HttpRootedHost`, `HttpNotLoopback`, `Host`, `Port`, `Path`, `TrailingSlash`, `Query`,
  `Fragment`), or by `Config::from_env` and `Config::check` as a `ConfigError`;
  `config::check_base_url(host, url)` checks one alone, `config::is_unrooted(host)` tells an
  unrooted host. The key document and `host` do not change, nor do the URLs in envelopes and
  headers (`poll`, a page's `next`, the `canonical` link), which stay on `https://<host>`
  (`Config::origin`): core's `Pending` takes an `https://` `poll` only. Unset (the default),
  the mesh document is 0.1.2's byte for byte.
- **An `http://` mesh document is local-only.** It is not valid under spec-0.4.3 (9.6: every
  URL `https://` on the host), and core 0.3.6's `resolve_mesh` (so `client::Client::resolve`)
  refuses it with `E_MESH_DOC`. spec-0.4.4 will allow exactly this: for an unrooted host,
  mesh-document URLs and `poll` may be `http://` on a loopback address only (`127.0.0.1`,
  `[::1]`, `localhost`); rooted hosts stay `https://`-only. datumesh-core 0.3.8 will accept it.
- Additive otherwise: `Config::base_url`, `Config::with_base_url`, `Config::mesh_document_base`,
  `config::BaseUrlError`, `config::check_base_url`, `config::is_unrooted`,
  `BuildError::BaseUrl`.

## 0.1.2

The first published release of this work. The tag `v0.1.1` marks a release run that stopped before publishing (the batch was over 10 times one cell on CI; fixed below), so 0.1.1 was never published.

spec-0.4.2 (datumesh-core 0.3.6, datumesh-engine 0.3.2, datumesh-formula 0.1.1; store 0.3.2).

- Pins: `/cell/name@<fn_hash>(args)` and `&fn_hash=` in a cell URI; every answer echoes
  `fn_hash`, `pending` in its `request`. An unknown pin is `not_covered`.
- Trust `computed` for receipts this mesh signed (not `primary`); envelopes built with core's
  `Answer` builder.
- `GET /slice` JSON answers are page envelopes (SPEC 5.6); Arrow is unchanged.
- `POST /cells` answers `[{"envelope", "status"}]`, or one envelope refusing the whole batch
  (SPEC 5.7).
- `POST /cells` is as fast as in 0.1.0 again (1,000 cells under 10 times one cell, the
  brief's criterion 8): each chunk writes and counts its own answers on the pool, a batch
  looks each binding up once (not once per chunk), and the core count is read once.
- `E_SLICE_AT_CELL` (`bad_request`) for a slice marker at `/cell`; `E_NAME_UNBOUND`
  (`not_covered` with `error`) for an unbound name.
- One manifest path, `/manifest/<slice id>/<members hash>` (`/slice/<hex>/<hex>` is gone).
- `GET /versions/<uri>` (SPEC 9.8), listed in the mesh document; the job lifetime text is
  "while the mesh runs".
- `client`: a store-less, blocking client for other meshes, every answer checked with core;
  `foreign` (cross-mesh import) is now a thin layer on it, and uses `versions` when a mesh
  lists it. `foreign::ForeignError` is now a type alias of `client::ClientError` (the same
  fields and traits).
- `publish`: `publish_stored` and `datumesh-mesh publish-stored <file.jsonl>`: stored and
  formula functions and their cells (blobs included) signed as the mesh, idempotently.
- `vectors`: `import-vectors` loads every formula and excel vector's function (by hash), so
  each answers by pinned hash; names only invalid vectors bind are bound too.
- Additive: `call::Target::pin`, `pinned_reference`, `call::format_pinned`, the provided
  `Bindings::pinned` and `Bindings::alias`.
- The test mesh (`deploy/`): the seed data (`deploy/seed.sh`: the synthetic ECB-shaped cells
  and the scripture corpora), the smoke suite and the drill for 0.1.1.

## 0.1.0 (unreleased)

The first release: the Datumesh engine (datumesh-engine 0.3.1, the formula runtime of
datumesh-formula 0.1.0, datumesh-store 0.3.2, datumesh-core 0.3.5) behind HTTPS, spec-0.4.1.

- Endpoints: `GET /cell/<call or uri>`, `POST /cells`, `GET /slice/...` (JSON pages by key,
  Arrow IPC), `GET /receipt|descriptor|manifest|blob/...`, `GET /explain/...`, `GET /find`
  (a stub), `POST /verify`, `GET /jobs/<id>`, the key and mesh documents, `GET /bindings`,
  `GET /health`, `/metrics`, `/openapi.json`.
- Envelopes of every outcome of SPEC 5.5, `pending` included (HTTP 202, jobs kept 300 s),
  canonical JSON; a call and its cell URI give the same bytes.
- Cross-mesh resolution, minimal form: foreign names imported into the store (bindings,
  descriptors, receipts verified against their signer's key document).
- Rate limits that refuse, never queue; caching headers; API keys hashed in a 0600 file.
- `datumesh-mesh`: serve, migrate, keygen (rotation), apikey, openapi, health; the Docker
  image and `docker-compose.yml`.
