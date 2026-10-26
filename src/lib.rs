//! The [Datumesh](https://datumesh.com) engine behind HTTPS.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod apikeys;
pub mod call;
pub mod config;
pub mod envelope;
pub mod foreign;
mod http;
mod jobs;
pub mod keydoc;
mod limit;
mod mesh;
mod metrics;
mod pool;
pub mod slice;

pub use config::Config;
pub use http::openapi;
pub use mesh::{BuildError, Mesh, MeshBuilder, Verified};
