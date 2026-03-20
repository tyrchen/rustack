//! Shard iterator encoding/decoding.
//!
//! Shard iterators are opaque tokens that encode the stream ARN, shard ID,
//! and position. For local development, we use a simple pipe-delimited format.

use ruststack_dynamodbstreams_model::error::DynamoDBStreamsError;

/// Encodes a shard iterator as an opaque token.
///
/// Format: `{stream_arn}|{shard_id}|{position}`
///
/// Where `position` is the 0-based index into the shard's record buffer.
#[must_use]
pub fn encode_iterator(stream_arn: &str, shard_id: &str, position: u64) -> String {
    format!("{stream_arn}|{shard_id}|{position}")
}

/// Decodes a shard iterator token into its components.
///
/// Returns `(stream_arn, shard_id, position)` or an error if the token
/// is malformed.
///
/// # Errors
///
/// Returns `DynamoDBStreamsError::ExpiredIteratorException` if the token is
/// malformed or contains an invalid position.
pub fn decode_iterator(token: &str) -> Result<(&str, &str, u64), DynamoDBStreamsError> {
    let parts: Vec<&str> = token.splitn(3, '|').collect();
    if parts.len() != 3 {
        return Err(DynamoDBStreamsError::expired_iterator(
            "The shard iterator is expired or invalid.",
        ));
    }

    let position = parts[2].parse::<u64>().map_err(|_| {
        DynamoDBStreamsError::expired_iterator("The shard iterator is expired or invalid.")
    })?;

    Ok((parts[0], parts[1], position))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_roundtrip_iterator() {
        let arn =
            "arn:aws:dynamodb:us-east-1:000000000000:table/Test/stream/2026-01-01T00:00:00.000";
        let shard_id = "shardId-00000000-0000-0000-0000-000000000000";
        let position = 42;

        let token = encode_iterator(arn, shard_id, position);
        let (decoded_arn, decoded_shard, decoded_pos) = decode_iterator(&token).unwrap();

        assert_eq!(decoded_arn, arn);
        assert_eq!(decoded_shard, shard_id);
        assert_eq!(decoded_pos, position);
    }

    #[test]
    fn test_should_error_on_malformed_token() {
        let err = decode_iterator("not-a-valid-token").unwrap_err();
        assert_eq!(
            err.code,
            ruststack_dynamodbstreams_model::error::DynamoDBStreamsErrorCode::ExpiredIteratorException,
        );
    }

    #[test]
    fn test_should_error_on_invalid_position() {
        let err = decode_iterator("arn|shard|notanumber").unwrap_err();
        assert_eq!(
            err.code,
            ruststack_dynamodbstreams_model::error::DynamoDBStreamsErrorCode::ExpiredIteratorException,
        );
    }
}
