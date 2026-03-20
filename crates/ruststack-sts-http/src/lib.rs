//! STS HTTP service layer for RustStack (awsQuery protocol).
//!
//! STS uses the `awsQuery` protocol, identical wire format to SNS.
//! Requests are `application/x-www-form-urlencoded` with `Action=<op>` dispatch.
//! Responses are `text/xml`. Gateway routing uses SigV4 `service=sts`.

pub mod body;
pub mod dispatch;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
