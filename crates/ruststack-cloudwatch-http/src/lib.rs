//! CloudWatch Metrics HTTP service layer for RustStack (awsQuery protocol).
//!
//! CloudWatch Metrics uses the `awsQuery` protocol (form-urlencoded requests,
//! XML responses), the same wire format as SNS. Requests are `POST /` with
//! `Content-Type: application/x-www-form-urlencoded` and the operation is
//! dispatched via the `Action=<op>` form parameter.

pub mod body;
pub mod dispatch;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
