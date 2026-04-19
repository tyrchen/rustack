#![allow(clippy::must_use_candidate)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_async)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::explicit_iter_loop)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::needless_pass_by_value)]
//! CloudFront HTTP (restXml) layer.
//!
//! Maps the 2020-05-31 CloudFront REST API onto the core `RustackCloudFront`
//! provider. XML serialization is hand-written in `xml/` to keep the
//! dependency graph minimal and to match AWS's exact wire format (which is
//! intolerant of whitespace and element ordering drift).

pub mod dispatch;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
pub mod xml;

pub use dispatch::CloudFrontHandler;
pub use service::{CloudFrontHttpConfig, CloudFrontHttpService};
