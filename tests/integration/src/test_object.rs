//! Object CRUD integration tests.

#[cfg(test)]
mod tests {
    use aws_sdk_s3::primitives::ByteStream;
    use bytes::Bytes;

    use crate::{cleanup_bucket, create_test_bucket, s3_client};

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "putget").await;

        let body = b"hello, rustack!";
        client
            .put_object()
            .bucket(&bucket)
            .key("greeting.txt")
            .body(ByteStream::from_static(body))
            .content_type("text/plain")
            .send()
            .await
            .expect("put_object");

        let resp = client
            .get_object()
            .bucket(&bucket)
            .key("greeting.txt")
            .send()
            .await
            .expect("get_object");

        assert_eq!(
            resp.content_type(),
            Some("text/plain"),
            "content_type should match"
        );
        assert_eq!(
            resp.content_length(),
            Some(i64::try_from(body.len()).unwrap())
        );

        let data = resp
            .body
            .collect()
            .await
            .expect("collect body")
            .into_bytes();
        assert_eq!(data.as_ref(), body);

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_head_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "head").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("file.bin")
            .body(ByteStream::from_static(b"binary data"))
            .content_type("application/octet-stream")
            .send()
            .await
            .expect("put_object");

        let resp = client
            .head_object()
            .bucket(&bucket)
            .key("file.bin")
            .send()
            .await
            .expect("head_object");

        assert_eq!(resp.content_length(), Some(11));
        assert_eq!(resp.content_type(), Some("application/octet-stream"),);
        assert!(resp.e_tag().is_some(), "etag should be present");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "del").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("delete-me.txt")
            .body(ByteStream::from_static(b"temp"))
            .send()
            .await
            .expect("put_object");

        client
            .delete_object()
            .bucket(&bucket)
            .key("delete-me.txt")
            .send()
            .await
            .expect("delete_object");

        let result = client
            .get_object()
            .bucket(&bucket)
            .key("delete-me.txt")
            .send()
            .await;
        assert!(result.is_err(), "get after delete should fail");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_objects_batch() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "batch").await;

        for i in 0..5 {
            client
                .put_object()
                .bucket(&bucket)
                .key(format!("file-{i}.txt"))
                .body(ByteStream::from_static(b"data"))
                .send()
                .await
                .expect("put_object");
        }

        let objects: Vec<aws_sdk_s3::types::ObjectIdentifier> = (0..5)
            .map(|i| {
                aws_sdk_s3::types::ObjectIdentifier::builder()
                    .key(format!("file-{i}.txt"))
                    .build()
                    .unwrap()
            })
            .collect();

        let delete = aws_sdk_s3::types::Delete::builder()
            .set_objects(Some(objects))
            .build()
            .unwrap();

        let resp = client
            .delete_objects()
            .bucket(&bucket)
            .delete(delete)
            .send()
            .await
            .expect("delete_objects");

        assert_eq!(resp.deleted().len(), 5, "should delete all 5 objects");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_copy_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "copy").await;

        let body = b"copy me";
        client
            .put_object()
            .bucket(&bucket)
            .key("original.txt")
            .body(ByteStream::from_static(body))
            .send()
            .await
            .expect("put_object");

        client
            .copy_object()
            .bucket(&bucket)
            .key("copied.txt")
            .copy_source(format!("{bucket}/original.txt"))
            .send()
            .await
            .expect("copy_object");

        let resp = client
            .get_object()
            .bucket(&bucket)
            .key("copied.txt")
            .send()
            .await
            .expect("get copied object");

        let data = resp.body.collect().await.expect("collect").into_bytes();
        assert_eq!(data.as_ref(), body);

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_object_with_metadata() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "meta").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("with-meta.txt")
            .body(ByteStream::from_static(b"data"))
            .metadata("x-custom-key", "custom-value")
            .metadata("another-key", "another-value")
            .send()
            .await
            .expect("put_object");

        let resp = client
            .head_object()
            .bucket(&bucket)
            .key("with-meta.txt")
            .send()
            .await
            .expect("head_object");

        let meta = resp.metadata().unwrap();
        assert_eq!(
            meta.get("x-custom-key").map(String::as_str),
            Some("custom-value")
        );
        assert_eq!(
            meta.get("another-key").map(String::as_str),
            Some("another-value")
        );

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_large_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "large").await;

        // 1 MB object.
        let data: Bytes = Bytes::from(vec![0xABu8; 1_048_576]);
        client
            .put_object()
            .bucket(&bucket)
            .key("large.bin")
            .body(ByteStream::from(data.clone()))
            .send()
            .await
            .expect("put large object");

        let resp = client
            .get_object()
            .bucket(&bucket)
            .key("large.bin")
            .send()
            .await
            .expect("get large object");

        assert_eq!(resp.content_length(), Some(1_048_576));

        let got = resp.body.collect().await.expect("collect").into_bytes();
        assert_eq!(got.len(), 1_048_576);
        assert!(got.iter().all(|&b| b == 0xAB));

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_object_range() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "range").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("range.txt")
            .body(ByteStream::from_static(b"0123456789"))
            .send()
            .await
            .expect("put_object");

        let resp = client
            .get_object()
            .bucket(&bucket)
            .key("range.txt")
            .range("bytes=3-6")
            .send()
            .await
            .expect("get_object with range");

        let data = resp.body.collect().await.expect("collect").into_bytes();
        assert_eq!(data.as_ref(), b"3456");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_nonexistent_key_returns_error() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "nokey").await;

        let result = client
            .get_object()
            .bucket(&bucket)
            .key("does-not-exist")
            .send()
            .await;
        assert!(result.is_err());

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_overwrite_object() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "overwrite").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("file.txt")
            .body(ByteStream::from_static(b"version1"))
            .send()
            .await
            .expect("put v1");

        client
            .put_object()
            .bucket(&bucket)
            .key("file.txt")
            .body(ByteStream::from_static(b"version2"))
            .send()
            .await
            .expect("put v2");

        let resp = client
            .get_object()
            .bucket(&bucket)
            .key("file.txt")
            .send()
            .await
            .expect("get");

        let data = resp.body.collect().await.expect("collect").into_bytes();
        assert_eq!(data.as_ref(), b"version2");

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_object_with_tagging() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "tag").await;

        client
            .put_object()
            .bucket(&bucket)
            .key("tagged.txt")
            .body(ByteStream::from_static(b"data"))
            .tagging("env=prod&team=backend")
            .send()
            .await
            .expect("put_object with tagging");

        let resp = client
            .get_object_tagging()
            .bucket(&bucket)
            .key("tagged.txt")
            .send()
            .await
            .expect("get_object_tagging");

        let tags = resp.tag_set();
        assert_eq!(tags.len(), 2);

        let tag_map: std::collections::HashMap<&str, &str> =
            tags.iter().map(|t| (t.key(), t.value())).collect();
        assert_eq!(tag_map.get("env"), Some(&"prod"));
        assert_eq!(tag_map.get("team"), Some(&"backend"));

        cleanup_bucket(&client, &bucket).await;
    }
}
