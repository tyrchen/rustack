//! CloudWatch Metrics HTTP service layer for RustStack.
//!
//! Supports two wire protocols:
//! - **awsQuery**: form-urlencoded requests, XML responses (legacy SDKs).
//!   Requests are `POST /` with `Content-Type: application/x-www-form-urlencoded`
//!   and the operation is dispatched via the `Action=<op>` form parameter.
//! - **rpcv2Cbor**: CBOR requests/responses (AWS SDK v1.108+).
//!   Requests are `POST /service/GraniteServiceVersion20100801/operation/{Op}`
//!   with `Content-Type: application/cbor` and `smithy-protocol: rpc-v2-cbor`.

pub mod body;
pub mod dispatch;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
