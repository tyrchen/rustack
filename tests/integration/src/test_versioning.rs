//! Versioning integration tests.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aws_sdk_s3::{primitives::ByteStream, types::BucketVersioningStatus};
    use reqwest::{Client, Method, Url};

    use crate::{cleanup_bucket, create_test_bucket, endpoint_url, s3_client};

    async fn enable_versioning(client: &aws_sdk_s3::Client, bucket: &str) {
        client
            .put_bucket_versioning()
            .bucket(bucket)
            .versioning_configuration(
                aws_sdk_s3::types::VersioningConfiguration::builder()
                    .status(BucketVersioningStatus::Enabled)
                    .build(),
            )
            .send()
            .await
            .expect("enable versioning");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_enable_and_check_versioning() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "ver").await;

        enable_versioning(&client, &bucket).await;

        let resp = client
            .get_bucket_versioning()
            .bucket(&bucket)
            .send()
            .await
            .expect("get versioning");

        assert_eq!(resp.status(), Some(&BucketVersioningStatus::Enabled),);

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_versions_on_overwrite() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "verput").await;
        enable_versioning(&client, &bucket).await;

        // Put two versions.
        let v1 = client
            .put_object()
            .bucket(&bucket)
            .key("versioned.txt")
            .body(ByteStream::from_static(b"v1"))
            .send()
            .await
            .expect("put v1");
        let v1_id = v1
            .version_id()
            .expect("v1 should have version_id")
            .to_owned();

        let v2 = client
            .put_object()
            .bucket(&bucket)
            .key("versioned.txt")
            .body(ByteStream::from_static(b"v2"))
            .send()
            .await
            .expect("put v2");
        let v2_id = v2
            .version_id()
            .expect("v2 should have version_id")
            .to_owned();

        assert_ne!(v1_id, v2_id, "version IDs should differ");

        // Latest get should return v2.
        let latest = client
            .get_object()
            .bucket(&bucket)
            .key("versioned.txt")
            .send()
            .await
            .expect("get latest");
        let data = latest.body.collect().await.expect("collect").into_bytes();
        assert_eq!(data.as_ref(), b"v2");

        // Get specific version v1.
        let old = client
            .get_object()
            .bucket(&bucket)
            .key("versioned.txt")
            .version_id(&v1_id)
            .send()
            .await
            .expect("get v1");
        let data = old.body.collect().await.expect("collect").into_bytes();
        assert_eq!(data.as_ref(), b"v1");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_object_versions() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "listver").await;
        enable_versioning(&client, &bucket).await;

        for i in 0..3 {
            client
                .put_object()
                .bucket(&bucket)
                .key("multi-ver.txt")
                .body(ByteStream::from(format!("version-{i}").into_bytes()))
                .send()
                .await
                .unwrap_or_else(|e| panic!("put version {i}: {e}"));
        }

        let resp = client
            .list_object_versions()
            .bucket(&bucket)
            .send()
            .await
            .expect("list_object_versions");

        let versions = resp.versions();
        assert_eq!(versions.len(), 3, "should have 3 versions");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_delete_marker() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "delmrk").await;
        enable_versioning(&client, &bucket).await;

        client
            .put_object()
            .bucket(&bucket)
            .key("to-delete.txt")
            .body(ByteStream::from_static(b"data"))
            .send()
            .await
            .expect("put");

        // Delete creates a delete marker.
        let del = client
            .delete_object()
            .bucket(&bucket)
            .key("to-delete.txt")
            .send()
            .await
            .expect("delete");

        assert!(del.delete_marker() == Some(true), "should be delete marker");

        // Get should now fail (object appears deleted).
        let result = client
            .get_object()
            .bucket(&bucket)
            .key("to-delete.txt")
            .send()
            .await;
        assert!(result.is_err(), "get after delete marker should fail");

        // GET and HEAD must distinguish a current delete marker from a missing key.
        // An explicitly selected marker is 405 and also includes Last-Modified.
        let http = Client::new();
        let marker_id = del.version_id().expect("delete marker version");
        let url = format!("{}/{bucket}/to-delete.txt", endpoint_url());
        for method in [Method::GET, Method::HEAD] {
            for explicit_version in [false, true] {
                let mut request_url = Url::parse(&url).expect("object URL");
                if explicit_version {
                    request_url
                        .query_pairs_mut()
                        .append_pair("versionId", marker_id);
                }
                let response = http
                    .request(method.clone(), request_url)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .expect("delete marker response");
                assert_eq!(
                    response.status().as_u16(),
                    if explicit_version { 405 } else { 404 }
                );
                assert_eq!(response.headers()["x-amz-delete-marker"], "true");
                assert_eq!(response.headers()["x-amz-version-id"], marker_id);
                if explicit_version {
                    assert!(response.headers().contains_key("last-modified"));
                }
                if method == Method::HEAD {
                    assert!(response.bytes().await.expect("HEAD body").is_empty());
                }
            }
        }

        // But versions should show both the object and the delete marker.
        let versions = client
            .list_object_versions()
            .bucket(&bucket)
            .send()
            .await
            .expect("list versions");

        assert!(
            !versions.versions().is_empty(),
            "should have object versions"
        );
        assert!(
            !versions.delete_markers().is_empty(),
            "should have delete markers"
        );

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_suspend_versioning() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "suspend").await;
        enable_versioning(&client, &bucket).await;

        client
            .put_bucket_versioning()
            .bucket(&bucket)
            .versioning_configuration(
                aws_sdk_s3::types::VersioningConfiguration::builder()
                    .status(BucketVersioningStatus::Suspended)
                    .build(),
            )
            .send()
            .await
            .expect("suspend versioning");

        let resp = client
            .get_bucket_versioning()
            .bucket(&bucket)
            .send()
            .await
            .expect("get versioning");

        assert_eq!(resp.status(), Some(&BucketVersioningStatus::Suspended));

        cleanup_bucket(&client, &bucket).await;
    }
}
