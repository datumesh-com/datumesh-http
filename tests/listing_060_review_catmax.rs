//! http 0.6.0, the review round, B4 as decided 27 Nov: under `DATUMESH_CATALOG_MAX`, subjects that hold only listed names
//! are cut first; they never push out a subject of this mesh's own names. In a file of its own: the job reads the variable
//! from the process environment.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use catjob::*;
use catworld::*;
use datumesh_store::SubjectKeys;
use listing060::*;

/// Three listed-only subjects whose keys sort before the relay's (domain `aaa` before `finance`), the cap at 2: the relay's
/// subject stays, and one listed-only subject fills the other place.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b4_listed_only_subjects_are_cut_first_under_the_catalogue_cap() {
    let lite = listing_lite("r060-catmax");
    for i in 0..3 {
        put_listing(
            &lite,
            SRC,
            &format!("{SRC}/aaa_{i}"),
            vec![SubjectKeys::new("aaa", "rate", "interest_rate", &format!("r{i}"))],
            RUN1,
        );
    }
    std::env::set_var("DATUMESH_CATALOG_MAX", "2");
    build(&lite, T3);
    std::env::remove_var("DATUMESH_CATALOG_MAX");
    let m = serve(lite, &kept(SRC)).await;
    assert_eq!(sources_at(&m, GB).await, [relay()], "the subject of this mesh's own name stays");
    let mut kept_listed = 0;
    for i in 0..3 {
        let a = source_at(&m, ("aaa", "rate", "interest_rate", &format!("r{i}")), 1).await;
        if a.outcome() == "ok" {
            kept_listed += 1;
        }
    }
    assert_eq!(kept_listed, 1, "one listed-only subject fills the second place");
}
