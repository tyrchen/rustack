//! Bounded, authenticated S3 upload staging; publication is performed by the core handler.

use std::{collections::BTreeMap, sync::Arc};

use bytes::{Buf, Bytes};
use http_body_util::BodyExt;
use rustack_auth::{AuthMode, sigv4::StreamingVerifier};
use rustack_core::http::{BodyBudget, BudgetedBody, S3_OBJECT_BODY_LIMIT};
use rustack_s3_core::{
    checksums::ChecksumAlgorithm,
    storage::{StagedUpload, UploadWriter},
};
use rustack_s3_model::error::{S3Error, S3ErrorCode};
use sha2::{Digest, Sha256};

fn invalid(message: impl std::fmt::Display) -> S3Error {
    S3Error::with_message(S3ErrorCode::InvalidRequest, message.to_string())
}
fn read_error(error: &rustack_core::http::BodyReadError) -> S3Error {
    let mut result = invalid(error.to_string());
    result.status_code = error.status_code();
    result
}

fn denied(error: impl std::fmt::Display) -> S3Error {
    S3Error::with_message(S3ErrorCode::AccessDenied, error.to_string())
}

/// Stage and authenticate decoded bytes without allocating a complete object.
pub(crate) async fn receive<B>(
    parts: &mut http::request::Parts,
    incoming: B,
    mode: AuthMode<'_>,
) -> Result<Arc<StagedUpload>, S3Error>
where
    B: http_body::Body<Data = Bytes>,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    for name in [
        "x-amz-content-sha256",
        "x-amz-trailer",
        "x-amz-decoded-content-length",
        "authorization",
    ] {
        if parts.headers.get_all(name).iter().count() > 1 {
            return Err(invalid("duplicate upload integrity header"));
        }
    }
    let marker = parts
        .headers
        .get("x-amz-content-sha256")
        .map(|value| value.to_str())
        .transpose()
        .map_err(|_| invalid("invalid payload hash header"))?
        .unwrap_or("")
        .to_owned();
    let chunked = crate::codec::is_aws_chunked(parts);
    let signed_chunks = matches!(
        marker.as_str(),
        "STREAMING-AWS4-HMAC-SHA256-PAYLOAD" | "STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER"
    );
    if chunked && !signed_chunks && marker != "STREAMING-UNSIGNED-PAYLOAD-TRAILER" {
        return Err(invalid("unsupported aws-chunked payload protocol"));
    }
    let mut verifier = match mode {
        AuthMode::Required(provider) if signed_chunks => {
            Some(StreamingVerifier::new(parts, provider).map_err(denied)?)
        }
        _ => None,
    };
    // Non-chunked strict uploads fail identity checks before any body is staged.
    if !chunked {
        if let AuthMode::Required(provider) = mode {
            fast_fail_strict_auth(parts, provider)?;
        }
    }
    // Encoded framing overhead is bounded independently; decoded object bytes have a 5 GiB cap.
    let budget = if chunked {
        BodyBudget::s3_encoded()
    } else {
        BodyBudget::s3_object()
    };
    let mut source = Source {
        body: BudgetedBody::new(incoming, budget),
        pending: Bytes::new(),
    };
    let mut writer = UploadWriter::new().await.map_err(invalid)?;
    let mut digest = Sha256::new();
    let mut trailers = BTreeMap::new();
    if chunked {
        trailers = source
            .copy_chunked(&mut writer, &mut digest, verifier.as_mut())
            .await?;
    } else {
        while let Some(data) = source.next().await? {
            digest.update(&data);
            writer.write(&data).await.map_err(invalid)?;
        }
    }
    let upload = Arc::new(writer.finish().await.map_err(invalid)?);
    let actual_hash = hex::encode(digest.finalize());
    if let Some(value) = parts.headers.get("x-amz-decoded-content-length") {
        let declared = value
            .to_str()
            .map_err(invalid)?
            .parse::<u64>()
            .map_err(invalid)?;
        if declared != upload.size() {
            return Err(invalid("decoded content length mismatch"));
        }
    }
    if !marker.is_empty()
        && !marker.starts_with("STREAMING-")
        && marker != "UNSIGNED-PAYLOAD"
        && marker != actual_hash
    {
        return Err(S3Error::with_message(
            S3ErrorCode::XAmzContentSHA256Mismatch,
            "payload digest mismatch",
        ));
    }
    if marker.starts_with("STREAMING-") && !chunked {
        return Err(invalid("streaming marker requires chunk framing"));
    }
    validate_trailers(parts, &upload, &trailers, verifier.as_mut(), &marker)?;
    authenticate(parts, mode, &actual_hash)?;
    for (name, value) in trailers {
        if name == "x-amz-trailer-signature" {
            continue;
        }
        let name = http::HeaderName::from_bytes(name.as_bytes()).map_err(invalid)?;
        let value = http::HeaderValue::from_str(&value).map_err(invalid)?;
        parts.headers.insert(name, value);
    }
    crate::codec::strip_aws_chunked_encoding(&mut parts.headers);
    Ok(upload)
}

/// Reject clearly invalid credentials before a strict-mode upload can spool any
/// bytes to disk: presigned URLs verify fully without a body, and a missing or
/// unknown access key cannot become valid by reading the body.
fn fast_fail_strict_auth(
    parts: &http::request::Parts,
    provider: &dyn rustack_auth::CredentialProvider,
) -> Result<(), S3Error> {
    if parts
        .uri
        .query()
        .is_some_and(|query| query.contains("X-Amz-Signature"))
    {
        return rustack_auth::verify_presigned(parts, provider)
            .map(|_| ())
            .map_err(denied);
    }
    let authorization = parts
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| denied("missing signature"))?;
    let access_key = if let Some(rest) = authorization.strip_prefix("AWS4-HMAC-SHA256 Credential=")
    {
        rest.split_once('/').map(|(key, _)| key)
    } else if let Some(rest) = authorization.strip_prefix("AWS ") {
        rest.split_once(':').map(|(key, _)| key)
    } else {
        None
    };
    if let Some(key) = access_key {
        provider
            .get_secret_key(key)
            .map_err(|_| denied("invalid access key"))?;
    } else if !authorization.starts_with("AWS4-HMAC-SHA256") {
        return Err(denied("malformed signature"));
    }
    Ok(())
}

fn authenticate(
    parts: &http::request::Parts,
    mode: AuthMode<'_>,
    actual_hash: &str,
) -> Result<(), S3Error> {
    if let AuthMode::Required(provider) = mode {
        if parts
            .uri
            .query()
            .is_some_and(|query| query.contains("X-Amz-Signature"))
        {
            rustack_auth::verify_presigned(parts, provider).map_err(denied)?;
        } else if parts
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(rustack_auth::is_sigv2)
        {
            rustack_auth::verify_sigv2(parts, provider).map_err(denied)?;
        } else {
            rustack_auth::sigv4::verify_s3_sigv4(parts, actual_hash, provider).map_err(denied)?;
        }
    }
    Ok(())
}

fn validate_trailers(
    parts: &http::request::Parts,
    upload: &StagedUpload,
    trailers: &BTreeMap<String, String>,
    verifier: Option<&mut StreamingVerifier>,
    marker: &str,
) -> Result<(), S3Error> {
    let declared = parts
        .headers
        .get("x-amz-trailer")
        .map(|value| value.to_str())
        .transpose()
        .map_err(invalid)?
        .unwrap_or("");
    let mut names: Vec<&str> = declared
        .split(',')
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    if names.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err(invalid("duplicate declared trailer"));
    }
    if marker.ends_with("-TRAILER") && names.is_empty() {
        return Err(invalid("missing declared checksum trailer"));
    }
    let mut canonical = String::new();
    for name in &names {
        let algorithm = match *name {
            "x-amz-checksum-crc32" => ChecksumAlgorithm::Crc32,
            "x-amz-checksum-crc32c" => ChecksumAlgorithm::Crc32c,
            "x-amz-checksum-crc64nvme" => ChecksumAlgorithm::Crc64Nvme,
            "x-amz-checksum-sha1" => ChecksumAlgorithm::Sha1,
            "x-amz-checksum-sha256" => ChecksumAlgorithm::Sha256,
            _ => return Err(invalid("invalid declared checksum trailer")),
        };
        let value = trailers
            .get(*name)
            .ok_or_else(|| invalid("missing checksum trailer"))?;
        if upload.checksum(algorithm).map_err(invalid)? != value {
            return Err(S3Error::with_message(
                S3ErrorCode::BadDigest,
                "trailer checksum mismatch",
            ));
        }
        if parts.headers.contains_key(*name) {
            return Err(invalid("checksum duplicated in header and trailer"));
        }
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
    }
    if trailers
        .keys()
        .any(|name| name != "x-amz-trailer-signature" && !names.contains(&name.as_str()))
    {
        return Err(invalid("undeclared trailer"));
    }
    if marker == "STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER" {
        let signature = trailers
            .get("x-amz-trailer-signature")
            .ok_or_else(|| denied("missing trailer signature"))?;
        if let Some(verifier) = verifier {
            verifier
                .verify_trailer(&canonical, signature)
                .map_err(denied)?;
        }
    } else if trailers.contains_key("x-amz-trailer-signature") {
        return Err(invalid("unexpected trailer signature"));
    }
    Ok(())
}

struct Source<B> {
    body: BudgetedBody<B>,
    pending: Bytes,
}

impl<B> Source<B>
where
    B: http_body::Body<Data = Bytes>,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    async fn copy_chunked(
        &mut self,
        writer: &mut UploadWriter,
        digest: &mut Sha256,
        mut verifier: Option<&mut StreamingVerifier>,
    ) -> Result<BTreeMap<String, String>, S3Error> {
        loop {
            let line = self.line().await?;
            let (size_text, signature) = match line.split_once(';') {
                Some((size, extension)) => (
                    size,
                    Some(
                        extension
                            .strip_prefix("chunk-signature=")
                            .ok_or_else(|| invalid("invalid chunk extension"))?,
                    ),
                ),
                None => (line.as_str(), None),
            };
            if size_text.is_empty()
                || size_text.len() > 16
                || !size_text.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("invalid chunk size"));
            }
            let size = u64::from_str_radix(size_text, 16).map_err(invalid)?;
            if size > S3_OBJECT_BODY_LIMIT {
                return Err(invalid("chunk exceeds object budget"));
            }
            let mut remaining = size;
            let mut chunk_digest = Sha256::new();
            while remaining != 0 {
                let data = self.take(remaining).await?;
                remaining = remaining
                    .checked_sub(data.len() as u64)
                    .ok_or_else(|| invalid("chunk length overflow"))?;
                chunk_digest.update(&data);
                digest.update(&data);
                writer.write(&data).await.map_err(invalid)?;
            }
            if let Some(verifier) = verifier.as_mut() {
                verifier
                    .verify_chunk(
                        &hex::encode(chunk_digest.finalize()),
                        signature.ok_or_else(|| denied("missing chunk signature"))?,
                    )
                    .map_err(denied)?;
            }
            if size == 0 {
                return self.trailers().await;
            }
            if !self.line().await?.is_empty() {
                return Err(invalid("missing chunk data terminator"));
            }
        }
    }

    async fn trailers(&mut self) -> Result<BTreeMap<String, String>, S3Error> {
        let mut trailers = BTreeMap::new();
        loop {
            let line = self.line().await?;
            if line.is_empty() {
                break;
            }
            if trailers.len() >= 8 {
                return Err(invalid("too many trailing headers"));
            }
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| invalid("invalid trailer"))?;
            let name = name.to_ascii_lowercase();
            if !matches!(
                name.as_str(),
                "x-amz-checksum-crc32"
                    | "x-amz-checksum-crc32c"
                    | "x-amz-checksum-crc64nvme"
                    | "x-amz-checksum-sha1"
                    | "x-amz-checksum-sha256"
                    | "x-amz-trailer-signature"
            ) {
                return Err(invalid("unsupported trailer"));
            }
            if trailers.insert(name, value.trim().to_owned()).is_some() {
                return Err(invalid("duplicate trailer"));
            }
        }
        self.finish().await?;
        Ok(trailers)
    }

    async fn next(&mut self) -> Result<Option<Bytes>, S3Error> {
        if !self.pending.is_empty() {
            return Ok(Some(std::mem::take(&mut self.pending)));
        }
        while let Some(frame) = self.body.frame().await {
            let frame = frame.map_err(|error| read_error(&error))?;
            match frame.into_data() {
                Ok(data) if !data.is_empty() => return Ok(Some(data)),
                Ok(_) => {}
                Err(_) => return Err(invalid("HTTP trailers are not aws-chunked trailers")),
            }
        }
        Ok(None)
    }

    async fn take(&mut self, max: u64) -> Result<Bytes, S3Error> {
        let mut data = self
            .next()
            .await?
            .ok_or_else(|| invalid("truncated upload"))?;
        let length = usize::try_from(max).unwrap_or(usize::MAX).min(data.len());
        let result = data.split_to(length);
        self.pending = data;
        Ok(result)
    }

    async fn line(&mut self) -> Result<String, S3Error> {
        let mut line = Vec::new();
        loop {
            let mut data = self
                .next()
                .await?
                .ok_or_else(|| invalid("truncated chunk framing"))?;
            while let Some(byte) = data.first().copied() {
                data.advance(1);
                line.push(byte);
                if line.len() > 8192 {
                    return Err(invalid("chunk header exceeds 8 KiB"));
                }
                if line.ends_with(b"\r\n") {
                    line.truncate(line.len().saturating_sub(2));
                    self.pending = data;
                    return String::from_utf8(line).map_err(invalid);
                }
            }
        }
    }

    async fn finish(&mut self) -> Result<(), S3Error> {
        if self.next().await?.is_some() {
            return Err(invalid("unexpected data after terminal chunk"));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use http_body_util::{Full, StreamBody};
    use rustack_auth::{
        StaticCredentialProvider,
        sigv4::{build_string_to_sign, compute_signature, derive_signing_key},
    };

    use super::*;

    fn parts(marker: &str) -> http::request::Parts {
        http::Request::builder()
            .method("PUT")
            .uri("/bucket/key")
            .header("x-amz-content-sha256", marker)
            .body(())
            .unwrap()
            .into_parts()
            .0
    }

    #[tokio::test]
    async fn test_should_stage_object_larger_than_control_limit_in_bounded_frames() {
        let chunk = Bytes::from(vec![7; 64 * 1024]);
        let frames = futures::stream::iter(
            (0..272).map(move |_| Ok::<_, Infallible>(http_body::Frame::data(chunk.clone()))),
        );
        let mut parts = parts("UNSIGNED-PAYLOAD");
        let upload = receive(&mut parts, StreamBody::new(frames), AuthMode::Development)
            .await
            .unwrap();
        assert_eq!(upload.size(), 17 * 1024 * 1024);
        let path = upload.path().to_owned();
        assert!(path.exists());
        drop(upload);
        assert!(
            !path.exists(),
            "unpublished staged data must be removed on cancellation/drop"
        );
    }

    #[tokio::test]
    async fn test_should_validate_unsigned_streaming_trailer_and_declared_length() {
        let wire = b"5\r\nhello\r\n0\r\nx-amz-checksum-crc32:NhCmhg==\r\n\r\n";
        let mut request = parts("STREAMING-UNSIGNED-PAYLOAD-TRAILER");
        request
            .headers
            .insert("x-amz-trailer", "x-amz-checksum-crc32".parse().unwrap());
        request
            .headers
            .insert("x-amz-decoded-content-length", "5".parse().unwrap());
        let upload = receive(
            &mut request,
            Full::new(Bytes::from_static(wire)),
            AuthMode::Development,
        )
        .await
        .unwrap();
        assert_eq!(upload.size(), 5);
        assert_eq!(
            upload.checksum(ChecksumAlgorithm::Crc32).unwrap(),
            "NhCmhg=="
        );
        for bad in [b"5\r\njello\r\n0\r\nx-amz-checksum-crc32:NhCmhg==\r\n\r\n".as_slice(), b"5\r\nhello\r\n0\r\nx-amz-checksum-crc32:NhCmhg==\r\nx-amz-checksum-crc32:NhCmhg==\r\n\r\n", b"5\r\nhello\r\n0\r\nx-amz-checksum-crc32:NhCmhg==\r\n\r\nextra"] {
            let mut request = parts("STREAMING-UNSIGNED-PAYLOAD-TRAILER");
            request.headers.insert("x-amz-trailer", "x-amz-checksum-crc32".parse().unwrap());
            assert!(receive(&mut request, Full::new(Bytes::copy_from_slice(bad)), AuthMode::Development).await.is_err());
        }
    }

    #[tokio::test]
    async fn test_should_reject_invalid_auth_before_staging_any_bytes() {
        use std::{
            pin::Pin,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            task::{Context, Poll},
        };

        use http_body::Frame;

        struct Counting {
            polled: Arc<AtomicUsize>,
            data: Bytes,
        }
        impl http_body::Body for Counting {
            type Data = Bytes;
            type Error = Infallible;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
                self.polled.fetch_add(1, Ordering::Relaxed);
                if self.data.is_empty() {
                    return Poll::Ready(None);
                }
                let data = std::mem::take(&mut self.data);
                Poll::Ready(Some(Ok(Frame::data(data))))
            }
        }

        let provider = StaticCredentialProvider::new(vec![]);
        let polled = Arc::new(AtomicUsize::new(0));
        for authorization in [
            None,
            Some(
                "AWS4-HMAC-SHA256 \
                 Credential=UNKNOWN/20260101/us-east-1/s3/aws4_request,SignedHeaders=host,\
                 Signature=00",
            ),
        ] {
            let mut request = parts("UNSIGNED-PAYLOAD");
            request.headers.insert("host", "localhost".parse().unwrap());
            if let Some(authorization) = authorization {
                request
                    .headers
                    .insert("authorization", authorization.parse().unwrap());
            }
            let error = receive(
                &mut request,
                Counting {
                    polled: Arc::clone(&polled),
                    data: Bytes::from(vec![7; 64 * 1024]),
                },
                AuthMode::Required(&provider),
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("AccessDenied"),
                "unexpected error: {error}"
            );
        }
        assert_eq!(
            polled.load(Ordering::Relaxed),
            0,
            "strict-mode rejection must not read any body bytes"
        );
    }

    // The signing matrix is long on purpose; fixtures stay readable in one test.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn test_should_verify_signed_stream_and_reject_corrupt_chunk_before_publication() {
        for marker in [
            "STREAMING-AWS4-HMAC-SHA256-PAYLOAD",
            "STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER",
        ] {
            let provider =
                StaticCredentialProvider::new(vec![("key".to_owned(), "secret".to_owned())]);
            let key = derive_signing_key("secret", "20260101", "us-east-1", "s3");
            let canonical = rustack_auth::canonical::build_canonical_request(
                "PUT",
                "/bucket/key",
                "",
                &[("host", "localhost"), ("x-amz-date", "20260101T000000Z")],
                &["host", "x-amz-date"],
                marker,
            );
            let seed = compute_signature(
                &key,
                &build_string_to_sign(
                    "20260101T000000Z",
                    "20260101/us-east-1/s3/aws4_request",
                    &rustack_auth::hash_payload(canonical.as_bytes()),
                ),
            );
            let make_parts = || {
                let mut request = parts(marker);
                if marker.ends_with("-TRAILER") {
                    request
                        .headers
                        .insert("x-amz-trailer", "x-amz-checksum-crc32".parse().unwrap());
                }
                request.headers.insert("host", "localhost".parse().unwrap());
                request
                    .headers
                    .insert("x-amz-date", "20260101T000000Z".parse().unwrap());
                request.headers.insert(
                    "authorization",
                    format!(
                        "AWS4-HMAC-SHA256 \
                         Credential=key/20260101/us-east-1/s3/aws4_request,SignedHeaders=host;\
                         x-amz-date,Signature={seed}"
                    )
                    .parse()
                    .unwrap(),
                );
                request
            };
            let sign = |previous: &str, data: &[u8]| {
                compute_signature(
                    &key,
                    &format!(
                        "AWS4-HMAC-SHA256-PAYLOAD\n20260101T000000Z\n20260101/us-east-1/s3/\
                         aws4_request\n{previous}\n{}\n{}",
                        rustack_auth::hash_payload(b""),
                        rustack_auth::hash_payload(data)
                    ),
                )
            };
            let first = sign(&seed, b"hello");
            let terminal = sign(&first, b"");
            let trailers = if marker.ends_with("-TRAILER") {
                let signature = compute_signature(
                    &key,
                    &format!(
                        "AWS4-HMAC-SHA256-TRAILER\n20260101T000000Z\n20260101/us-east-1/s3/\
                         aws4_request\n{terminal}\n{}",
                        rustack_auth::hash_payload(b"x-amz-checksum-crc32:NhCmhg==\n")
                    ),
                );
                format!("x-amz-checksum-crc32:NhCmhg==\r\nx-amz-trailer-signature:{signature}\r\n")
            } else {
                String::new()
            };
            let mut wire =
                format!("5;chunk-signature={first}\r\nhello\r\n0;chunk-signature={terminal}\r\n");
            wire.push_str(&trailers);
            wire.push_str("\r\n");
            let upload = receive(
                &mut make_parts(),
                Full::new(Bytes::from(wire.clone())),
                AuthMode::Required(&provider),
            )
            .await
            .unwrap();
            assert_eq!(upload.size(), 5);
            if marker.ends_with("-TRAILER") {
                let bad_signature =
                    wire.replace("x-amz-trailer-signature:", "x-amz-trailer-signature:0");
                assert!(
                    receive(
                        &mut make_parts(),
                        Full::new(Bytes::from(bad_signature)),
                        AuthMode::Required(&provider)
                    )
                    .await
                    .is_err()
                );
            }
            let tampered = wire.replace("hello", "jello");
            assert!(
                receive(
                    &mut make_parts(),
                    Full::new(Bytes::from(tampered)),
                    AuthMode::Required(&provider)
                )
                .await
                .is_err()
            );
        }
    }
}
