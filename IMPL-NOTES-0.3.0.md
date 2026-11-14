# datumesh-http 0.3.0 (classes, describe, resolved): notes for the implementer

For `/root/datumesh-libs/CONTRACT-http-0.3.0-classes.md`. The tests are written (three commits on the
branch `step-1.5-domain-validation`, on top of `77aa1c7`); yours are the implementation commits, one per
tests commit, in the same order. This file is committed with the first tests commit and covers all
three. The limits of the contract hold: do not push, tag, publish or deploy; do not touch the running
mesh containers or their volumes, `/root/crates-api-token` or `/root/cf-key`; remove build directories
you create over a few GB (a debug build of the tests here is about 14 GB: `target-1.99/`).

## 0. How to build

* **Spec.** `spec` is at `c8896ad` (the first tests commit moves it).
* **Dependencies** are unpublished. The committed `Cargo.toml` names the published versions
  (`datumesh-core` 0.3.8, `datumesh-store` 0.4.1, `datumesh-engine` 0.4.2, `datumesh-formula` 0.3.0).
  Locally, an uncommitted `.cargo/config.toml` (excluded through `.git/info/exclude`, already in the
  working tree) patches them to paths inside the cargo container:

  ```toml
  [patch.crates-io]
  datumesh-core = { path = "/core/crates/datumesh-core" }       # core 0.3.8, 6780c7c
  datumesh-store = { path = "/store/crates/datumesh-store" }    # store 0.4.1, db69efc (step-1.5-namespace-validation)
  datumesh-engine = { path = "/engine" }                        # engine 0.4.2, eb727b7
  datumesh-formula = { path = "/formula" }                      # formula 0.3.0, b6dd484
  ```

  The container is `tools/cargo.sh`'s with four more read-only mounts: `-v
  /root/datumesh-libs/datumesh-core:/core:ro -v /root/datumesh-libs/datumesh-store:/store:ro -v
  /root/datumesh-libs/datumesh-engine:/engine:ro -v <formula>:/formula:ro`. **Formula:** b6dd484 (the
  coordinator's update after the tests were first written against a6fd557: it has the c8896ad
  re-pin and `super` through the engine). Its working tree may have uncommitted changes: mount a clean
  `git archive b6dd484` of it.
* `Cargo.lock` is not committed with the tests: a build rewrites it for the patches; `git checkout
  Cargo.lock` before committing (locks are fixed at release time).
* The tests need PostgreSQL: `DATABASE_URL` as `tools/cargo.sh` sets it (`datumesh-pg` on the `datumesh`
  network). A full `cargo test` takes about 6 minutes (`tests/slice.rs` alone about 3).

## 1. The commits and what fails

| # | Tests (this file's author) | Files | Failing now |
|---|---|---|---|
| 1 | publish-stored class lines, function lines with `classes`/`default`/`key_normalisation`, the bindings resource, a suspended namespace across meshes, faults | `tests/classes_publish.rs` (4), `tests/classes_faults.rs` (2, guards), `tests/domain.rs` (one test changed), `tests/common/{mod,world}.rs`, the API stubs | 5 failing: `classes_publish` 4, `domain` 1 (`classes_faults` 2 pass: guards) |
| 2 | requests by call form: method-call equivalence over HTTP, `resolved`, defaults, labels, aliases, key errors, the batch; the class call vectors through `call::resolve`; the old tests whose answers changed | `tests/classes_requests.rs` (10), `tests/classes_call_vectors.rs` (2), `tests/syntax.rs`, `tests/pins.rs` | 14 failing: `classes_requests` 10, `classes_call_vectors` 2, `syntax` 1, `pins` 1 |
| 3 | describe, short ids, the mesh document | `tests/classes_describe.rs` (7, one `#[ignore]`d tool), `tests/base_url.rs` | 8 failing: `classes_describe` 6 (+1 ignored tool), `base_url` 2 |

The first commit also holds what all three share: the dependency minima and `spec` at `c8896ad`, the
API stubs of section 2 (everything compiles; clippy `-D warnings`, `cargo fmt --check` and the docs pass),
and `tests/common` (`world.rs`: the test classes; `mod.rs`: `without_resolved`, the describe schemas).

`tests/classes_faults.rs` already passes: engine 0.4.2 and formula 0.3.0 answer `E_FORMULA_KEY` and
`E_FORMULA_DISPATCH_KEY` as `not_covered` with `fault` through the existing `Answer::NotCovered` mapping,
so contract item 7 needs no code here. The two tests are guards (they pin it through HTTP).
Contract item 9 (domain validation, fetch codes): `tests/domain.rs` and `tests/foreign_unreachable.rs`
pass on the new dependencies, as do the other old tests but the ones listed in section 5.

## 2. The API (the stubs are in the tree; they compile; behaviour is `todo!()` or absent)

Breaking or not is for `cargo public-api` against 0.2.1 to say; the stubs below are additive, but the
formula 0.3.0 types in the public API (`call::Env`... are private; `Callees`, `Bindings`) may still make
the release 0.3.0.

```rust
// call.rs
pub struct BoundClass { pub class_hash: String, pub since: String, pub descriptor: Arc<ClassDescriptor> }  // new
pub trait Bindings {
    ...
    fn class(&self, name: &str, asof: &str) -> Result<Option<Arc<BoundClass>>, ErrorCode>;                // provided: None
    fn lookup(&self, class: &str, label: &str, asof: &str) -> Result<Option<String>, ErrorCode>;          // provided: None
    fn aliases_tx(&self) -> Option<String>;                                                                // provided: None
}
pub struct Target { ..., pub resolved: Option<datumesh_core::Resolved> }   // new pub field; Target has private fields, so not breaking
// mesh.rs
impl Mesh { pub fn describe(&self, id: &str) -> Reply }                    // new; STUB todo!()
// http.rs: route GET /describe/{*id} (a stub handler over Mesh::describe; no OpenAPI annotation yet)
// publish.rs
pub fn publish_stored_in(store, key, host, owned: &[String], path) -> Result<Published, PublishError>   // new; publish_stored is it with [host]
pub struct Published { ..., pub classes: usize }                           // #[non_exhaustive]: additive
```

What each is for, and what the tests need of it:

* **`Bindings::class`** (a class name as of an instant, from `Store::resolve_class_binding` and
  `Store::class`): what formula's `CallEnv::bound_class` is answered from. `call::Env` (private, in
  `call.rs`) must implement `bound_class`, `now` (the `now` it is given), `lookup` and `aliases_tx`
  from `Bindings`; today it implements none of them, so formula sees no class and no clock. **`Memo`
  (`mesh.rs`) must forward `class` and `lookup` to the `Inner`** as it forwards `pinned` and `alias`
  (the provided defaults answer none, silently).
* **`Bindings::lookup`**: the cell `<class>/<lookup method>(label: <label>)` read as of `asof` (the
  class's `lookup` member names the method; it is a stored function returning a key of the class); its
  value is the key encoding. `None` when the class has no lookup or the cell has no value.
* **`Target::resolved`** is `Call::resolved` of formula's `parse_call`, kept when `resolve_call` goes on
  to `resolve_uri` (the target the URI resolves to may come from the target cache: clone it, set
  `resolved`, do not cache the clone).
* **`resolved` in the envelope.** The answers are built from cached bytes (`ReceiptInfo::ok`), so
  `resolved` is added per request, to every outcome that carries it (`ok`, `stale`, `not_covered`,
  `unlicensed`, `payment_required`, `error`: 5.5 lists it for these, not for `bad_request`,
  `rate_limited` and `pending`). `datumesh_core::Answer::with_resolved` builds it. A plain or pinned call
  of a full name, and a cell URI, have none. `aliases_tx` is present exactly when an alias snapshot was
  used: the mesh's configured alias map has none, so `Bindings::aliases_tx` answers none there (the
  tests do not assert it on the mesh; `classes_call_vectors.rs` does through a `Bindings` that has one).
* **`Mesh::describe(id)`** and the route: section 4.
* **`publish_stored_in`**: the owner rule needs the mesh's own namespaces (`Config::namespaces`; the
  CLI passes them). The namespace of a class is its name up to the last `/`; it is owned when its host
  (first segment) is one of `owned`. Core's `define_class` takes the definer namespace: pass the
  class's own namespace when its host is owned, any other string (the mesh's host) when it is not, and
  core answers `E_CLASS_OWNER` itself. The line's error is `PublishError { line, detail }` whose
  `detail` must contain the code (`E_CLASS_OWNER`, `E_CLASS_METHOD`...: `DefineError`'s `Display` has it).
  The tests do not change `PublishError`.

### Smaller things the tests pin

* `envelope::BAD_REQUEST_CODES` is asserted by the old `tests/contract.rs` to be exactly the codes of
  5.4's URI parse (`every URI code reached`). **Do not add the new codes to it** unless you also make
  that test reach them: keep a second constant (for example `CALL_CODES`: `E_KEY_NORMALISATION`,
  `E_KEY_ENCODING`, `E_SHORT_ID_AMBIGUOUS`) and make `is_request_code` accept both. `is_error_code`
  must refuse the three, `E_NAMESPACE_SUSPENDED` and the eight class codes (5.5: no `error` envelope
  carries them).
* `foreign::NAMESPACE_SUSPENDED` becomes `ErrorCode::NamespaceSuspended`, and `Inner::unavailable`
  answers it `not_covered` with that `error` (`Answer::not_covered().with_error(..)`, status 404,
  `Cache::NoStore`, the function hash echoed), never an `error` envelope and never `stale` instead
  (a cell served before is served as before: it is not an import).
* `publish-stored` function lines already carry `classes`, `default` and `key_normalisation` (core's
  `FunctionDescriptor::from_value` takes them), and formula lines with them (formula 0.3.0): only the
  `class` line is new. **`publish.rs`'s `StoreCallees` must answer `bound_class`, `class_descriptor`
  and `function_descriptor`** (the `Callees` defaults answer none): a class's methods are found by
  hash, and a formula that dispatches needs its classes bound (`E_FORMULA_UNRESOLVED` otherwise).
* `class_lines_define_store_and_bind` publishes in the order *method functions (bound at
  `BIND_TX`), classes (`bound` later), the formulas that dispatch (later again)*: `Store::bind_class`
  wants a `tx` later than the latest binding of every method name.
* `tests/vectors.rs` (old) is red until `vectors::import` (`src/vectors.rs`) also imports a vector
  world's `classes` (`put_class` and `bind_class` from each `bound`, after its functions): six
  `formula-eval` vectors of spec-0.5.0 (`class-dispatch-bond`, `-swap`, `class-super` and the three
  `invalid/formula-eval-class-dispatch-*`) answer `E_EVAL_UNAVAILABLE` today because their classes are
  not in the store. It failed before this work's first line: the spec move alone does it.

## 3. The test world (`tests/common/world.rs`)

Modelled on the live check (the seed's descriptors carry no key rule):

| Class | Keys | Methods |
|---|---|---|
| `<h>/scripture/verse` | `corpus`, `book` (strings, `none`), `chapter`, `verse` (numbers, unit 1) | `text` (stored: `edition` after the keys; returns a blob; the seed's `verse_text`), `consonantal` (a formula over `text`, the keys first) |
| `<h>/ecb/fx_pair` | `base`, `quote` (`none`) | `rate` (stored, the seed's `fx_rate`), `cross` (a formula; `date` has `"default": "asof"`) |
| `<h>/ecb/fx_upper` | `base`, `quote` under `upper_ascii` (the method functions' key arguments carry the rule) | `rate` (stored) |
| `<h>/ecb/fx_named` | `base`, `quote` (`none`), `lookup`: `named` | `rate` (stored), `named` (stored: a label to a key encoding) |

and the formulas `<h>/ecb/upper_rate(pair, date)` (`pair.rate(date: date)`, `pair` declaring
`classes: [fx_upper]`: a dispatch) and `<h>/ecb/lower_rate(x, date)` (reads `fx_upper/rate` with
`lower(x)`: the fault `E_FORMULA_KEY`). `install(&node)` builds all of it in the node's store with
formula's `define_class`/`publish_class` and the store's own API (so the request and describe tests
do not depend on the `class` line of `publish-stored`); `classes_publish.rs` builds the same world
through `publish_stored_in` and compares the class hashes.

## 4. describe and short ids (items 5 and 6)

* **Route and endpoint.** `GET /describe/{*id}` (an id may be a class name, which has slashes, or a
  percent-encoded key encoding); the mesh document lists `"describe": "<base>/describe/{id}"`
  (`mesh_document`, sorted: `describe` sorts before `descriptor`). `tests/base_url.rs`'s byte-for-byte
  document gains that member (the test is changed in the third tests commit, deliberately).
* **What `{id}` may be**, and the answer:
  * a **class name** bound as of now, or a **class hash** the mesh has bound: `kind: "class"`. By hash
    the `class` member is the name it is bound to (the first in ascending order of name if several);
    the body is byte-identical to the one by name (the tests compare). Unknown: `404`.
  * an **object's key encoding** (16.5) of a bound class: `kind: "object"`. Any well-formed encoding
    describes an object (objects are never stored); the keys are decoded and checked against the class
    as `check_describe_with` does (names, types, `key_normalisation`).
  * an **object id** or a **short id** (12 lowercase hex digits): the objects the mesh *knows*: those
    of a bound class that it holds a cell of a method for (a stored version, or a cell it computed).
    The cell's arguments that are the class keys give the object. How the mesh finds them is yours (a
    scan of the class's method functions' cells, or an index kept when a cell is stored or computed);
    the tests only need the objects of stored cells found (`install` stores them with
    `put_value` followed by `stored_version_added`). Unknown: `404`; **more than one object with that
    short id: `400`, `bad_request`, `E_SHORT_ID_AMBIGUOUS`** (an envelope of 5.5 with `error` only). An
    object id (64 digits) is never ambiguous.
* **The answers** are the `describe-class` and `describe-object` documents: exactly the members of
  16.8, canonical JSON (the tests do not require JCS, but the mesh writes everything so), `200`,
  `application/json`. The class answer's method table has, for each method of the flattened
  descriptor, `fn` = `<class>/<method>`, `fn_hash`, `args` = the method function's signature `args`
  without the class keys (each declaration as in the descriptor: `valid`, `default`...),
  `returns` = its `returns`; **no `description`** (a function descriptor has no metadata, 7; the
  contract says "if it has any": it has none; keep the code ready, the tests assert absence). The object
  answer has `object`, `object_id`, `short_id`, `call` (`datumesh_core::ObjectKey::call_form`),
  `methods` (the class's method names, ascending) and no `same_as` (the test classes have no `same_as`
  method; implement it for a class that has one if cheap: it is the keys that method's cells return).
  `ObjectKey::object_value`, `object_id` and `short_id` are core's.
* The tests check with `datumesh_core::check_describe_with` against the classes and the method
  descriptors the **test** takes from the store, never from the answer, so a wrong `class_hash`, a
  wrong table or a wrong `object_id` fails there.
* **Not tested, your call:** a **cell id** as `{id}` (9.6 lists "a cell id" among the ids; a cell is not
  a class or an object, so there is nothing to describe: `404` is suggested); `/cell/{id}` by cell id or
  short id (the mesh does not serve a cell by id today: the contract says "if the mesh serves that");
  the object **call form** (`N(base: "EUR", quote: "USD")`) as `{id}`: formula does not expose an
  object parser, only the key encoding is tested (the call form of the answer is core's
  `call_form`; accepting it back is optional).
* **`COLLISION`** in `classes_describe.rs`: two keys of `fx_pair` whose objects share a short id (a
  48-bit collision, found once with the ignored test `find_a_short_id_collision`, Floyd's cycle search,
  a few minutes). It depends on the class hash of the test `fx_pair`: if you change the world's
  descriptors, the test says so and how to regenerate it.

## 5. The old tests that changed, and the ones that must keep passing

Changed by the tests commits, because the spec moved their answers (each with a comment):

* `tests/domain.rs::a_suspended_foreign_namespace_is_not_imported_from`: `404 not_covered`
  `E_NAMESPACE_SUSPENDED` (was `503 error` `E_INPUT_UNAVAILABLE`); the envelope passes core's check.
* `tests/syntax.rs`: an unquoted token is a value by position type (5.8), so `rate(USD, 2026-03-01)` is
  a call, not `E_PARSE_URI`, and `rate(1, ...)` is the string `"1"`; an `@alias` call answers the URI's
  answer plus `resolved` (compared with `without_resolved`).
* `tests/pins.rs`: `@fx@<hash>(...)` likewise carries `resolved` (a pinned alias is a resolution).
* `tests/base_url.rs`: the mesh document has the `describe` endpoint.

Must keep passing as they are (they failed or would fail under a careless implementation): everything
else in `tests/`, in particular `contract.rs` (`BAD_REQUEST_CODES`, see section 2), `licence.rs`,
`cross_mesh.rs`, `foreign_unreachable.rs` and `domain.rs` (items 9). `tests/vectors.rs` is red before
and after the tests commits until `vectors::import` imports classes (section 2).

## 6. Where the tests could not be exact

* **Formula.** The tests were written against a6fd557, which answered two vectors of `c8896ad` wrongly
  (H3 quoted key encodings; an extension without the class keys). With b6dd484 they pass:
  `classes_call_vectors::the_vectors_of_the_formula_repin_resolve_as_they_say` and
  `classes_requests::a_quoted_encoding_is_decoded_against_its_class` (checked against the throwaway
  implementation below). The split of that vector test is historical; merge it if you like.
* **Vectors and licences (coordinator).** A `formula-eval` or `excel` vector's `receipt_hash` assumes
  `licence: CC0-1.0` (SPEC 11); a mesh on engine 0.4.1+ signs the inherited licence, so a test comparing
  a computed receipt's hash with a vector's must first set `licence` to `CC0-1.0` (renaming computed inner
  inputs and slice members likewise): see datumesh-formula `tests/common/engine.rs`, `convention_hash` /
  `convention_inputs`. `tests/vectors.rs` compares values and faults only, so it is not affected.
  `invalid/formula-eval-class-dispatch-rebound` cannot be loaded through a store (its class names a
  method hash the world lacks, by design): `vectors::import` must skip it, with that reason, and the
  by-pinned-hash test must not count it as failing.
* **Validation of the tests.** They were checked against a throwaway implementation (not committed): the
  `class` line of `publish.rs`, `Bindings::class`/`lookup`/`aliases_tx`, `call::Env`, `resolved`
  injection, the bindings answer, `describe` of a class by name and of an object by key encoding, and
  the mesh document. With it every test of `classes_publish.rs`, `classes_requests.rs` (but the formula
  one above), `classes_call_vectors.rs` (but the re-pin one), `classes_describe.rs`'s class, object and
  mesh document tests, and the changed old tests (`syntax`, `pins`) pass. **Not run against anything:**
  describe by class hash, by object id and by short id, `E_SHORT_ID_AMBIGUOUS`, and the foreign
  `E_NAMESPACE_SUSPENDED` mapping was checked with a four-line change (the placeholder constant and
  `Inner::unavailable`) and passes. Read the three describe tests that were not run against the spec.
* **The store has no index for describe.** `datumesh-store` 0.4.1's `Store` trait (what `Mesh` holds, a
  `dyn Store`) cannot list class names, give the names a class hash is bound to, or enumerate the cells
  of a function (but `slice_members`, which enumerates a stored function's cells for a slice, can).
  Describe by class hash (the `class` member is a name) and by object id / short id (the objects the mesh
  knows) therefore need something the mesh keeps itself, or a store method. That is a design decision
  for the implementer (and, for a store method, for the coordinator); if describe by hash or by id is
  not in 0.3.0 after all, say so in the report and drop `describe_a_class_by_hash`'s hash half and the
  two id tests. Suggested: an in-memory map filled when the mesh resolves a class name or sees a
  stored/computed cell of a class method (`Mesh::stored_version_added` is the hook for stored cells),
  rebuilt from the bindings of the names it is asked about; or `slice_members` over each method's
  function with every key argument `*` for the stored methods.

## 7. `shadows` (16.9, contract item 8)

The test mesh has no reference classes, so it never emits `shadows`, and `Mesh` has nothing to compare a
class name against. Implement nothing; document it in the crate docs and the CHANGELOG. The requests
tests assert that no envelope the mesh writes carries `shadows`.

## 8. Order of work, suggested

1. Commit 1: `StoreCallees` (class methods), the `class` line, `publish_stored_in`, the bindings
   answer, `NAMESPACE_SUSPENDED`; the CLI's `publish-stored` passes `Config::namespaces`.
2. Commit 2: `Bindings::class`/`lookup`/`aliases_tx`, `call::Env`, `Target::resolved`, the envelope's
   `resolved`, the codes; `vectors::import` classes.
3. Commit 3: `Mesh::describe`, the route, the mesh document, short ids; OpenAPI for `/describe/{id}`;
   README and crate docs (calls, `resolved`, describe, short ids, `shadows`), CHANGELOG,
   `public-api.txt`, `API-CHANGES-<version>.md`, the CLI's usage text.
