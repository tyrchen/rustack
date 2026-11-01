use rstest::rstest;
use serde_json::{from_str, json, to_string, to_vec};

use super::{EndpointChallenge, EndpointReport, GatewayEndpoint, GatewayInstanceId, GatewayReply};

#[rstest]
#[case("")]
#[case("00000000-0000-0000-0000-000000000000")]
#[case("AAAAAAAA-1111-4111-8111-111111111111")]
#[case("aaaaaaaa111141118111111111111111")]
fn test_should_reject_noncanonical_or_nil_gateway_identity(#[case] raw: &str) {
    assert!(raw.parse::<GatewayInstanceId>().is_err());
}
#[test]
fn test_should_roundtrip_validated_identity_reply_and_challenge_without_debug_values() {
    let id = GatewayInstanceId::generate().unwrap();
    let raw: String = id.into();
    assert_eq!(raw.parse::<GatewayInstanceId>().unwrap(), id);
    let reply = GatewayReply::new(id);
    let text = to_string(&reply).unwrap();
    assert_eq!(from_str::<GatewayReply>(&text).unwrap(), reply);
    assert!(!format!("{id:?}").contains(&raw));
    let endpoint: GatewayEndpoint = "http://private-endpoint.example:4566".parse().unwrap();
    let challenge = EndpointChallenge::new(endpoint, id);
    let bytes = to_vec(&challenge).unwrap();
    let decoded = EndpointChallenge::decode(&bytes).unwrap();
    assert_eq!(decoded.expected(), id);
    assert!(!format!("{decoded:?}").contains("private-endpoint"));
}
#[rstest]
#[case(" http://localhost")]
#[case("http://localhost ")]
#[case("http://@localhost")]
#[case("http://user:password@localhost")]
#[case("http://localhost/a/..")]
#[case("http://localhost/%2e")]
#[case("http://localhost/?x")]
#[case("http://localhost/#x")]
#[case("http://localhost\\")]
#[case("http://localhost:0")]
#[case("ssh://localhost")]
#[case("http:localhost")]
fn test_should_reject_endpoint_rewrites_before_url_normalization(#[case] raw: &str) {
    assert!(raw.parse::<GatewayEndpoint>().is_err());
}
#[test]
fn test_should_reject_report_schema_identity_and_challenge_bounds() {
    let id = GatewayInstanceId::generate().unwrap();
    let id: String = id.into();
    for text in [
        format!("{{\"version\":0,\"instanceId\":\"{id}\"}}"),
        format!("{{\"version\":1,\"instanceId\":\"{id}\",\"extra\":1}}"),
        format!("{{\"version\":1,\"instanceId\":\"{id}\",\"instanceId\":\"{id}\"}}"),
        format!("{{\"version\":1,\"instanceId\":\"{id}\"}} trailing"),
    ] {
        assert!(from_str::<GatewayReply>(&text).is_err());
    }
    assert!(EndpointChallenge::decode(&vec![b' '; 4097]).is_err());
    assert!(
        EndpointChallenge::decode(
            to_vec(&json!({"version":1,"endpoint":"http://localhost/path","instanceId":id}))
                .unwrap()
                .as_slice()
        )
        .is_err()
    );
    assert!(from_str::<EndpointReport>(r#"{"version":2,"attempt":"00000000-0000-0000-0000-000000000000","architecture":"arm64","instanceId":"00000000-0000-0000-0000-000000000000"}"#).is_err());
}
