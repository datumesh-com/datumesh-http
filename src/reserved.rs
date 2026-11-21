//! Reserved paths under the mesh's own host (CONTRACT-0.4.9 H8, decision D43).
//!
//! The mesh's own paths (`<host>/series/…`, `<host>/fn/…`, `<host>/takeover/…`, `<host>/x/…`) are not names: a
//! name whose first segment after the host is one of them is refused at every write path. A name under
//! `<host>/key/…` is accepted only as a key-derived name (`<host>/key/k<24 hex>/…`, the 0.4.8 rule) from the
//! registered key it is derived from; the mesh's own key does not publish there, and any other `key/…` name is
//! reserved. Names under another host are not reserved: they are rooted at their own domain.

/// The first segments after the host that no name may take.
const RESERVED: [&str; 4] = ["series", "fn", "takeover", "x"];

/// The code of a refused name.
pub(crate) const CODE: &str = "E_NAME_RESERVED";

/// Checks `name` against the reserved paths under `host`. `signer_ns` is the key namespace the writing key may
/// publish under (the namespace derived from a registered key that signs the write), or `None` for a write by
/// the mesh itself or its operator. The error text starts with the code.
pub(crate) fn check(host: &str, name: &str, signer_ns: Option<&str>) -> Result<(), String> {
    let Some(rest) = name.strip_prefix(host).and_then(|r| r.strip_prefix('/')) else { return Ok(()) };
    let first = rest.split('/').next().unwrap_or("");
    if RESERVED.contains(&first) {
        return Err(format!("{CODE}: {name} is under {host}/{first}, a path the mesh reserves"));
    }
    if first == "key" {
        let derived = signer_ns.is_some_and(|ns| name.strip_prefix(ns).is_some_and(|r| r.starts_with('/')));
        // A well-formed key namespace written by a registered key that is not the one it derives from is not
        // reserved: it is refused as not owned (`E_NOT_OWNED`, the key-names rule, 0.4.8).
        let well_formed = rest.split('/').nth(1).is_some_and(is_key_segment);
        let registered_other = signer_ns.is_some() && well_formed;
        if !derived && !registered_other {
            return Err(format!(
                "{CODE}: {name} is under {host}/key, which takes only the names of the key it is derived from"
            ));
        }
    }
    Ok(())
}

/// `k` and 24 lower-case hex digits: the segment of a key namespace.
fn is_key_segment(seg: &str) -> bool {
    seg.len() == 25
        && seg.starts_with('k')
        && seg[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "mesh-test.datumesh.com";

    #[test]
    fn the_mesh_paths_are_reserved_and_their_look_alikes_are_not() {
        for n in ["series/a", "fn/a", "takeover/a", "x/a", "series/deep/er/a"] {
            assert!(check(H, &format!("{H}/{n}"), None).is_err(), "{n}");
        }
        for n in ["seriesx/a", "series_a/b", "fnord/a", "h049/series/a", "series_a"] {
            assert!(check(H, &format!("{H}/{n}"), None).is_ok(), "{n}");
        }
        // A name of another host is never reserved, whatever its first segment (the mesh host is H).
        assert!(check(H, "other.example/series/a", None).is_ok());
    }

    #[test]
    fn a_key_name_is_the_signers_own_only() {
        let ns = format!("{H}/key/k0123456789abcdef01234567");
        assert!(check(H, &format!("{ns}/f"), Some(&ns)).is_ok());
        assert!(check(H, &format!("{ns}/f"), None).is_err());
        assert!(check(H, &format!("{H}/key/foo"), Some(&ns)).is_err());
        assert!(check(H, &format!("{H}/key/k0123456789abcdef01234567x/f"), Some(&ns)).is_err());
    }
}
