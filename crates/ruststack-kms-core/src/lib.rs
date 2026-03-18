//! KMS core business logic for RustStack.
#![allow(
    missing_docs,
    clippy::doc_markdown,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::implicit_hasher
)]

pub mod ciphertext;
pub mod config;
pub mod crypto;
pub mod handler;
pub mod key;
pub mod provider;
pub mod resolve;
pub mod state;
pub mod validation;
