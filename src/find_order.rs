//! The order of `/find` results (CONTRACT-http-listing L5, *[25 Nov]*): a true order.
//!
//! **Stub of the test commit.** [`today_cmp`] is today's key (it is what `find.rs` sorts by in 0.5.x); [`order`],
//! [`sort_keys`] and [`cmp_sort_keys`] are `todo!()`. The implementer writes them and makes `find.rs` sort by them.
//!
//! 1. **Group.** Results that share a subject form one group; a result in no subject is a group of its own.
//! 2. **Place the groups.** Each group takes the place its best member has by today's key (match type, then score
//!    descending, then kind, then name); groups are sorted by that key.
//! 3. **Order inside a group** by catalogue rank (1 first); equal ranks by today's key.
//!
//! A result's subject, for the order, is the one subject (of the catalogue job's `source(k)` cells) in which it has its
//! lowest rank; on a tie, the subject whose key encoding is first in byte order. With no subjects at all, the order is
//! exactly today's.

use std::cmp::Ordering;

/// A result's place in a subject: the subject's key encoding (`<host>/catalog/subject(domain:"..",...)`) and the result's
/// catalogue rank in it (1 first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectRank {
    /// The subject's key encoding.
    pub subject: String,
    /// The catalogue rank, from 1.
    pub rank: u32,
}

/// What the order knows of one result. Names are unique within one answer.
#[derive(Clone, Debug, PartialEq)]
pub struct FindKey {
    /// 0 exact, 1 text, 2 vector.
    pub match_type: u8,
    /// The score (higher first); finite.
    pub score: f64,
    /// 0 class, 1 method, 2 function.
    pub kind: u8,
    /// The full name.
    pub name: String,
    /// The result's subject and rank, when it is in one.
    pub subject: Option<SubjectRank>,
}

/// Today's key (0.5.x): match type, then score descending, then kind, then name.
pub fn today_cmp(a: &FindKey, b: &FindKey) -> Ordering {
    a.match_type
        .cmp(&b.match_type)
        .then(b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
        .then(a.kind.cmp(&b.kind))
        .then(a.name.cmp(&b.name))
}

/// The key of one result in the grouped order, given the whole answer (see [`sort_keys`]). Opaque.
#[derive(Clone, Debug, PartialEq)]
pub struct SortKey {
    // Stub: the implementer replaces this field with the key's parts (the group's place, the rank, today's key).
    _todo: (),
}

/// Each result's key in the grouped order of the answer `results` (in the same order as `results`). Comparing two keys
/// with [`cmp_sort_keys`] is a total order on the answer's results.
pub fn sort_keys(results: &[FindKey]) -> Vec<SortKey> {
    let _ = results;
    todo!("listing 0.6.0: find_order::sort_keys")
}

/// The comparison of two keys of one answer.
pub fn cmp_sort_keys(a: &SortKey, b: &SortKey) -> Ordering {
    let _ = (a, b);
    todo!("listing 0.6.0: find_order::cmp_sort_keys")
}

/// The indices of `results` in the order of `/find` (steps 1 to 3). The same multiset of results in any input order
/// gives the same sequence of names.
pub fn order(results: &[FindKey]) -> Vec<usize> {
    let _ = results;
    todo!("listing 0.6.0: find_order::order")
}
