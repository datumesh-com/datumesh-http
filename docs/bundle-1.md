# Bundle 1

An operator format (datumesh-http 0.3.0), not part of the spec. A bundle carries one namespace's
function descriptors, its name bindings and its signed receipts, with a manifest that the
namespace's host signs. `datumesh-mesh import-signed <file>` imports one; the same check and
write sequence is the body of the plan-3.1 write endpoint. The Rust API is `datumesh_http::bundle`.

## The object

A bundle is one JCS (RFC 8785) JSON object with exactly these six members:

```json
{
  "bundle": 1,
  "descriptors": [ {…canonical function descriptor…}, … ],
  "manifest": {"key_id": "<16 lowercase hex>", "signature": "<128 lowercase hex>"},
  "names": [ {"fn_hash": "<64 lowercase hex>", "name": "<ns>/<fn>"}, … ],
  "namespace": "<host>/<namespace>",
  "receipts": [ {…receipt…}, … ]
}
```

- `names` are ascending by `name`, each name under `namespace/`.
- `receipts` are ascending by their receipt hash (SPEC 10.1, the receipt's payload hash).
- A receipt is the receipt JSON as the mesh stores it (`sig` plus the payload members).

## The manifest

The manifest signs the JCS bytes of

```json
{"names": [{"fn_hash": …, "name": …}, …], "namespace": "<namespace>", "receipts": ["<receipt hash>", …]}
```

with the names and receipt hashes of the bundle itself, in those orders. The signature is Ed25519
(RFC 8032, pure, no context) with the key of the namespace's host whose `key_id` is in the manifest.
`build` signs with the 32-byte seed: its key is `SigningKey::from_seed(seed)`, and the signer is
the RFC 8032 one in `datumesh_http::bundle` (curve25519-dalek and sha2, as core uses them).
Verification is core's strict `verify_ed25519`.

## The checks, in order

The first failing check is the refusal; nothing is written unless all pass.

1. **Format.** Valid JSON; exactly the six members; `bundle` is 1; the manifest and each name have
   exactly their members; `key_id` and `signature` and every `fn_hash` are hex of the right length;
   `names` ascending, `receipts` ascending by hash.
2. **Owned.** The namespace's host (its first segment) is this mesh's host or one of its owned
   namespaces (`DATUMESH_NAMESPACES`); every name is under `namespace/`. Otherwise `NotOwned`.
3. **Manifest.** The store holds the namespace host's key document; the manifest's `key_id` is in
   it; that key holds now (`valid_from <= now`, and `now < valid_to` when present); and the
   signature verifies over the bundle's names and receipt hashes. Otherwise `Manifest`.
4. **Descriptor.** Each descriptor is a canonical function descriptor, and each name's `fn_hash` is
   one of the bundle's descriptors or already stored. Otherwise `Descriptor`.
5. **Receipt.** Each receipt verifies (SPEC 10.1 steps 1 to 6) against the host's key document; has
   no `inputs` (computed receipts are not imported); its cell's function (`cell.ns/cell.fn`) is one
   of the names, with that name's `fn_hash`. Otherwise `Receipt`.
6. **TxInFuture.** No receipt's `tx` is later than now.
7. **Bind and version conflicts.** A name whose latest binding is at or after now is `Bind`. A cell
   with a version at or after a new receipt's `tx`, other than that receipt, is `Store`. A name
   already bound to its hash, and a version already there, are skipped (not counted).

## The writes, and why in this order

1. `put_descriptor` for each descriptor.
2. `put_receipt` for each receipt, then `put_version` for each new version.
3. `bind_name` for each name not already bound to its hash, at now. Last.

A name answers only through its binding, so a version without its binding is not answered. Writing
the bindings last means a crash part-way leaves nothing answerable. The partial state is
harmless, and a re-run of the same bundle completes it: a version already there is skipped, so
nothing gets a second version, and a binding already to that hash is skipped, so nothing gets a
second binding. The result is reported as `Imported` (`functions_bound`, `receipts`, `versions`,
`bound_at`), printed as one JCS line by `import-signed`.

The test hook `datumesh_http::bundle::import_signed_with` (`#[doc(hidden)]`) stops after the n-th
write, to test that re-run.

## Errors

`BundleError` (`#[non_exhaustive]`): `Format`, `NotOwned`, `Manifest`, `Descriptor`, `Receipt`,
`TxInFuture { receipt, tx, now }`, `Bind`, `Store`, and `Stopped(n)` (the test hook only). Each
displays as one line; `import-signed` prints that line on stderr and exits 1.
