# API changes in datumesh-http 0.2.2

Generated with `cargo public-api -ss --color=never` (cargo-public-api 0.52.0, nightly-tools image)
on this branch against the `public-api.txt` of the v0.2.1 tag; the full diff is at the end.
**Additions only** (59 lines added, none removed or changed; this includes the 42 lines of the
domain validation, which were committed with it), so a patch release. None of `datumesh-formula`
0.3.0's types is in the public API; the engine 0.4.2, store 0.4.1 and core 0.3.8 types that are
(`Runtime`, `Inputs`, `Engine`, `Clock`, `Store`, the core model) changed additively.

| Item | What |
|---|---|
| `call::BoundClass { class_hash, since, descriptor }` | New: a class name's binding as of an instant (SPEC 9.7): the class hash and the flattened descriptor. |
| `call::Bindings::class`, `lookup`, `aliases_tx` | New **provided** methods of the `Bindings` trait (defaults: no class, no lookup, no snapshot). Implementors that do not override them are unaffected. |
| `call::Target::resolved` | New public field (`Option<datumesh_core::Resolved>`); `Target` has private fields, so no one outside can construct it and no code breaks. |
| `envelope::CALL_CODES` | New: `E_KEY_NORMALISATION`, `E_KEY_ENCODING`, `E_SHORT_ID_AMBIGUOUS`, `bad_request` codes besides `BAD_REQUEST_CODES` (unchanged: the 5.4 set). |
| `envelope::with_resolved(reply, &Resolved)` | New: adds the `resolved` member to an envelope that may carry it. |
| `publish::publish_stored_in`, `Published::classes` | New: `publish-stored` with the mesh's own namespaces (the class owner rule); `Published` is `#[non_exhaustive]`. `publish_stored` is `publish_stored_in` with `[host]`. |
| `Mesh::describe(&self, id) -> Reply` | New: the describe answer (SPEC 16.8). |

Behaviour changes without API changes: a call may be a method call, take unquoted values, labels,
the `asof` default and bare names; envelopes carry `resolved`; a suspended namespace across meshes
is `not_covered` `E_NAMESPACE_SUSPENDED`; `/bindings` answers `class_hash` for a class name;
`GET /describe/{id}` and the mesh document's `describe` endpoint; `envelope::is_error_code` refuses
`E_NAMESPACE_SUSPENDED`, the class codes and `CALL_CODES`. See the CHANGELOG.

## Full diff

```text
Removed items from the public API
=================================
(none)

Changed items in the public API
===============================
(none)

Added items to the public API
=============================
+pub struct datumesh_http::call::BoundClass
+pub datumesh_http::call::BoundClass::class_hash: alloc::string::String
+pub datumesh_http::call::BoundClass::descriptor: alloc::rcs::arc::Arc<datumesh_core::class::ClassDescriptor>
+pub datumesh_http::call::BoundClass::since: alloc::string::String
+impl core::clone::Clone for datumesh_http::call::BoundClass
+pub fn datumesh_http::call::BoundClass::clone(&self) -> Self
+impl core::fmt::Debug for datumesh_http::call::BoundClass
+pub fn datumesh_http::call::BoundClass::fmt(&self, &mut core::fmt::Formatter<'_>) -> core::fmt::Result
+pub datumesh_http::call::Target::resolved: core::option::Option<datumesh_core::envelope::Resolved>
+pub fn datumesh_http::call::Bindings::aliases_tx(&self) -> core::option::Option<alloc::string::String>
+pub fn datumesh_http::call::Bindings::class(&self, &str, &str) -> core::result::Result<core::option::Option<alloc::rcs::arc::Arc<datumesh_http::call::BoundClass>>, datumesh_core::error::ErrorCode>
+pub fn datumesh_http::call::Bindings::lookup(&self, &str, &str, &str) -> core::result::Result<core::option::Option<alloc::string::String>, datumesh_core::error::ErrorCode>
+pub mod datumesh_http::domain
+#[non_exhaustive] pub enum datumesh_http::domain::Outcome
+pub datumesh_http::domain::Outcome::Invalid
+pub datumesh_http::domain::Outcome::Invalid::code: datumesh_core::error::ErrorCode
+pub datumesh_http::domain::Outcome::Invalid::detail: alloc::string::String
+pub datumesh_http::domain::Outcome::Skipped
+pub datumesh_http::domain::Outcome::Valid
+impl core::clone::Clone for datumesh_http::domain::Outcome
+pub fn datumesh_http::domain::Outcome::clone(&self) -> Self
+impl core::cmp::Eq for datumesh_http::domain::Outcome
+impl core::cmp::PartialEq for datumesh_http::domain::Outcome
+pub fn datumesh_http::domain::Outcome::eq(&self, &Self) -> bool
+impl core::fmt::Debug for datumesh_http::domain::Outcome
+pub fn datumesh_http::domain::Outcome::fmt(&self, &mut core::fmt::Formatter<'_>) -> core::fmt::Result
+impl core::marker::StructuralPartialEq for datumesh_http::domain::Outcome
+#[non_exhaustive] pub struct datumesh_http::domain::Validation
+pub datumesh_http::domain::Validation::failures: u32
+pub datumesh_http::domain::Validation::host: alloc::string::String
+pub datumesh_http::domain::Validation::outcome: datumesh_http::domain::Outcome
+pub datumesh_http::domain::Validation::reinstated: bool
+pub datumesh_http::domain::Validation::rotated: bool
+pub datumesh_http::domain::Validation::suspended: bool
+pub datumesh_http::domain::Validation::suspended_now: bool
+impl datumesh_http::domain::Validation
+pub fn datumesh_http::domain::Validation::failed(&self) -> bool
+pub fn datumesh_http::domain::Validation::to_json(&self) -> alloc::vec::Vec<u8>
+impl core::clone::Clone for datumesh_http::domain::Validation
+pub fn datumesh_http::domain::Validation::clone(&self) -> Self
+impl core::cmp::Eq for datumesh_http::domain::Validation
+impl core::cmp::PartialEq for datumesh_http::domain::Validation
+pub fn datumesh_http::domain::Validation::eq(&self, &Self) -> bool
+impl core::fmt::Debug for datumesh_http::domain::Validation
+pub fn datumesh_http::domain::Validation::fmt(&self, &mut core::fmt::Formatter<'_>) -> core::fmt::Result
+impl core::marker::StructuralPartialEq for datumesh_http::domain::Validation
+pub struct datumesh_http::domain::Validator
+impl datumesh_http::domain::Validator
+pub fn datumesh_http::domain::Validator::new(datumesh_http::client::Client, alloc::vec::Vec<alloc::string::String>) -> datumesh_http::domain::Validator
+pub fn datumesh_http::domain::Validator::validate(&self, &dyn datumesh_store::Store, &str, &str) -> core::result::Result<datumesh_http::domain::Validation, datumesh_store::error::Error>
+pub fn datumesh_http::domain::Validator::validate_all(&self, &dyn datumesh_store::Store, &str) -> core::result::Result<alloc::vec::Vec<datumesh_http::domain::Validation>, datumesh_store::error::Error>
+impl core::fmt::Debug for datumesh_http::domain::Validator
+pub fn datumesh_http::domain::Validator::fmt(&self, &mut core::fmt::Formatter<'_>) -> core::fmt::Result
+pub const datumesh_http::domain::SUSPEND_AFTER: u32
+pub const datumesh_http::envelope::CALL_CODES: &[datumesh_core::error::ErrorCode]
+pub fn datumesh_http::envelope::with_resolved(datumesh_http::envelope::Reply, &datumesh_core::envelope::Resolved) -> datumesh_http::envelope::Reply
+pub datumesh_http::publish::Published::classes: usize
+pub fn datumesh_http::publish::publish_stored_in<S: datumesh_store::Store + ?core::marker::Sized>(&S, &datumesh_core::sign::SigningKey, &str, &[alloc::string::String], &std::path::Path) -> core::result::Result<datumesh_http::publish::Published, datumesh_http::publish::PublishError>
+pub fn datumesh_http::Mesh::describe(&self, &str) -> datumesh_http::envelope::Reply
```
