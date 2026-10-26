//! Bounded deployment DTO parsing with one borrowed inline-code exception.
//!
//! Pure blocking parse tasks, completed results and active dispatch retain one
//! of four permits until their bounded input work is released. An
//! abandoned caller intentionally leaves only bounded CPU/memory work running;
//! parsing cannot commit metadata, filesystem or Docker effects.

use std::collections::BTreeMap;

use bytes::Bytes;
use rustack_core::{
    http::append_bounded,
    json::{JsonLimits, validate},
};
use rustack_lambda_model::error::{LambdaError, LambdaErrorCode};
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use tokio::{
    sync::{Semaphore, SemaphorePermit},
    task::spawn_blocking,
};

use crate::{artifact::MAX_ZIP_BASE64_BYTES, error::LambdaServiceError};

const BODY_BYTES: usize = 96 * 1024 * 1024;
const META_BYTES: usize = 1024 * 1024;
const STRING_BYTES: usize = 16 * 1024;
const KEY_BYTES: usize = 256;
const DEPTH: usize = 8;
const MEMBERS: usize = 4096;
// A fixed aggregate safety ceiling for large deployment parsing. Request
// admission remains separately operator-configurable at the HTTP boundary.
static PARSERS: Semaphore = Semaphore::const_new(4);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Profile {
    Create,
    Code,
    Configuration,
}

fn invalid() -> LambdaError {
    LambdaError::invalid_parameter("Invalid or unsupported deployment request")
}

pub(crate) async fn decode<T: DeserializeOwned + Send + 'static>(
    body: &Bytes,
    profile: Profile,
) -> Result<(T, SemaphorePermit<'static>), LambdaError> {
    let body = body.clone();
    run(&PARSERS, move || parse(&body, profile)).await
}

async fn run<T: Send + 'static>(
    pool: &'static Semaphore,
    work: impl FnOnce() -> Result<T, LambdaError> + Send + 'static,
) -> Result<(T, SemaphorePermit<'static>), LambdaError> {
    let permit = pool.try_acquire().map_err(|_| {
        LambdaError::new(
            LambdaErrorCode::TooManyRequestsException,
            "Deployment parser capacity exhausted",
        )
    })?;
    run_admitted(permit, work).await
}

async fn run_admitted<T: Send + 'static>(
    permit: SemaphorePermit<'static>,
    work: impl FnOnce() -> Result<T, LambdaError> + Send + 'static,
) -> Result<(T, SemaphorePermit<'static>), LambdaError> {
    spawn_blocking(move || {
        let value = work()?;
        Ok((value, permit))
    })
    .await
    .map_err(|_| LambdaError::service_error("Deployment parser failed"))?
}

fn parse<T: DeserializeOwned>(body: &[u8], profile: Profile) -> Result<T, LambdaError> {
    lexical(body)?;
    // Borrow raw values, so large strings are not allocated before identifying
    // the only field allowed to exceed the ordinary metadata allowance.
    let fields: BTreeMap<String, &RawValue> =
        serde_json::from_slice(body).map_err(|_| invalid())?;
    let blob = match profile {
        Profile::Code => fields.get("ZipFile").copied(),
        Profile::Create => match fields.get("Code") {
            Some(code) => {
                let code: BTreeMap<String, &RawValue> =
                    serde_json::from_str(code.get()).map_err(|_| invalid())?;
                code.get("ZipFile").copied()
            }
            None => None,
        },
        Profile::Configuration => None,
    }
    .filter(|value| value.get().as_bytes().first() == Some(&b'"'));
    let mut metadata = Vec::new();
    if let Some(blob) = blob {
        let raw = blob.get().as_bytes();
        if raw
            .len()
            .checked_sub(2)
            .is_none_or(|size| size > MAX_ZIP_BASE64_BYTES)
        {
            return Err(invalid());
        }
        // Borrowed RawValue is a span in this very input (serde_json's slice
        // deserializer contract). Checked integer offsets never dereference a
        // raw pointer; bounded slice access re-establishes the range.
        let start = raw
            .as_ptr()
            .addr()
            .checked_sub(body.as_ptr().addr())
            .ok_or_else(invalid)?;
        let end = start.checked_add(raw.len()).ok_or_else(invalid)?;
        append(&mut metadata, body.get(..start).ok_or_else(invalid)?)?;
        append(&mut metadata, b"\"\"")?;
        append(&mut metadata, body.get(end..).ok_or_else(invalid)?)?;
    } else {
        append(&mut metadata, body)?;
    }
    let limits = JsonLimits::new(META_BYTES, DEPTH, MEMBERS, STRING_BYTES, KEY_BYTES)
        .map_err(|_| invalid())?;
    validate(&metadata, limits).map_err(|_| invalid())?;
    serde_json::from_slice(body).map_err(|_| invalid())
}
fn append(metadata: &mut Vec<u8>, bytes: &[u8]) -> Result<(), LambdaError> {
    append_bounded(metadata, bytes, 1024 * 1024).map_err(|_| invalid())
}

/// Bound structure and encoded keys before borrowed-map key materialization.
fn lexical(body: &[u8]) -> Result<(), LambdaError> {
    if body.len() > BODY_BYTES {
        return Err(invalid());
    }
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut delimiters = 0usize;
    while let Some(byte) = body.get(cursor) {
        match byte {
            b'"' => {
                let start = cursor;
                cursor = quoted_end(body, cursor)?;
                let following = body
                    .get(cursor..)
                    .ok_or_else(invalid)?
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace());
                if following == Some(&b':') && cursor.saturating_sub(start) > KEY_BYTES * 6 + 2 {
                    return Err(invalid());
                }
                continue;
            }
            b'{' | b'[' => {
                depth = depth
                    .checked_add(1)
                    .filter(|depth| *depth <= DEPTH)
                    .ok_or_else(invalid)?;
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1).ok_or_else(invalid)?;
            }
            b',' | b':' => {
                delimiters = delimiters
                    .checked_add(1)
                    .filter(|count| *count <= MEMBERS * 2)
                    .ok_or_else(invalid)?;
            }
            _ => {}
        }
        cursor = cursor.checked_add(1).ok_or_else(invalid)?;
    }
    Ok(())
}
fn quoted_end(body: &[u8], start: usize) -> Result<usize, LambdaError> {
    let mut cursor = start.checked_add(1).ok_or_else(invalid)?;
    loop {
        match body.get(cursor) {
            Some(b'"') => return cursor.checked_add(1).ok_or_else(invalid),
            Some(b'\\') => cursor = cursor.checked_add(2).ok_or_else(invalid)?,
            Some(_) => cursor = cursor.checked_add(1).ok_or_else(invalid)?,
            None => return Err(invalid()),
        }
    }
}

/// Check standard canonical base64 without allocating decoded archive contents.
pub(crate) async fn validate_inline(
    encoded: String,
    admitted: Option<SemaphorePermit<'static>>,
) -> Result<SemaphorePermit<'static>, LambdaServiceError> {
    let result = if let Some(permit) = admitted {
        run_admitted(permit, move || inline(&encoded)).await
    } else {
        run(&PARSERS, move || inline(&encoded)).await
    };
    result
        .map(|((), permit)| permit)
        .map_err(|error| match error.code {
            LambdaErrorCode::TooManyRequestsException => LambdaServiceError::TooManyRequests,
            LambdaErrorCode::ServiceException => LambdaServiceError::Internal {
                message: "Deployment parser failed".into(),
            },
            _ => LambdaServiceError::InvalidParameter {
                message: "Invalid inline code encoding".into(),
            },
        })
}

fn inline(encoded: &str) -> Result<(), LambdaError> {
    if encoded.len() > MAX_ZIP_BASE64_BYTES || !encoded.len().is_multiple_of(4) {
        return Err(invalid());
    }
    let data = encoded.trim_end_matches('=');
    let padding = encoded.len().checked_sub(data.len()).ok_or_else(invalid)?;
    if padding > 2 {
        return Err(invalid());
    }
    let mut last = 0;
    for byte in data.bytes() {
        last = match byte {
            b'A'..=b'Z' => byte.checked_sub(b'A'),
            b'a'..=b'z' => byte
                .checked_sub(b'a')
                .and_then(|value| value.checked_add(26)),
            b'0'..=b'9' => byte
                .checked_sub(b'0')
                .and_then(|value| value.checked_add(52)),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
        .ok_or_else(invalid)?;
    }
    if (padding == 2 && last & 15 != 0) || (padding == 1 && last & 3 != 0) {
        return Err(invalid());
    }
    let decoded = encoded
        .len()
        .checked_div(4)
        .and_then(|groups| groups.checked_mul(3))
        .and_then(|bytes| bytes.checked_sub(padding))
        .ok_or_else(invalid)?;
    if decoded > crate::artifact::MAX_ZIP_BYTES {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
#[path = "deployment_input_tests.rs"]
mod tests;
