//! `GET /manifest/<slice_id>/<members_hash>` (SPEC 15.13; CONTRACT-guard-patch G4, G5, G10).
//!
//! A held manifest is served as it is. Otherwise the members are rebuilt, with nothing stored:
//! from the member-set row (its `asof`), or, with `?asof=`, from the slice's definition as of that
//! instant (a past read's receipt needs no row). The rebuild checks the hash of the members
//! (`Engine::manifest_asof`), so a false `asof` or a member that cannot be listed is refused, never
//! guessed: `503 E_INPUT_UNAVAILABLE`, or `503 E_EVAL_BUDGET` over the fuel.
//!
//! The full form is one response. The paged form (`?after=<member hash>&limit=<n>`, `after`
//! exclusive, `limit` 1..=10,000, default 1,000) answers a page and a `Link` to the next.

use datumesh_core::json::Value;
use datumesh_core::{lexical, ErrorCode, Manifest};
use datumesh_engine::EngineError;

use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;

/// The query of the route. Every value is text here, so a malformed one is a `400`, not a
/// rejected extraction.
#[derive(serde::Deserialize, Default)]
pub(crate) struct ManifestQuery {
    pub after: Option<String>,
    pub limit: Option<String>,
    pub asof: Option<String>,
}

/// The default page of the paged form.
pub(crate) const DEFAULT_LIMIT: usize = 1_000;
/// The most members a page holds.
pub(crate) const MAX_LIMIT: usize = 10_000;

/// The reply of the route, and the path of the next page when there is one (a `Link` header).
pub(crate) fn reply(inner: &Inner, slice_id: &str, members_hash: &str, q: &ManifestQuery) -> (Reply, Option<String>) {
    let entry = format!("slice:{slice_id}:{members_hash}");
    let paged = q.after.is_some() || q.limit.is_some();
    let limit = match q.limit.as_deref() {
        None => DEFAULT_LIMIT,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => return (envelope::bad_request_reply(ErrorCode::Schema), None),
        },
    };
    if let Some(a) = &q.after {
        if lexical::check_hex(a, 64, "$").is_err() {
            return (envelope::bad_request_reply(ErrorCode::Schema), None);
        }
    }
    if let Some(a) = &q.asof {
        if lexical::check_instant(a, "$").is_err() {
            return (envelope::bad_request_reply(ErrorCode::Schema), None);
        }
    }
    let bytes = match held_or_rebuilt(inner, slice_id, &entry, q) {
        Ok(b) => b,
        Err(r) => return (r, None),
    };
    if !paged {
        return (Reply::json(200, bytes, Cache::Immutable), None);
    }
    let m = match Manifest::from_json(&bytes) {
        Ok(m) => m,
        Err(_) => return (envelope::error_reply(ErrorCode::InputUnavailable), None),
    };
    let all = m.members();
    let start = match &q.after {
        Some(a) => all.partition_point(|x| x.as_str() <= a.as_str()),
        None => 0,
    };
    let end = (start + limit).min(all.len());
    let page = &all[start..end];
    let body = Value::Object(vec![
        ("entry".into(), Value::String(entry.clone())),
        ("members".into(), Value::Array(page.iter().map(|x| Value::String(x.clone())).collect())),
    ]);
    let next = (end < all.len()).then(|| {
        let last = page.last().cloned().unwrap_or_default();
        let mut url = format!("/manifest/{slice_id}/{members_hash}?after={last}&limit={limit}");
        if let Some(a) = &q.asof {
            url.push_str(&format!("&asof={a}"));
        }
        url
    });
    (Reply::json(200, body.to_jcs(), Cache::Immutable), next)
}

/// The canonical bytes of the manifest of `entry`: held, else rebuilt. A `404` (no reply body
/// beyond the usual) when neither the bytes nor the means to rebuild them are held.
fn held_or_rebuilt(inner: &Inner, slice_id: &str, entry: &str, q: &ManifestQuery) -> Result<Vec<u8>, Reply> {
    let store = &*inner.store;
    if let Some(b) = store.manifest(entry) {
        return Ok(b.into_owned());
    }
    let asof = match &q.asof {
        Some(a) => a.clone(),
        None => match store.member_set(entry) {
            Ok(Some(row)) => row.asof,
            Ok(None) => return Err(not_found()),
            Err(_) => return Err(envelope::error_reply(ErrorCode::InputUnavailable)),
        },
    };
    // A slice the mesh holds neither a definition nor a row of is refused at once, no rebuild
    // counted (G10). Otherwise the engine takes the slice from the definition or the row.
    match store.slice_definition(slice_id) {
        Ok(Some(_)) => {}
        Ok(None) => match store.member_set(entry) {
            Ok(Some(_)) => {}
            Ok(None) => return Err(not_found()),
            Err(_) => return Err(envelope::error_reply(ErrorCode::InputUnavailable)),
        },
        Err(_) => return Err(envelope::error_reply(ErrorCode::InputUnavailable)),
    }
    inner.metrics.manifest_rebuild();
    let deadline = std::time::Instant::now() + inner.config.budget;
    match inner.engine.manifest_asof_until(entry, &asof, deadline) {
        Ok(Some(b)) => Ok(b),
        Ok(None) => Err(not_found()),
        Err(e) => {
            inner.metrics.manifest_rebuild_failure();
            tracing::warn!(entry, asof = asof.as_str(), "manifest rebuild refused: {e}");
            Err(refusal(&e))
        }
    }
}

fn not_found() -> Reply {
    Reply::json(404, br#"{"error":"no such manifest"}"#.to_vec(), Cache::Public(60))
}

/// `503`: `E_EVAL_BUDGET` over the fuel, else `E_INPUT_UNAVAILABLE` (a hash that does not match,
/// a member that cannot be listed, a store that failed). Never cached.
fn refusal(e: &EngineError) -> Reply {
    match e {
        EngineError::BudgetExceeded { .. } | EngineError::WorkExceeded { .. } => {
            Reply::json(503, br#"{"error":"E_EVAL_BUDGET","outcome":"error"}"#.to_vec(), Cache::NoStore)
        }
        _ => envelope::error_reply(ErrorCode::InputUnavailable),
    }
}

/// The members of a slice entry for a reader that needs them (G6: `keys_for_receipt`,
/// `foreign_deps`): the held manifest, else the rebuild as of the member-set row's `asof`, or as of
/// `asof` (the receipt's own, for a past read with no row). Counted as a rebuild when `metrics` is
/// given; within `deadline` (the time budget); nothing is stored. `None` when neither gives the members.
pub(crate) fn members_for_reader(
    store: &(dyn datumesh_store::Store + Send + Sync),
    engine: &datumesh_engine::Engine,
    metrics: Option<&crate::metrics::Metrics>,
    entry: &str,
    asof: Option<&str>,
    deadline: std::time::Instant,
) -> Option<Manifest> {
    if let Some(b) = store.manifest(entry) {
        return Manifest::from_json(&b).ok();
    }
    let at = match store.member_set(entry).ok().flatten() {
        Some(row) => row.asof,
        None => asof?.to_string(),
    };
    if let Some(m) = metrics {
        m.manifest_rebuild();
    }
    match engine.manifest_asof_until(entry, &at, deadline) {
        Ok(Some(b)) => Manifest::from_json(&b).ok(),
        Ok(None) => None,
        Err(e) => {
            if let Some(m) = metrics {
                m.manifest_rebuild_failure();
            }
            tracing::warn!(entry, asof = at.as_str(), "manifest rebuild refused: {e}");
            None
        }
    }
}

/// The member count and the slice of a slice entry, for `/explain` (G6): from its member-set row,
/// else from a held manifest. Nothing is rebuilt.
pub(crate) fn count_and_slice(inner: &Inner, entry: &str) -> Option<(u64, Value)> {
    if let Ok(Some(row)) = inner.store.member_set(entry) {
        return Some((row.members, datumesh_core::json::parse(&row.slice).ok()?));
    }
    let m = inner.store.manifest(entry).and_then(|b| Manifest::from_json(&b).ok())?;
    Some((m.members().len() as u64, m.slice().to_value()))
}
