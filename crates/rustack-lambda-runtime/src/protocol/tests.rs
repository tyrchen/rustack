use std::{num::NonZeroU64, time::Duration};

use ciborium::Value;
use proptest::{collection, prelude::*};
use rstest::rstest;
use tokio::io::{duplex, empty};

use super::*;

fn payload() -> Payload {
    Bytes::from_static(b"secret-payload\0\xff")
        .try_into()
        .unwrap()
}

fn messages() -> Vec<Message> {
    let attempt = AttemptId::default();
    vec![
        Message::Hello {
            architecture: Architecture::Arm64,
        },
        Message::Hello {
            architecture: Architecture::X86_64,
        },
        Message::Ready,
        Message::BootstrapReady,
        Message::Invoke {
            attempt,
            deadline_unix_ms: NonZeroU64::new(1_800_000_000_000).unwrap(),
            handler_timeout_ms: None,
            function_arn: "arn:aws:lambda:us-east-1:000000000000:function:echo:prod"
                .parse()
                .unwrap(),
            trace_id: "Root=1-00000000-000000000000000000000000;Sampled=0"
                .parse()
                .unwrap(),
            payload: payload(),
        },
        Message::InvocationResult {
            attempt,
            outcome: Outcome::Success,
            payload: payload(),
        },
        Message::InvocationResult {
            attempt,
            outcome: Outcome::FunctionError,
            payload: payload(),
        },
        Message::InitError { payload: payload() },
        Message::Cancel { attempt },
        Message::Close,
        Message::Ack(Acknowledgement::Close),
        Message::Ack(Acknowledgement::Cancel(attempt)),
        Message::InvocationTimedOut { attempt },
    ]
}

fn body(value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).unwrap();
    bytes
}

fn value(message: Message) -> Value {
    let encoded = encode(&Frame::new(EnvironmentId::default(), message)).unwrap();
    ciborium::from_reader(encoded.get(4..).unwrap()).unwrap()
}

fn field<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    value
        .as_map_mut()
        .unwrap()
        .iter_mut()
        .find_map(|(k, v)| (k.as_text() == Some(key)).then_some(v))
        .unwrap()
}

#[test]
fn test_should_round_trip_every_variant_and_redact_payload_debug() {
    for message in messages() {
        let frame = Frame::new(EnvironmentId::default(), message);
        let bytes = encode(&frame).unwrap();
        let length = u32::from_be_bytes(bytes.get(..4).unwrap().try_into().unwrap());
        assert_eq!(usize::try_from(length).unwrap(), bytes.len() - 4);
        assert_eq!(decode(bytes.get(4..).unwrap()).unwrap(), frame);
        assert!(!format!("{frame:?}").contains("secret-payload"));
    }
    let encoded = value(Message::InitError { payload: payload() });
    let mut encoded = encoded;
    let data = field(field(&mut encoded, "message"), "data");
    assert!(matches!(field(data, "payload"), Value::Bytes(_)));
}

#[rstest]
#[case("00000000-0000-0000-0000-000000000000")]
#[case("00112233-4455-6677-8899-AABBCCDDEEFF")]
#[case("00112233445566778899aabbccddeeff")]
#[case("00112233-4455-6677-8899-aabbccddeeff\n")]
#[case("not-an-id")]
fn test_should_reject_noncanonical_or_nil_identities(#[case] value: &str) {
    assert!(value.parse::<EnvironmentId>().is_err());
    assert!(value.parse::<AttemptId>().is_err());
}

#[rstest]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo:$LATEST")]
#[case("arn:aws-cn:lambda:cn-north-1:123456789012:function:echo:1")]
#[case("arn:aws-us-gov:lambda:us-gov-west-1:123456789012:function:echo:prod")]
fn test_should_accept_scoped_function_metadata(#[case] value: &str) {
    assert_eq!(value.parse::<FunctionArn>().unwrap().as_ref(), value);
}

#[rstest]
#[case("")]
#[case("arn:aws:s3:us-east-1:000000000000:function:echo")]
#[case("arn:aws:lambda::000000000000:function:echo")]
#[case("arn:aws:lambda:us-east-1:0000:function:echo")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo:")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo:0")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo:18446744073709551616")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo:prod:extra")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:echo\r\nsecret")]
fn test_should_reject_invalid_header_metadata_without_echoing_it(#[case] value: &str) {
    let error = value.parse::<FunctionArn>().unwrap_err();
    assert_eq!(error.to_string(), "invalid control metadata");
    assert!(!format!("{error:?}").contains("secret"));
    assert!("trace\nsecret".parse::<TraceId>().is_err());
    assert!("é".parse::<TraceId>().is_err());
    assert!("x".repeat(257).parse::<TraceId>().is_err());

    let mut packet = self::value(messages().into_iter().nth(4).unwrap());
    *field(field(field(&mut packet, "message"), "data"), "functionArn") = Value::Text(value.into());
    assert!(matches!(
        decode(&body(&packet)),
        Err(ProtocolError::InvalidSchema)
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn test_should_never_panic_on_bounded_malformed_control_input(bytes in collection::vec(any::<u8>(), 0..8192)) {
        if let Ok(frame) = decode(&bytes) {
            let encoded = encode(&frame).unwrap();
            prop_assert_eq!(decode(encoded.get(4..).unwrap()).unwrap(), frame);
        }
    }

    #[test]
    fn test_should_preserve_arbitrary_binary_payloads(bytes in collection::vec(any::<u8>(), 0..16384)) {
        let frame = Frame::new(EnvironmentId::default(), Message::InitError {
            payload: Payload::try_from(Bytes::from(bytes)).unwrap(),
        });
        let encoded = encode(&frame).unwrap();
        prop_assert_eq!(decode(encoded.get(4..).unwrap()).unwrap(), frame);
    }

    #[test]
    fn test_should_handle_mutated_valid_envelopes_without_panicking(position in any::<usize>(), mask in any::<u8>()) {
        let frame = Frame::new(EnvironmentId::default(), Message::InitError { payload: payload() });
        let mut encoded = encode(&frame).unwrap().get(4..).unwrap().to_vec();
        let position = position % encoded.len();
        *encoded.get_mut(position).unwrap() ^= mask;
        if let Ok(frame) = decode(&encoded) {
            let bytes = encode(&frame).unwrap();
            prop_assert_eq!(decode(bytes.get(4..).unwrap()).unwrap(), frame);
        }
    }
}

#[test]
fn test_should_reject_unknown_duplicate_and_mismatched_variant_fields() {
    let frame = value(Message::Ready);
    let mut unknown = frame.clone();
    unknown
        .as_map_mut()
        .unwrap()
        .push((Value::Text("secret-unrecognized".into()), Value::Bool(true)));
    let error = decode(&body(&unknown)).unwrap_err();
    assert!(matches!(error, ProtocolError::InvalidSchema));
    assert!(!format!("{error:?} {error}").contains("secret-unrecognized"));
    let mut duplicate = frame.clone();
    let first = duplicate.as_map().unwrap().first().unwrap().clone();
    duplicate.as_map_mut().unwrap().push(first);
    assert!(matches!(
        decode(&body(&duplicate)),
        Err(ProtocolError::InvalidStructure)
    ));
    let mut unknown_kind = frame.clone();
    *field(field(&mut unknown_kind, "message"), "kind") = Value::Text("secret-variant".into());
    assert!(matches!(
        decode(&body(&unknown_kind)),
        Err(ProtocolError::InvalidSchema)
    ));
    let mut version = frame;
    *field(&mut version, "version") = Value::Integer(1.into());
    assert!(matches!(
        decode(&body(&version)),
        Err(ProtocolError::UnsupportedVersion)
    ));

    let mut ack = value(Message::Ack(Acknowledgement::Cancel(AttemptId::default())));
    *field(field(field(&mut ack, "message"), "data"), "operation") = Value::Text("close".into());
    assert!(decode(&body(&ack)).is_err());
    let data = field(field(&mut ack, "message"), "data");
    data.as_map_mut()
        .unwrap()
        .retain(|(k, _)| k.as_text() != Some("attempt"));
    *field(data, "operation") = Value::Text("cancel".into());
    assert!(decode(&body(&ack)).is_err());

    for mut message in [
        value(Message::InitError { payload: payload() }),
        value(Message::InvocationResult {
            attempt: AttemptId::default(),
            outcome: Outcome::Success,
            payload: payload(),
        }),
    ] {
        let data = field(field(&mut message, "message"), "data");
        let duplicate = data.as_map().unwrap().first().unwrap().clone();
        data.as_map_mut().unwrap().push(duplicate);
        assert!(matches!(
            decode(&body(&message)),
            Err(ProtocolError::InvalidStructure)
        ));
    }
}

#[test]
fn test_should_reject_extra_data_fields_for_every_message_variant_and_zero_deadlines() {
    for message in messages() {
        let mut packet = value(message);
        let message = field(&mut packet, "message");
        if let Some((_, data)) = message
            .as_map_mut()
            .unwrap()
            .iter_mut()
            .find(|(key, _)| key.as_text() == Some("data"))
        {
            data.as_map_mut()
                .unwrap()
                .push((Value::Text("secret-extra".into()), Value::Bool(true)));
        } else {
            message
                .as_map_mut()
                .unwrap()
                .push((Value::Text("data".into()), Value::Bool(true)));
        }
        let error = decode(&body(&packet)).unwrap_err();
        assert!(!format!("{error} {error:?}").contains("secret-extra"));
    }
    let mut packet = value(messages().into_iter().nth(4).unwrap());
    *field(
        field(field(&mut packet, "message"), "data"),
        "deadlineUnixMs",
    ) = Value::Integer(0.into());
    assert!(matches!(
        decode(&body(&packet)),
        Err(ProtocolError::InvalidSchema)
    ));
}

#[test]
fn test_should_bound_payload_metadata_nesting_and_collection_counts_before_deserialization() {
    let maximum = Payload::try_from(Bytes::from(vec![0xff; MAX_PAYLOAD_BYTES])).unwrap();
    let frame = Frame::new(
        EnvironmentId::default(),
        Message::InitError { payload: maximum },
    );
    let encoded = encode(&frame).unwrap();
    assert_eq!(decode(encoded.get(4..).unwrap()).unwrap(), frame);
    assert!(Payload::try_from(Bytes::from(vec![0; MAX_PAYLOAD_BYTES + 1])).is_err());
    let mut too_large = value(Message::InitError { payload: payload() });
    *field(field(field(&mut too_large, "message"), "data"), "payload") =
        Value::Bytes(vec![0; MAX_PAYLOAD_BYTES + 1]);
    assert!(matches!(
        decode(&body(&too_large)),
        Err(ProtocolError::TooLarge)
    ));
    let metadata = Value::Map(
        (0..65)
            .map(|i| (Value::Text(format!("k{i}")), Value::Text("x".repeat(1024))))
            .collect(),
    );
    assert!(matches!(
        preflight::validate(&body(&metadata)),
        Err(ProtocolError::TooLarge)
    ));
    let members = Value::Map(
        (0..129)
            .map(|i| (Value::Text(format!("k{i}")), Value::Bool(true)))
            .collect(),
    );
    assert!(matches!(
        preflight::validate(&body(&members)),
        Err(ProtocolError::TooLarge)
    ));
    let mut nested = Value::Bool(true);
    for _ in 0..17 {
        nested = Value::Map(vec![(Value::Text("x".into()), nested)]);
    }
    assert!(matches!(
        preflight::validate(&body(&nested)),
        Err(ProtocolError::TooLarge)
    ));
    assert!(matches!(
        preflight::validate(&body(&Value::Map(vec![(
            Value::Text("k".repeat(65)),
            Value::Bool(true)
        )]))),
        Err(ProtocolError::TooLarge)
    ));
}

#[rstest]
#[case(&[0x9f, 0xff])]
#[case(&[0xbf, 0xff])]
#[case(&[0x5f, 0x40, 0xff])]
#[case(&[0xc0, 0x00])]
#[case(&[0x20])]
#[case(&[0x80])]
#[case(&[0xf9, 0, 20])]
#[case(&[0xfa, 0, 0, 0, 20])]
#[case(&[0xfb, 0, 0, 0, 0, 0, 0, 0, 20])]
#[case(&[0xf6])]
#[case(&[0xa1, 0x61, b'x', 0x7b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])]
fn test_should_reject_forbidden_cbor_shapes_and_huge_declared_lengths(#[case] bytes: &[u8]) {
    assert!(preflight::validate(bytes).is_err());
}

#[test]
fn test_should_reject_trailing_data_and_payload_arrays_or_text() {
    let frame = Frame::new(EnvironmentId::default(), Message::Ready);
    let mut bytes = encode(&frame).unwrap().get(4..).unwrap().to_vec();
    bytes.push(0x00);
    assert!(matches!(
        decode(&bytes),
        Err(ProtocolError::InvalidStructure)
    ));
    for invalid in [Value::Array(vec![]), Value::Text("base64-secret".into())] {
        let mut message = value(Message::InitError { payload: payload() });
        *field(field(field(&mut message, "message"), "data"), "payload") = invalid;
        assert!(decode(&body(&message)).is_err());
    }
}

#[tokio::test]
async fn test_should_round_trip_fragmented_transport_and_distinguish_clean_and_truncated_eof() {
    let frame = Frame::new(
        EnvironmentId::default(),
        Message::InitError { payload: payload() },
    );
    let encoded = encode(&frame).unwrap();
    let (mut writer, mut reader) = duplex(3);
    let expected = frame.clone();
    let send = tokio::spawn(async move {
        write_frame(&mut writer, &frame, Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
    });
    assert_eq!(
        read_frame(&mut reader, Instant::now() + Duration::from_secs(2))
            .await
            .unwrap()
            .unwrap(),
        expected
    );
    send.await.unwrap();
    assert!(
        read_frame(&mut reader, Instant::now() + Duration::from_secs(2))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        read_frame(&mut empty(), Instant::now() + Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    for length in [1, 2, 3, 4, encoded.len() - 1] {
        let mut truncated = encoded.get(..length).unwrap();
        assert!(
            matches!(read_frame(&mut truncated, Instant::now() + Duration::from_secs(1)).await, Err(ProtocolError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
    }
    let oversized = u32::MAX.to_be_bytes();
    assert!(matches!(
        read_frame(
            &mut oversized.as_slice(),
            Instant::now() + Duration::from_secs(1)
        )
        .await,
        Err(ProtocolError::TooLarge)
    ));
}

#[tokio::test(start_paused = true)]
async fn test_should_enforce_absolute_deadlines_for_partial_reads_and_stalled_writes() {
    let (mut writer, mut reader) = duplex(2);
    writer.write_all(&[0, 0]).await.unwrap();
    assert!(matches!(
        read_frame(&mut reader, Instant::now() + Duration::from_secs(1)).await,
        Err(ProtocolError::Deadline)
    ));
    let frame = Frame::new(EnvironmentId::default(), Message::Ready);
    assert!(matches!(
        write_frame(&mut writer, &frame, Instant::now() + Duration::from_secs(1)).await,
        Err(ProtocolError::Deadline)
    ));
    let encoded = encode(&frame).unwrap();
    let mut unread = encoded.as_ref();
    assert!(matches!(
        read_frame(&mut unread, Instant::now()).await,
        Err(ProtocolError::Deadline)
    ));
    assert_eq!(
        unread,
        encoded.as_ref(),
        "expired reads must not consume a ready frame"
    );
}

#[rstest]
#[case(0)]
#[case(999)]
#[case(1001)]
#[case(900_001)]
#[case(u32::MAX)]
fn test_should_reject_out_of_profile_handler_timeouts_before_session_admission(
    #[case] milliseconds: u32,
) {
    assert!(HandlerTimeout::try_from(milliseconds).is_err());
    let mut invoke = messages().into_iter().nth(4).unwrap();
    if let Message::Invoke {
        handler_timeout_ms, ..
    } = &mut invoke
    {
        *handler_timeout_ms = Some(HandlerTimeout::try_from(1000).unwrap());
    }
    let mut packet = value(invoke);
    *field(
        field(field(&mut packet, "message"), "data"),
        "handlerTimeoutMs",
    ) = Value::Integer(milliseconds.into());
    assert!(matches!(
        decode(&body(&packet)),
        Err(ProtocolError::InvalidSchema)
    ));
}
#[test]
fn test_should_round_trip_handler_budget_timeout_event_and_reject_older_relays() {
    for seconds in [1, 900] {
        let timeout = HandlerTimeout::try_from(Duration::from_secs(seconds)).unwrap();
        assert_eq!(timeout.duration(), Duration::from_secs(seconds));
        let mut invoke = messages().into_iter().nth(4).unwrap();
        if let Message::Invoke {
            handler_timeout_ms, ..
        } = &mut invoke
        {
            *handler_timeout_ms = Some(timeout);
        }
        let packet = value(invoke.clone());
        assert_eq!(decode(&body(&packet)).unwrap().into_message(), invoke);
    }
    for timeout in [
        Duration::ZERO,
        Duration::from_millis(1001),
        Duration::from_secs(901),
        Duration::MAX,
    ] {
        assert!(HandlerTimeout::try_from(timeout).is_err());
    }
    let message = Message::InvocationTimedOut {
        attempt: AttemptId::default(),
    };
    let packet = value(message.clone());
    assert_eq!(decode(&body(&packet)).unwrap().into_message(), message);
    for version in [0, 1, 3, u32::MAX] {
        let mut packet = value(Message::Ready);
        *field(&mut packet, "version") = Value::Integer(version.into());
        assert!(matches!(
            decode(&body(&packet)),
            Err(ProtocolError::UnsupportedVersion)
        ));
    }
    let mut invoke = messages().into_iter().nth(4).unwrap();
    if let Message::Invoke {
        handler_timeout_ms, ..
    } = &mut invoke
    {
        *handler_timeout_ms = Some(HandlerTimeout::try_from(1000).unwrap());
    }
    let mut packet = value(invoke);
    *field(
        field(field(&mut packet, "message"), "data"),
        "handlerTimeoutMs",
    ) = Value::Null;
    assert!(matches!(
        decode(&body(&packet)),
        Err(ProtocolError::InvalidStructure)
    ));
}
