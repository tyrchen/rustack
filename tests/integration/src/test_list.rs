//! List objects integration tests.

#[cfg(test)]
mod tests {
    use aws_sdk_s3::primitives::ByteStream;

    use crate::{cleanup_bucket, create_test_bucket, s3_client};

    async fn populate_bucket(client: &aws_sdk_s3::Client, bucket: &str) {
        let keys = [
            "photos/2024/jan/img1.jpg",
            "photos/2024/jan/img2.jpg",
            "photos/2024/feb/img3.jpg",
            "photos/2025/mar/img4.jpg",
            "documents/report.pdf",
            "documents/readme.txt",
            "root.txt",
        ];
        for key in keys {
            client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(ByteStream::from_static(b"x"))
                .send()
                .await
                .unwrap_or_else(|e| panic!("put {key}: {e}"));
        }
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_objects_v2() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "listv2").await;
        populate_bucket(&client, &bucket).await;

        let resp = client
            .list_objects_v2()
            .bucket(&bucket)
            .send()
            .await
            .expect("list_objects_v2");

        assert_eq!(resp.key_count(), Some(7));

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_with_prefix() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "prefix").await;
        populate_bucket(&client, &bucket).await;

        let resp = client
            .list_objects_v2()
            .bucket(&bucket)
            .prefix("photos/2024/")
            .send()
            .await
            .expect("list with prefix");

        assert_eq!(resp.key_count(), Some(3));

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_with_delimiter() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "delim").await;
        populate_bucket(&client, &bucket).await;

        let resp = client
            .list_objects_v2()
            .bucket(&bucket)
            .delimiter("/")
            .send()
            .await
            .expect("list with delimiter");

        // Should have 1 direct key (root.txt) and 2 common prefixes (photos/, documents/).
        let keys: Vec<&str> = resp.contents().iter().filter_map(|o| o.key()).collect();
        assert_eq!(keys, vec!["root.txt"]);

        let prefixes: Vec<&str> = resp
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix())
            .collect();
        assert!(prefixes.contains(&"documents/"));
        assert!(prefixes.contains(&"photos/"));

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_with_prefix_and_delimiter() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "prefdelim").await;
        populate_bucket(&client, &bucket).await;

        let resp = client
            .list_objects_v2()
            .bucket(&bucket)
            .prefix("photos/2024/")
            .delimiter("/")
            .send()
            .await
            .expect("list");

        // No direct keys under photos/2024/, only common prefixes.
        assert_eq!(resp.contents(), []);

        let prefixes: Vec<&str> = resp
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix())
            .collect();
        assert!(prefixes.contains(&"photos/2024/jan/"));
        assert!(prefixes.contains(&"photos/2024/feb/"));

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_paginate_list_objects() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "paginate").await;

        // Create 5 objects.
        for i in 0..5 {
            client
                .put_object()
                .bucket(&bucket)
                .key(format!("key-{i:03}"))
                .body(ByteStream::from_static(b"x"))
                .send()
                .await
                .unwrap_or_else(|e| panic!("put key-{i:03}: {e}"));
        }

        // List with max_keys=2 to force pagination.
        let mut all_keys = Vec::new();
        let mut continuation_token = None;
        loop {
            let mut req = client.list_objects_v2().bucket(&bucket).max_keys(2);
            if let Some(token) = continuation_token.take() {
                req = req.continuation_token(token);
            }

            let resp = req.send().await.expect("list page");
            for obj in resp.contents() {
                if let Some(key) = obj.key() {
                    all_keys.push(key.to_owned());
                }
            }

            if resp.is_truncated() == Some(true) {
                continuation_token = resp.next_continuation_token().map(ToOwned::to_owned);
            } else {
                break;
            }
        }

        assert_eq!(all_keys.len(), 5);
        assert_eq!(
            all_keys,
            vec!["key-000", "key-001", "key-002", "key-003", "key-004"]
        );

        cleanup_bucket(&client, &bucket).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_empty_bucket() {
        let client = s3_client();
        let bucket = create_test_bucket(&client, "empty").await;

        let resp = client
            .list_objects_v2()
            .bucket(&bucket)
            .send()
            .await
            .expect("list empty");

        assert_eq!(resp.key_count(), Some(0));
        assert_eq!(resp.contents(), []);

        cleanup_bucket(&client, &bucket).await;
    }
}
