#![allow(clippy::must_use_candidate)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_async)]
#![allow(clippy::explicit_iter_loop)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::fn_params_excessive_bools)]
//! Pass-through CloudFront data plane.
//!
//! The data plane does not cache. Every request hits origin. Its purpose is
//! end-to-end IaC validation, not CDN simulation. See
//! `specs/rustack-cloudfront-dataplane-design.md` for the full design.

pub mod behavior;
pub mod config;
pub mod dispatch;
pub mod divergence;
pub mod error;
pub mod host;
pub mod plane;
pub mod transform;

pub use config::DataPlaneConfig;
pub use plane::{DataPlane, DataPlaneBuilder};
