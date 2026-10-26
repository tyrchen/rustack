//! Exact target-family claiming and bounded HTTP framing policy.

use http::{
    HeaderMap, Method,
    header::{
        AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE as TYPE_HEADER, HOST,
        TRANSFER_ENCODING,
    },
    request::Parts,
};
use rustack_stepfunctions_model::StepFunctionsOperation;

use crate::HttpError;

/// AWS JSON1.0 media type emitted by every transport response.
pub const CONTENT_TYPE: &str = "application/x-amz-json-1.0";
const PREFIX: &str = "AWSStepFunctions.";

/// Claim a target family even when its action or framing will be rejected.
///
/// ```
/// let mut headers=http::HeaderMap::new();
/// headers.insert("x-amz-target",http::HeaderValue::from_static("AWSStepFunctions.Unknown"));
/// assert!(rustack_stepfunctions_http::claims_request(&headers));
/// ```
#[must_use]
pub fn claims_request(headers: &HeaderMap) -> bool {
    headers.get_all("x-amz-target").iter().any(|value| {
        value.as_bytes().starts_with(PREFIX.as_bytes()) || value.as_bytes() == b"AWSStepFunctions"
    })
}
/// Resolve exactly one bounded target into a modeled action.
///
/// # Errors
/// Returns envelope errors for ambiguity/non-ASCII/oversize or `UnknownOperation`.
///
/// ```
/// let mut headers=http::HeaderMap::new();
/// headers.insert("x-amz-target",http::HeaderValue::from_static("AWSStepFunctions.StartExecution"));
/// let operation=rustack_stepfunctions_http::resolve_operation(&headers)?;
/// assert_eq!(operation,rustack_stepfunctions_model::StepFunctionsOperation::StartExecution);
/// # Ok::<(),rustack_stepfunctions_http::HttpError>(())
/// ```
pub fn resolve_operation(headers: &HeaderMap) -> Result<StepFunctionsOperation, HttpError> {
    let target = single(headers, "x-amz-target", 256)?.ok_or(HttpError::Envelope)?;
    let name = target
        .strip_prefix(PREFIX)
        .ok_or(HttpError::UnknownOperation)?;
    StepFunctionsOperation::from_name(name).ok_or(HttpError::UnknownOperation)
}
fn single<'a>(
    headers: &'a HeaderMap,
    name: &str,
    maximum: usize,
) -> Result<Option<&'a str>, HttpError> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() || value.as_bytes().len() > maximum || !value.as_bytes().is_ascii() {
        return Err(HttpError::Envelope);
    }
    value.to_str().map(Some).map_err(|_| HttpError::Envelope)
}
pub(crate) fn envelope(parts: &Parts) -> Result<Option<usize>, HttpError> {
    if parts.method != Method::POST || parts.uri.path() != "/" || parts.uri.query().is_some() {
        return Err(HttpError::Envelope);
    }
    let mut bytes = 0_usize;
    if parts.headers.len() > 64 {
        return Err(HttpError::Envelope);
    }
    for (name, value) in &parts.headers {
        if value.as_bytes().len() > 8192 {
            return Err(HttpError::Envelope);
        }
        bytes = bytes
            .checked_add(name.as_str().len())
            .and_then(|count| count.checked_add(value.as_bytes().len()))
            .ok_or(HttpError::Envelope)?;
        if bytes > 32768 {
            return Err(HttpError::Envelope);
        }
    }
    single(&parts.headers, "x-amz-target", 256)?.ok_or(HttpError::Envelope)?;
    let content = single(&parts.headers, TYPE_HEADER.as_str(), 256)?.ok_or(HttpError::Envelope)?;
    let mut fields = content.split(';');
    if !fields
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case(CONTENT_TYPE))
    {
        return Err(HttpError::Envelope);
    }
    if let Some(charset) = fields.next() {
        let charset = charset.trim();
        if !(charset.eq_ignore_ascii_case("charset=utf-8")
            || charset.eq_ignore_ascii_case("charset=\"utf-8\""))
            || fields.next().is_some()
        {
            return Err(HttpError::Envelope);
        }
    }
    if single(&parts.headers, CONTENT_ENCODING.as_str(), 64)?
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(HttpError::Envelope);
    }
    for name in [
        AUTHORIZATION.as_str(),
        HOST.as_str(),
        "x-amz-date",
        "x-amz-security-token",
        "x-amz-content-sha256",
    ] {
        single(&parts.headers, name, 8192)?;
    }
    let length = single(&parts.headers, CONTENT_LENGTH.as_str(), 20)?
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(HttpError::Envelope);
            }
            value.parse::<usize>().map_err(|_| HttpError::Envelope)
        })
        .transpose()?;
    if let Some(encoding) = single(&parts.headers, TRANSFER_ENCODING.as_str(), 64)? {
        if length.is_some() || !encoding.eq_ignore_ascii_case("chunked") {
            return Err(HttpError::Envelope);
        }
    }
    Ok(length)
}
