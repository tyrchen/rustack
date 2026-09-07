//! Queue URL and ARN generation and parsing.

use rustack_core::settings::advertised_endpoint;

/// Build a queue URL.
///
/// Uses the validated runtime advertised endpoint when installed; otherwise
/// falls back to `http://<host>:<port>/<account-id>/<queue-name>`.
#[must_use]
pub fn queue_url(host: &str, port: u16, account_id: &str, queue_name: &str) -> String {
    queue_url_with_endpoint(advertised_endpoint(), host, port, account_id, queue_name)
}

fn queue_url_with_endpoint(
    endpoint: Option<&str>,
    host: &str,
    port: u16,
    account_id: &str,
    queue_name: &str,
) -> String {
    if let Some(endpoint) = endpoint {
        return format!("{endpoint}/{account_id}/{queue_name}");
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!("http://{host}:{port}/{account_id}/{queue_name}")
}

/// Build a queue ARN.
///
/// Format: `arn:aws:sqs:<region>:<account-id>:<queue-name>`
#[must_use]
pub fn queue_arn(region: &str, account_id: &str, queue_name: &str) -> String {
    format!("arn:aws:sqs:{region}:{account_id}:{queue_name}")
}

/// Extract the queue name from a queue URL.
///
/// Accepts URLs like:
/// - `http://localhost:4566/000000000000/my-queue`
/// - `http://sqs.us-east-1.localhost:4566/000000000000/my-queue`
///
/// Extracts the last path segment as the queue name.
#[must_use]
pub fn extract_queue_name(queue_url: &str) -> Option<&str> {
    let path = queue_url
        .strip_prefix("http://")
        .or_else(|| queue_url.strip_prefix("https://"))
        .unwrap_or(queue_url);

    // Find the path portion after host:port.
    let path = path.find('/').map(|i| &path[i..])?;

    // Split path into segments: /account-id/queue-name
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() >= 2 {
        Some(segments[segments.len() - 1])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_build_queue_url() {
        assert_eq!(
            queue_url("localhost", 4566, "000000000000", "my-queue"),
            "http://localhost:4566/000000000000/my-queue"
        );
    }

    #[test]
    fn test_should_preserve_advertised_scheme_authority_and_ipv6() {
        assert_eq!(
            queue_url_with_endpoint(Some("https://[::1]:8443"), "ignored", 1, "123", "queue"),
            "https://[::1]:8443/123/queue"
        );
        assert_eq!(
            queue_url_with_endpoint(None, "::1", 4567, "123", "queue"),
            "http://[::1]:4567/123/queue"
        );
    }

    #[test]
    fn test_should_build_queue_arn() {
        assert_eq!(
            queue_arn("us-east-1", "000000000000", "my-queue"),
            "arn:aws:sqs:us-east-1:000000000000:my-queue"
        );
    }

    #[test]
    fn test_should_extract_queue_name_from_url() {
        assert_eq!(
            extract_queue_name("http://localhost:4566/000000000000/my-queue"),
            Some("my-queue")
        );
        assert_eq!(
            extract_queue_name("http://sqs.us-east-1.localhost:4566/000000000000/test-queue.fifo"),
            Some("test-queue.fifo")
        );
    }
}
