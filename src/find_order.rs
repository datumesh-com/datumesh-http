//! The order of `/find` results (CONTRACT-http-listing L5, *[25 Nov]*): a true order.
//!
//! [`today_cmp`] is today's key (what `find.rs` sorts by in 0.5.x); [`order`] is the sort by [`sort_keys`] and
//! [`cmp_sort_keys`].
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
    /// The best member of the result's group, by today's key: the group's place.
    place: FindKey,
    /// The result's catalogue rank inside its group (0 for a result in no subject).
    rank: u32,
    /// The result itself, by today's key (the tie-break inside a group).
    own: FindKey,
}

/// Each result's key in the grouped order of the answer `results` (in the same order as `results`). Comparing two keys
/// with [`cmp_sort_keys`] is a total order on the answer's results.
pub fn sort_keys(results: &[FindKey]) -> Vec<SortKey> {
    // The group of a result: its subject's key encoding, or its own name when it is in no subject (names are unique).
    let group_of = |k: &FindKey| -> String {
        match &k.subject {
            Some(s) => format!("s:{}", s.subject),
            None => format!("n:{}", k.name),
        }
    };
    let mut best: std::collections::HashMap<String, FindKey> = Default::default();
    for k in results {
        let g = group_of(k);
        match best.get(&g) {
            Some(b) if today_cmp(b, k) != Ordering::Greater => {}
            _ => {
                best.insert(g, k.clone());
            }
        }
    }
    results
        .iter()
        .map(|k| SortKey {
            place: best[&group_of(k)].clone(),
            rank: k.subject.as_ref().map(|s| s.rank).unwrap_or(0),
            own: k.clone(),
        })
        .collect()
}

/// The comparison of two keys of one answer: the groups by their places (today's key of the best members), then inside a
/// group by catalogue rank, then by today's key. Two results share a place exactly when they share a group, and names are
/// unique, so the comparison is total.
pub fn cmp_sort_keys(a: &SortKey, b: &SortKey) -> Ordering {
    if a.place.name == b.place.name {
        a.rank.cmp(&b.rank).then_with(|| today_cmp(&a.own, &b.own))
    } else {
        today_cmp(&a.place, &b.place)
    }
}

/// The indices of `results` in the order of `/find` (steps 1 to 3). The same multiset of results in any input order
/// gives the same sequence of names.
pub fn order(results: &[FindKey]) -> Vec<usize> {
    let keys = sort_keys(results);
    let mut idx: Vec<usize> = (0..results.len()).collect();
    idx.sort_by(|&a, &b| cmp_sort_keys(&keys[a], &keys[b]));
    idx
}
