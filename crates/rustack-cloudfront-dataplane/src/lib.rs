#![allow(clippy::must_use_candidate)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_async)]
#![allow(clippy::explicit_iter_loop)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::fn_params_excessive_bools)]
//! CloudFront data plane.
//!
//! The data plane resolves distributions, dispatches to origins, and keeps a
//! lightweight in-memory response cache for cacheable GET/HEAD requests. See
//! `specs/rustack-cloudfront-dataplane-design.md` for the full design.

pub mod behavior;
pub mod cache;
pub mod config;
pub mod dispatch;
pub mod divergence;
pub mod error;
pub mod host;
pub mod plane;
pub mod transform;

pub use cache::{CacheSnapshotError, CloudFrontCacheSnapshot};
pub use config::DataPlaneConfig;
pub use plane::{DataPlane, DataPlaneBuilder};
