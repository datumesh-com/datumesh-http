//! The catalogue job's run for the tests (CONTRACT-classes-round.md C5), apart from `catworld.rs` so that the files that do not
//! run the job compile without it. Include with `#[path = "common/catjob.rs"] mod catjob;` after `catworld`.
#![allow(dead_code)]

use crate::catworld::LICENCE;
use crate::common::lite::*;
use crate::common::pub031::opts;

/// Runs the job over `lite`'s store at `clock` (the mesh's key, the mesh's host, the licence [`LICENCE`]).
pub fn build(lite: &Lite, clock: &'static str) -> datumesh_http::catalog_build::CatalogBuilt {
    datumesh_http::catalog_build::catalog_build_with(&*lite.store, &lite.key, HOST, LICENCE, &opts(clock))
        .unwrap_or_else(|e| panic!("catalog-build: {e}"))
}

/// The job's refusal, as text.
pub fn build_err(lite: &Lite, clock: &'static str) -> String {
    match datumesh_http::catalog_build::catalog_build_with(&*lite.store, &lite.key, HOST, LICENCE, &opts(clock)) {
        Ok(b) => panic!("catalog-build did not refuse: {b:?}"),
        Err(e) => e.to_string(),
    }
}

/// The number of cell versions a run wrote.
pub fn cells(b: &datumesh_http::catalog_build::CatalogBuilt) -> u64 {
    b.cells
}
