//! The forms of an instant a request may carry (CONTRACT-protocol-part1 H3; DECISIONS D7 and D7.1).
//!
//! Wherever a request takes an instant, these are accepted and canonicalised to six fractional digits before anything
//! else reads them: `YYYY-MM-DDTHH:MM:SS` with 0 to 6 fractional digits and a `Z`, and (for an as-of) a date
//! `YYYY-MM-DD`, meaning the end of that day in UTC, `YYYY-MM-DDT23:59:59.999999Z`. An offset other than `Z` is not
//! one of these forms: it is left as it is, so core refuses it with `E_CANON_INSTANT`. Stored and signed values never
//! change form; only what a request names is read through this helper, and the answer echoes the canonical instant.

use datumesh_core::lexical;

/// The canonical form of `s` when it is one of the accepted forms (`date` allows a bare date, which means the end of
/// its day), `None` otherwise.
pub(crate) fn canonical(s: &str, date: bool) -> Option<String> {
    if date && s.len() == 10 && lexical::check_date(s, "$").is_ok() {
        return Some(format!("{s}T23:59:59.999999Z"));
    }
    let body = s.strip_suffix('Z')?;
    let (base, frac) = match body.find('.') {
        Some(i) if i + 1 < body.len() => (&body[..i], &body[i + 1..]),
        Some(_) => return None,
        None => (body, ""),
    };
    if frac.len() > 6 || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let out = format!("{base}.{frac:0<6}Z");
    lexical::check_instant(&out, "$").ok()?;
    Some(out)
}

/// `s` canonicalised when it has an accepted form, else `s` as it is (so the caller's own check refuses it).
pub(crate) fn canonical_or_same(s: &str, date: bool) -> String {
    canonical(s, date).unwrap_or_else(|| s.to_string())
}

/// A URI's `<rest>` (query included) with every `asof=` parameter canonicalised, as a bare date too.
pub(crate) fn in_query(rest: &str) -> String {
    let Some((path, query)) = rest.split_once('?') else {
        return rest.to_string();
    };
    let ps: Vec<String> = query
        .split('&')
        .map(|p| match p.strip_prefix("asof=") {
            Some(v) => format!("asof={}", canonical_or_same(v, true)),
            None => p.to_string(),
        })
        .collect();
    format!("{path}?{}", ps.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(canonical("2026-05-10T12:00:00Z", false).as_deref(), Some("2026-05-10T12:00:00.000000Z"));
        assert_eq!(canonical("2026-05-10T12:00:00.5Z", false).as_deref(), Some("2026-05-10T12:00:00.500000Z"));
        assert_eq!(canonical("2026-05-10T12:00:00.123456Z", false).as_deref(), Some("2026-05-10T12:00:00.123456Z"));
        assert_eq!(canonical("2026-05-10", true).as_deref(), Some("2026-05-10T23:59:59.999999Z"));
        assert_eq!(canonical("2026-05-10", false), None);
        assert_eq!(canonical("2026-05-10T12:00:00.5+01:00", false), None);
        assert_eq!(canonical("2026-05-10T12:00:00.1234567Z", false), None);
        assert_eq!(canonical("2026-05-10T12:00:00.Z", false), None);
        assert_eq!(canonical("2026-05-10T12:00Z", false), None);
        assert_eq!(canonical("2026-05-1", true), None);
        assert_eq!(in_query("a/b?x=1&asof=2026-05-10"), "a/b?x=1&asof=2026-05-10T23:59:59.999999Z");
    }
}
