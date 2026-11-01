//! Bounded complete encoding, exact operation replies and static error envelopes.

use std::io::{self, Write};

use bytes::Bytes;
use http::{
    HeaderValue, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE as TYPE_HEADER},
};
use serde::Serialize;
use uuid::Uuid;

use crate::{CONTENT_TYPE, HttpError, StepFunctionsBody, transport::Response};

const LIMIT: usize = 8 * 1024 * 1024;
struct Writer {
    bytes: Vec<u8>,
    failed: bool,
}
impl Write for Writer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let Some(length) = self
            .bytes
            .len()
            .checked_add(data.len())
            .filter(|length| *length <= LIMIT)
        else {
            self.failed = true;
            return Err(io::Error::other("response byte limit exceeded"));
        };
        let capacity = self.bytes.capacity();
        let target = length.max(capacity.saturating_mul(2).max(4096)).min(LIMIT);
        if length > capacity
            && self
                .bytes
                .try_reserve_exact(target - self.bytes.len())
                .is_err()
        {
            self.failed = true;
            return Err(io::Error::other("response allocation failed"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(
    write: impl FnOnce(&mut Writer) -> Result<(), serde_json::Error>,
) -> Result<Bytes, HttpError> {
    let mut writer = Writer {
        bytes: Vec::new(),
        failed: false,
    };
    if let Err(error) = write(&mut writer) {
        return Err(if writer.failed {
            HttpError::Response
        } else {
            HttpError::Encoding(error)
        });
    }
    Ok(Bytes::from(writer.bytes))
}
pub(crate) fn success(
    value: &Response,
    id: Uuid,
) -> Result<http::Response<StepFunctionsBody>, HttpError> {
    let bytes = match value {
        Response::SendTaskSuccess(_)
        | Response::SendTaskFailure(_)
        | Response::SendTaskHeartbeat(_) => Bytes::new(),
        _ => encode(|writer| value.encode(writer))?,
    };
    build(bytes, StatusCode::OK, id, None)
}
#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    #[serde(rename = "__type")]
    kind: &'a str,
    message: &'a str,
}
pub(crate) fn failure(error: &HttpError, id: Uuid) -> http::Response<StepFunctionsBody> {
    let (code, message, status) = error.wire();
    tracing::debug!(error_code = code, "Step Functions request rejected");
    let result = encode(|writer| {
        serde_json::to_writer(
            writer,
            &ErrorEnvelope {
                kind: code,
                message,
            },
        )
    })
    .and_then(|bytes| build(bytes, status, id, Some(code)));
    match result {
        Ok(response) => response,
        Err(_) => emergency(),
    }
}
fn build(
    bytes: Bytes,
    status: StatusCode,
    id: Uuid,
    error: Option<&'static str>,
) -> Result<http::Response<StepFunctionsBody>, HttpError> {
    let length = bytes.len();
    let mut response = http::Response::new(StepFunctionsBody::new(bytes));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(TYPE_HEADER, HeaderValue::from_static(CONTENT_TYPE));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(
        "x-amzn-requestid",
        HeaderValue::try_from(id.to_string()).map_err(|_| HttpError::Response)?,
    );
    headers.insert("server", HeaderValue::from_static("Rustack"));
    if let Some(code) = error {
        headers.insert("x-amzn-errortype", HeaderValue::from_static(code));
    }
    Ok(response)
}
fn emergency() -> http::Response<StepFunctionsBody> {
    let mut response = http::Response::new(StepFunctionsBody::new(Bytes::from_static(
        br#"{"__type":"InternalFailure","message":"Request processing failed"}"#,
    )));
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
        .headers_mut()
        .insert(TYPE_HEADER, HeaderValue::from_static(CONTENT_TYPE));
    response
}
