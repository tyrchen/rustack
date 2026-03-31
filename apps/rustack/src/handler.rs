//! S3 operation handler implementation for [`RustackS3`].
//!
//! This module bridges the HTTP layer (`rustack-s3-http`) with the business logic
//! (`rustack-s3-core`) by implementing the [`S3Handler`] trait. Each S3 operation is
//! dispatched to the corresponding `handle_*` method on [`RustackS3`], with request
//! deserialization via [`FromS3Request`] and response serialization via [`IntoS3Response`].

use std::{collections::HashMap, future::Future, pin::Pin};

use bytes::Bytes;
use rustack_s3_core::RustackS3;
use rustack_s3_http::{
    body::S3ResponseBody, dispatch::S3Handler, multipart, request::FromS3Request,
    response::IntoS3Response, router::RoutingContext,
};
use rustack_s3_model::{
    S3Operation,
    error::{S3Error, S3ErrorCode},
    input::PutObjectInput,
    request::StreamingBlob,
};

/// Wrapper that implements [`S3Handler`] by delegating to [`RustackS3`] handler methods.
#[derive(Debug, Clone)]
pub struct RustackHandler(pub RustackS3);

impl S3Handler for RustackHandler {
    // This function dispatches all S3 operations via a match expression. Each arm
    // is a single-line delegation, so the overall line count is proportional to
    // the number of S3 operations rather than logic complexity.
    #[allow(clippy::too_many_lines)]
    fn handle_operation(
        &self,
        op: S3Operation,
        parts: http::request::Parts,
        body: Bytes,
        ctx: RoutingContext,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<S3ResponseBody>, S3Error>> + Send>> {
        let provider = self.0.clone();
        Box::pin(async move {
            let bucket = ctx.bucket.as_deref();
            let key = ctx.key.as_deref();
            let query_params = &ctx.query_params;

            match op {
                // ---------------------------------------------------------------
                // Bucket CRUD
                // ---------------------------------------------------------------
                S3Operation::CreateBucket => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_create_bucket(input)
                    })
                    .await
                }
                S3Operation::DeleteBucket => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket(input)
                    })
                    .await
                }
                S3Operation::HeadBucket => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_head_bucket(input)
                    })
                    .await
                }
                S3Operation::ListBuckets => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_buckets(input)
                    })
                    .await
                }
                S3Operation::GetBucketLocation => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_location(input)
                    })
                    .await
                }

                // ---------------------------------------------------------------
                // Bucket Configuration
                // ---------------------------------------------------------------
                S3Operation::GetBucketVersioning => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_versioning(input)
                    })
                    .await
                }
                S3Operation::PutBucketVersioning => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_versioning(input)
                    })
                    .await
                }
                S3Operation::GetBucketEncryption => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_encryption(input)
                    })
                    .await
                }
                S3Operation::PutBucketEncryption => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_encryption(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketEncryption => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_encryption(input)
                    })
                    .await
                }
                S3Operation::GetBucketCors => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_cors(input)
                    })
                    .await
                }
                S3Operation::PutBucketCors => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_cors(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketCors => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_cors(input)
                    })
                    .await
                }
                S3Operation::GetBucketLifecycleConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_lifecycle_configuration(input)
                    })
                    .await
                }
                S3Operation::PutBucketLifecycleConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_lifecycle_configuration(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketLifecycle => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_lifecycle(input)
                    })
                    .await
                }
                S3Operation::GetBucketPolicy => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_policy(input)
                    })
                    .await
                }
                S3Operation::PutBucketPolicy => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_policy(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketPolicy => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_policy(input)
                    })
                    .await
                }
                S3Operation::GetBucketTagging => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_tagging(input)
                    })
                    .await
                }
                S3Operation::PutBucketTagging => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_tagging(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketTagging => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_tagging(input)
                    })
                    .await
                }
                S3Operation::GetBucketNotificationConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_notification_configuration(input)
                    })
                    .await
                }
                S3Operation::PutBucketNotificationConfiguration => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_notification_configuration(input)
                    })
                    .await
                }
                S3Operation::GetBucketLogging => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_logging(input)
                    })
                    .await
                }
                S3Operation::PutBucketLogging => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_logging(input)
                    })
                    .await
                }
                S3Operation::GetPublicAccessBlock => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_public_access_block(input)
                    })
                    .await
                }
                S3Operation::PutPublicAccessBlock => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_public_access_block(input)
                    })
                    .await
                }
                S3Operation::DeletePublicAccessBlock => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_public_access_block(input)
                    })
                    .await
                }
                S3Operation::GetBucketOwnershipControls => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_ownership_controls(input)
                    })
                    .await
                }
                S3Operation::PutBucketOwnershipControls => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_ownership_controls(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketOwnershipControls => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_ownership_controls(input)
                    })
                    .await
                }
                S3Operation::GetObjectLockConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_lock_configuration(input)
                    })
                    .await
                }
                S3Operation::PutObjectLockConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object_lock_configuration(input)
                    })
                    .await
                }
                S3Operation::GetBucketAccelerateConfiguration => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_accelerate_configuration(input)
                    })
                    .await
                }
                S3Operation::PutBucketAccelerateConfiguration => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_accelerate_configuration(input)
                    })
                    .await
                }
                S3Operation::GetBucketRequestPayment => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_request_payment(input)
                    })
                    .await
                }
                S3Operation::PutBucketRequestPayment => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_request_payment(input)
                    })
                    .await
                }
                S3Operation::GetBucketWebsite => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_website(input)
                    })
                    .await
                }
                S3Operation::PutBucketWebsite => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_website(input)
                    })
                    .await
                }
                S3Operation::DeleteBucketWebsite => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_bucket_website(input)
                    })
                    .await
                }
                S3Operation::GetBucketAcl => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_acl(input)
                    })
                    .await
                }
                S3Operation::PutBucketAcl => {
                    dispatch_void(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_bucket_acl(input)
                    })
                    .await
                }
                S3Operation::GetBucketPolicyStatus => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_bucket_policy_status(input)
                    })
                    .await
                }

                // ---------------------------------------------------------------
                // Object CRUD
                // ---------------------------------------------------------------
                S3Operation::PutObject => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object(input)
                    })
                    .await
                }
                S3Operation::GetObject => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object(input)
                    })
                    .await
                }
                S3Operation::HeadObject => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_head_object(input)
                    })
                    .await
                }
                S3Operation::DeleteObject => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_object(input)
                    })
                    .await
                }
                S3Operation::DeleteObjects => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_objects(input)
                    })
                    .await
                }
                S3Operation::CopyObject => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_copy_object(input)
                    })
                    .await
                }
                S3Operation::PostObject => {
                    dispatch_post_object(&parts, bucket, body, &provider).await
                }

                // ---------------------------------------------------------------
                // Object Configuration
                // ---------------------------------------------------------------
                S3Operation::GetObjectTagging => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_tagging(input)
                    })
                    .await
                }
                S3Operation::PutObjectTagging => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object_tagging(input)
                    })
                    .await
                }
                S3Operation::DeleteObjectTagging => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_delete_object_tagging(input)
                    })
                    .await
                }
                S3Operation::GetObjectAcl => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_acl(input)
                    })
                    .await
                }
                S3Operation::PutObjectAcl => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object_acl(input)
                    })
                    .await
                }
                S3Operation::GetObjectRetention => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_retention(input)
                    })
                    .await
                }
                S3Operation::PutObjectRetention => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object_retention(input)
                    })
                    .await
                }
                S3Operation::GetObjectLegalHold => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_legal_hold(input)
                    })
                    .await
                }
                S3Operation::PutObjectLegalHold => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_put_object_legal_hold(input)
                    })
                    .await
                }
                S3Operation::GetObjectAttributes => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_get_object_attributes(input)
                    })
                    .await
                }

                // ---------------------------------------------------------------
                // List Operations
                // ---------------------------------------------------------------
                S3Operation::ListObjects => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_objects(input)
                    })
                    .await
                }
                S3Operation::ListObjectsV2 => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_objects_v2(input)
                    })
                    .await
                }
                S3Operation::ListObjectVersions => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_object_versions(input)
                    })
                    .await
                }

                // ---------------------------------------------------------------
                // Multipart Operations
                // ---------------------------------------------------------------
                S3Operation::CreateMultipartUpload => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_create_multipart_upload(input)
                    })
                    .await
                }
                S3Operation::UploadPart => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_upload_part(input)
                    })
                    .await
                }
                S3Operation::UploadPartCopy => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_upload_part_copy(input)
                    })
                    .await
                }
                S3Operation::CompleteMultipartUpload => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_complete_multipart_upload(input)
                    })
                    .await
                }
                S3Operation::AbortMultipartUpload => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_abort_multipart_upload(input)
                    })
                    .await
                }
                S3Operation::ListParts => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_parts(input)
                    })
                    .await
                }
                S3Operation::ListMultipartUploads => {
                    dispatch_output(&parts, bucket, key, query_params, body, |input| {
                        provider.handle_list_multipart_uploads(input)
                    })
                    .await
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Generic dispatch helpers
// ---------------------------------------------------------------------------

/// Dispatch an operation that returns a typed Output implementing [`IntoS3Response`].
///
/// 1. Deserializes the HTTP request into the Input type via [`FromS3Request`].
/// 2. Calls the handler function with the deserialized input.
/// 3. Serializes the output into an HTTP response via [`IntoS3Response`].
async fn dispatch_output<I, O, F, Fut>(
    parts: &http::request::Parts,
    bucket: Option<&str>,
    key: Option<&str>,
    query_params: &[(String, String)],
    body: Bytes,
    handler_fn: F,
) -> Result<http::Response<S3ResponseBody>, S3Error>
where
    I: FromS3Request,
    O: IntoS3Response,
    F: FnOnce(I) -> Fut,
    Fut: Future<Output = Result<O, S3Error>>,
{
    let input = I::from_s3_request(parts, bucket, key, query_params, body)?;
    let output = handler_fn(input).await?;
    output.into_s3_response()
}

/// Dispatch an operation that returns `Result<(), S3Error>` (void result).
///
/// Returns a 204 No Content response on success.
async fn dispatch_void<I, F, Fut>(
    parts: &http::request::Parts,
    bucket: Option<&str>,
    key: Option<&str>,
    query_params: &[(String, String)],
    body: Bytes,
    handler_fn: F,
) -> Result<http::Response<S3ResponseBody>, S3Error>
where
    I: FromS3Request,
    F: FnOnce(I) -> Fut,
    Fut: Future<Output = Result<(), S3Error>>,
{
    let input = I::from_s3_request(parts, bucket, key, query_params, body)?;
    handler_fn(input).await?;
    http::Response::builder()
        .status(http::StatusCode::NO_CONTENT)
        .body(S3ResponseBody::empty())
        .map_err(|e| S3Error::internal_error(e.to_string()))
}

/// Dispatch an S3 POST Object (browser-based / presigned POST upload).
///
/// This is a special case that doesn't use `FromS3Request` because the key and body
/// come from multipart form data rather than URL path and raw body.
async fn dispatch_post_object(
    parts: &http::request::Parts,
    bucket: Option<&str>,
    body: Bytes,
    provider: &RustackS3,
) -> Result<http::Response<S3ResponseBody>, S3Error> {
    let bucket_name = bucket
        .ok_or_else(|| S3Error::with_message(S3ErrorCode::InvalidRequest, "Missing bucket name"))?;

    // Extract Content-Type header to get the boundary.
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            S3Error::with_message(
                S3ErrorCode::InvalidRequest,
                "POST Object requires Content-Type header",
            )
        })?;

    let boundary = multipart::extract_boundary(content_type)?;
    let form = multipart::parse_multipart(&body, &boundary)?;

    // The "key" field is required.
    let key = form.fields.get("key").ok_or_else(|| {
        S3Error::with_message(
            S3ErrorCode::InvalidRequest,
            "Missing required 'key' field in POST form data",
        )
    })?;

    // Determine success_action_status (default 204).
    let success_status = form
        .fields
        .get("success_action_status")
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(204);

    // Extract user metadata from form fields (x-amz-meta-* fields).
    let metadata: HashMap<String, String> = form
        .fields
        .iter()
        .filter(|(k, _)| k.starts_with("x-amz-meta-"))
        .map(|(k, v)| {
            (
                k.strip_prefix("x-amz-meta-").unwrap_or(k).to_owned(),
                v.clone(),
            )
        })
        .collect();

    // Build a PutObjectInput and delegate to the existing handler.
    let input = PutObjectInput {
        bucket: bucket_name.to_owned(),
        key: key.clone(),
        body: Some(StreamingBlob::new(form.file_data)),
        content_type: form.file_content_type,
        metadata,
        ..PutObjectInput::default()
    };

    let output = provider.handle_put_object(input).await?;

    // Build response based on success_action_status.
    match success_status {
        200 | 201 => {
            // Return XML response with bucket, key, etag, location.
            let etag = output.e_tag.unwrap_or_default();
            let location = format!("/{bucket_name}/{key}");
            let xml = format!(
                "<?xml version=\"1.0\" \
                 encoding=\"UTF-8\"?>\n<PostResponse>\n<Location>{location}</Location>\\
                 \
                 n<Bucket>{bucket_name}</Bucket>\n<Key>{key}</Key>\n<ETag>{etag}</ETag>\n</\
                 PostResponse>"
            );
            http::Response::builder()
                .status(success_status)
                .header("Content-Type", "application/xml")
                .header("ETag", &etag)
                .body(S3ResponseBody::from_string(xml))
                .map_err(|e| S3Error::internal_error(e.to_string()))
        }
        _ => {
            // 204 No Content (default).
            let mut builder = http::Response::builder().status(204);
            if let Some(etag) = &output.e_tag {
                builder = builder.header("ETag", etag);
            }
            builder
                .body(S3ResponseBody::empty())
                .map_err(|e| S3Error::internal_error(e.to_string()))
        }
    }
}
