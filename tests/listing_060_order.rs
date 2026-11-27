//! http 0.6.0, the listing release (CONTRACT-http-listing.md L5, *[25 Nov]*): find orders results that share a subject by the
//! catalogue rank, and the order is a true order. Unit tests on `datumesh_http::find_order`; failing on the test commit's
//! stubs (`order`, `sort_keys` and `cmp_sort_keys` are `todo!()`).
//!
//! 1. Group: results that share a subject form one group; a result in no subject is a group of its own.
//! 2. Place the groups: each group takes the place its best member has by today's key (match type, score descending, kind,
//!    name); groups are sorted by that key.
//! 3. Inside a group: by catalogue rank (1 first); equal ranks by today's key.

use std::cmp::Ordering;

use datumesh_http::find_order::{cmp_sort_keys, order, sort_keys, today_cmp, FindKey, SubjectRank};

const S: &str =
    r#"mesh-test.datumesh.com/catalog/subject(domain:"finance",measure:"interest_rate",region:"gb",thing:"rate")"#;
const T: &str =
    r#"mesh-test.datumesh.com/catalog/subject(domain:"finance",measure:"interest_rate",region:"ea",thing:"rate")"#;

fn key(name: &str, match_type: u8, score: f64, subject: Option<(&str, u32)>) -> FindKey {
    FindKey {
        match_type,
        score,
        kind: 2,
        name: name.to_string(),
        subject: subject.map(|(s, rank)| SubjectRank { subject: s.to_string(), rank }),
    }
}

fn names(results: &[FindKey], idx: &[usize]) -> Vec<String> {
    idx.iter().map(|&i| results[i].name.clone()).collect()
}

/// Every permutation of 0..n (n small).
fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![vec![]];
    }
    let mut out = Vec::new();
    for p in permutations(n - 1) {
        for i in 0..=p.len() {
            let mut q = p.clone();
            q.insert(i, n - 1);
            out.push(q);
        }
    }
    out
}

/// The three results of the contract that loop under the draft's rule: A and B in subject S (A rank 2, score 0.9; B rank
/// 1, score 0.5), C in no subject (0.7). The group {A, B} takes A's place: B, A, C. From every input order.
#[test]
fn l5_the_three_results_that_loop_under_the_draft_give_b_a_c() {
    let base = [key("A", 1, 0.9, Some((S, 2))), key("B", 1, 0.5, Some((S, 1))), key("C", 1, 0.7, None)];
    for p in permutations(3) {
        let input: Vec<FindKey> = p.iter().map(|&i| base[i].clone()).collect();
        assert_eq!(names(&input, &order(&input)), ["B", "A", "C"], "input order {p:?}");
    }
}

/// A group spanning two match types takes its best member's place: A exact (rank 2) and B vector (rank 1) in S place the
/// group where A is, before the text result C; inside it B (rank 1) comes first. D, vector and in no subject, stays after
/// C by today's key.
#[test]
fn l5_a_group_spanning_two_match_types_takes_its_best_members_place() {
    let base = [
        key("A", 0, 0.3, Some((S, 2))),
        key("B", 2, 0.99, Some((S, 1))),
        key("C", 1, 1.0, None),
        key("D", 2, 0.95, None),
    ];
    for p in permutations(4) {
        let input: Vec<FindKey> = p.iter().map(|&i| base[i].clone()).collect();
        assert_eq!(names(&input, &order(&input)), ["B", "A", "C", "D"], "input order {p:?}");
    }
}

/// Two groups and loose results: each group stays together at its best member's place; equal ranks inside a group are by
/// today's key.
#[test]
fn l5_groups_stay_together_and_equal_ranks_go_by_todays_key() {
    let input = vec![
        key("s_low", 1, 0.2, Some((S, 3))),
        key("t_one", 1, 0.4, Some((T, 1))),
        key("loose_high", 1, 0.95, None),
        key("s_best", 1, 0.9, Some((S, 2))),
        key("t_two_a", 1, 0.6, Some((T, 2))),
        key("t_two_b", 1, 0.8, Some((T, 2))),
        key("loose_mid", 1, 0.7, None),
        key("s_one", 2, 0.1, Some((S, 1))),
    ];
    // groups: S best is s_best (text 0.9); T best is t_two_b (text 0.8); loose_high 0.95, loose_mid 0.7.
    assert_eq!(
        names(&input, &order(&input)),
        ["loose_high", "s_one", "s_best", "s_low", "t_one", "t_two_b", "t_two_a", "loose_mid"]
    );
}

/// With no subjects at all, the order is exactly today's.
#[test]
fn l5_with_no_subjects_the_order_is_todays() {
    let mut rng = Rng(0x5eed_0001);
    for round in 0..200 {
        let n = 1 + (rng.next() % 12) as usize;
        let input: Vec<FindKey> = (0..n).map(|i| random_key(&mut rng, i, false)).collect();
        let mut want: Vec<usize> = (0..n).collect();
        want.sort_by(|&a, &b| today_cmp(&input[a], &input[b]));
        assert_eq!(order(&input), want, "round {round}: {input:?}");
    }
}

/// The comparator is a total order on an answer's results: over random answers (ties in score, match type and rank
/// included), `cmp_sort_keys` is reflexive, antisymmetric, transitive and total (Equal only for the same result); `order` is
/// the sort by it; the reference model of the three steps gives the same sequence; and a permutation of the input gives
/// the same names.
#[test]
fn l5_the_comparator_is_a_total_order_property() {
    let mut rng = Rng(0x5eed_0002);
    for round in 0..300 {
        let n = 1 + (rng.next() % 10) as usize;
        let input: Vec<FindKey> = (0..n).map(|i| random_key(&mut rng, i, true)).collect();
        let keys = sort_keys(&input);
        assert_eq!(keys.len(), n);
        for a in 0..n {
            assert_eq!(cmp_sort_keys(&keys[a], &keys[a]), Ordering::Equal, "reflexive");
            for b in 0..n {
                let ab = cmp_sort_keys(&keys[a], &keys[b]);
                assert_eq!(ab, cmp_sort_keys(&keys[b], &keys[a]).reverse(), "antisymmetric: round {round}, {a} {b}");
                if a != b {
                    assert_ne!(ab, Ordering::Equal, "total: two results never tie (round {round}, {a} {b}): {input:?}");
                }
                for c in 0..n {
                    let bc = cmp_sort_keys(&keys[b], &keys[c]);
                    if ab != Ordering::Greater && bc != Ordering::Greater {
                        assert_ne!(
                            cmp_sort_keys(&keys[a], &keys[c]),
                            Ordering::Greater,
                            "transitive: round {round}, {a} <= {b} <= {c}: {input:?}"
                        );
                    }
                }
            }
        }
        let mut by_keys: Vec<usize> = (0..n).collect();
        by_keys.sort_by(|&a, &b| cmp_sort_keys(&keys[a], &keys[b]));
        let got = order(&input);
        assert_eq!(got, by_keys, "order is the sort by the keys: round {round}");
        assert_eq!(names(&input, &got), names(&input, &reference(&input)), "the three steps: round {round}: {input:?}");
        let mut shuffled = input.clone();
        rng.shuffle(&mut shuffled);
        assert_eq!(names(&shuffled, &order(&shuffled)), names(&input, &got), "permutation: round {round}");
    }
}

/// The three steps, written out plainly (the test's reference model).
fn reference(input: &[FindKey]) -> Vec<usize> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_subject: std::collections::BTreeMap<String, usize> = Default::default();
    for (i, k) in input.iter().enumerate() {
        match &k.subject {
            Some(s) => {
                let g = *by_subject.entry(s.subject.clone()).or_insert_with(|| {
                    groups.push(Vec::new());
                    groups.len() - 1
                });
                groups[g].push(i);
            }
            None => groups.push(vec![i]),
        }
    }
    for g in &mut groups {
        g.sort_by(|&a, &b| {
            let ra = input[a].subject.as_ref().map(|s| s.rank).unwrap_or(0);
            let rb = input[b].subject.as_ref().map(|s| s.rank).unwrap_or(0);
            ra.cmp(&rb).then(today_cmp(&input[a], &input[b]))
        });
    }
    let best = |g: &Vec<usize>| *g.iter().min_by(|&&a, &&b| today_cmp(&input[a], &input[b])).unwrap();
    groups.sort_by(|a, b| today_cmp(&input[best(a)], &input[best(b)]));
    groups.into_iter().flatten().collect()
}

fn random_key(rng: &mut Rng, i: usize, subjects: bool) -> FindKey {
    let scores = [0.25, 0.5, 0.5, 0.75, 0.9, 1.0];
    let subject = if subjects && !rng.next().is_multiple_of(3) {
        let s = ["S1", "S2", "S3"][(rng.next() % 3) as usize];
        Some((s, 1 + (rng.next() % 3) as u32))
    } else {
        None
    };
    FindKey {
        match_type: (rng.next() % 3) as u8,
        score: scores[(rng.next() % scores.len() as u64) as usize],
        kind: (rng.next() % 3) as u8,
        name: format!("n{:02}_{}", rng.next() % 50, i),
        subject: subject.map(|(s, rank)| SubjectRank { subject: s.to_string(), rank }),
    }
}

/// xorshift64: a fixed sequence, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}
