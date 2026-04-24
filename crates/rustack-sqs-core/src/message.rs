//! Message types and MD5 computation.

use std::collections::HashMap;

use md5::{Digest, Md5};
use rustack_sqs_model::types::MessageAttributeValue;

/// Internal representation of a queue message.
#[derive(Debug, Clone)]
pub struct QueueMessage {
    /// Unique message identifier (UUID).
    pub message_id: String,
    /// Message body (up to 256 KiB).
    pub body: String,
    /// MD5 hex digest of the body.
    pub md5_of_body: String,
    /// User-defined message attributes (up to 10).
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    /// MD5 hex digest of message attributes.
    pub md5_of_message_attributes: Option<String>,
    /// Sender ID (account ID).
    pub sender_id: String,
    /// Timestamp when the message was sent (epoch millis).
    pub sent_timestamp: u64,
    /// Approximate number of times the message has been received.
    pub approximate_receive_count: u32,
    /// Timestamp of the first receive (epoch millis).
    pub approximate_first_receive_timestamp: Option<u64>,
    /// FIFO-only: sequence number.
    pub sequence_number: Option<String>,
    /// FIFO-only: message group ID.
    pub message_group_id: Option<String>,
    /// FIFO-only: message deduplication ID.
    pub message_deduplication_id: Option<String>,
    /// When this message becomes available (for delayed messages).
    pub available_at: tokio::time::Instant,
    /// Per-message delay in seconds (0 = no delay).
    pub delay_seconds: i32,
}

/// An in-flight message with its visibility timeout.
#[derive(Debug)]
pub struct InFlightMessage {
    /// The original message.
    pub message: QueueMessage,
    /// Receipt handle for this delivery.
    pub receipt_handle: String,
    /// When this message becomes visible again.
    pub visible_at: tokio::time::Instant,
}

/// Compute the MD5 hex digest of a message body.
#[must_use]
pub fn md5_of_body(body: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(body.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compute the MD5 hex digest of message attributes following the AWS specification.
///
/// Attributes are sorted by name, then each is encoded as:
///   - `length(name)` as 4-byte big-endian + UTF-8(name)
///   - `length(data_type)` as 4-byte big-endian + UTF-8(data_type)
///   - 1 byte transport type (1=String, 2=Binary)
///   - `length(value)` as 4-byte big-endian + value_bytes
#[must_use]
#[allow(clippy::implicit_hasher)] // Internal function; always called with default HashMap.
#[allow(clippy::cast_possible_truncation)] // AWS binary protocol encodes lengths as u32.
pub fn md5_of_message_attributes(attrs: &HashMap<String, MessageAttributeValue>) -> Option<String> {
    if attrs.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = attrs.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);

    let mut hasher = Md5::new();
    for (name, value) in sorted {
        // Encode name length + name bytes.
        hasher.update((name.len() as u32).to_be_bytes());
        hasher.update(name.as_bytes());
        // Encode data type length + data type bytes.
        hasher.update((value.data_type.len() as u32).to_be_bytes());
        hasher.update(value.data_type.as_bytes());
        // Encode transport type + value.
        if let Some(ref string_value) = value.string_value {
            hasher.update([1u8]); // STRING type
            hasher.update((string_value.len() as u32).to_be_bytes());
            hasher.update(string_value.as_bytes());
        } else if let Some(ref binary_value) = value.binary_value {
            hasher.update([2u8]); // BINARY type
            hasher.update((binary_value.len() as u32).to_be_bytes());
            hasher.update(binary_value);
        }
    }
    Some(hex::encode(hasher.finalize()))
}

/// Generate a receipt handle for a received message.
///
/// The receipt handle encodes the message ID and a random nonce to ensure
/// uniqueness per receive operation.
#[must_use]
pub fn generate_receipt_handle(message_id: &str) -> String {
    use base64::Engine;
    let nonce = uuid::Uuid::new_v4();
    let epoch_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let raw = format!("{message_id}:{epoch_nanos}:{nonce}");
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// Get the current epoch time in milliseconds.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Epoch millis fits in u64 for millennia.
pub fn now_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Get the current epoch time in seconds.
#[must_use]
pub fn now_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_compute_md5_of_body() {
        // "hello" -> 5d41402abc4b2a76b9719d911017c592
        let md5 = md5_of_body("hello");
        assert_eq!(md5, "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn test_should_return_none_for_empty_attributes() {
        let attrs = HashMap::new();
        assert!(md5_of_message_attributes(&attrs).is_none());
    }

    #[test]
    fn test_should_compute_md5_of_single_string_attribute() {
        let mut attrs = HashMap::new();
        attrs.insert(
            "testAttr".to_owned(),
            MessageAttributeValue {
                data_type: "String".to_owned(),
                string_value: Some("testValue".to_owned()),
                binary_value: None,
            },
        );
        let md5 = md5_of_message_attributes(&attrs);
        assert!(md5.is_some());
        // The MD5 should be consistent.
        assert_eq!(md5, md5_of_message_attributes(&attrs));
    }

    #[test]
    fn test_should_generate_unique_receipt_handles() {
        let h1 = generate_receipt_handle("msg-1");
        let h2 = generate_receipt_handle("msg-1");
        assert_ne!(h1, h2, "receipt handles should be unique");
    }
}
