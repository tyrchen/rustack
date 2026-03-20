//! STS handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_sts_http::body::StsResponseBody;
use ruststack_sts_http::dispatch::StsHandler;
use ruststack_sts_http::request::parse_form_params;
use ruststack_sts_http::response::{XmlWriter, xml_response};
use ruststack_sts_model::error::StsError;
use ruststack_sts_model::operations::StsOperation;
use ruststack_sts_model::types::Credentials;

use crate::provider::RustStackSts;

/// Handler that bridges the HTTP layer to the STS provider.
#[derive(Debug)]
pub struct RustStackStsHandler {
    provider: Arc<RustStackSts>,
}

impl RustStackStsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackSts>) -> Self {
        Self { provider }
    }
}

impl StsHandler for RustStackStsHandler {
    fn handle_operation(
        &self,
        op: StsOperation,
        body: Bytes,
        caller_access_key: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<StsResponseBody>, StsError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(
            async move { dispatch(provider.as_ref(), op, &body, caller_access_key.as_deref()) },
        )
    }
}

/// Dispatch an STS operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackSts,
    op: StsOperation,
    body: &[u8],
    caller_access_key: Option<&str>,
) -> Result<http::Response<StsResponseBody>, StsError> {
    let params = parse_form_params(body);
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        StsOperation::GetCallerIdentity => {
            let output = provider.get_caller_identity(caller_access_key);
            let mut w = XmlWriter::new();
            w.start_response("GetCallerIdentity");
            w.start_result("GetCallerIdentity");
            w.write_optional_element("Arn", output.arn.as_deref());
            w.write_optional_element("UserId", output.user_id.as_deref());
            w.write_optional_element("Account", output.account.as_deref());
            w.end_element("GetCallerIdentityResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetCallerIdentityResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::AssumeRole => {
            let output = provider.assume_role(caller_access_key, &params)?;
            let mut w = XmlWriter::new();
            w.start_response("AssumeRole");
            w.start_result("AssumeRole");
            if let Some(ref creds) = output.credentials {
                write_credentials_xml(&mut w, creds);
            }
            if let Some(ref assumed) = output.assumed_role_user {
                w.raw("<AssumedRoleUser>");
                w.write_element("AssumedRoleId", &assumed.assumed_role_id);
                w.write_element("Arn", &assumed.arn);
                w.raw("</AssumedRoleUser>");
            }
            if let Some(size) = output.packed_policy_size {
                w.write_int_element("PackedPolicySize", size);
            }
            w.write_optional_element("SourceIdentity", output.source_identity.as_deref());
            w.end_element("AssumeRoleResult");
            w.write_response_metadata(&request_id);
            w.end_element("AssumeRoleResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::GetSessionToken => {
            let output = provider.get_session_token(caller_access_key, &params)?;
            let mut w = XmlWriter::new();
            w.start_response("GetSessionToken");
            w.start_result("GetSessionToken");
            if let Some(ref creds) = output.credentials {
                write_credentials_xml(&mut w, creds);
            }
            w.end_element("GetSessionTokenResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetSessionTokenResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::GetAccessKeyInfo => {
            let output = provider.get_access_key_info(&params)?;
            let mut w = XmlWriter::new();
            w.start_response("GetAccessKeyInfo");
            w.start_result("GetAccessKeyInfo");
            w.write_optional_element("Account", output.account.as_deref());
            w.end_element("GetAccessKeyInfoResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetAccessKeyInfoResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::AssumeRoleWithSAML => {
            let output = provider.assume_role_with_saml(&params)?;
            let mut w = XmlWriter::new();
            w.start_response("AssumeRoleWithSAML");
            w.start_result("AssumeRoleWithSAML");
            if let Some(ref creds) = output.credentials {
                write_credentials_xml(&mut w, creds);
            }
            if let Some(ref assumed) = output.assumed_role_user {
                w.raw("<AssumedRoleUser>");
                w.write_element("AssumedRoleId", &assumed.assumed_role_id);
                w.write_element("Arn", &assumed.arn);
                w.raw("</AssumedRoleUser>");
            }
            w.write_optional_element("Audience", output.audience.as_deref());
            w.write_optional_element("Issuer", output.issuer.as_deref());
            w.write_optional_element("Subject", output.subject.as_deref());
            w.write_optional_element("SubjectType", output.subject_type.as_deref());
            if let Some(size) = output.packed_policy_size {
                w.write_int_element("PackedPolicySize", size);
            }
            w.end_element("AssumeRoleWithSAMLResult");
            w.write_response_metadata(&request_id);
            w.end_element("AssumeRoleWithSAMLResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::AssumeRoleWithWebIdentity => {
            let output = provider.assume_role_with_web_identity(&params)?;
            let mut w = XmlWriter::new();
            w.start_response("AssumeRoleWithWebIdentity");
            w.start_result("AssumeRoleWithWebIdentity");
            if let Some(ref creds) = output.credentials {
                write_credentials_xml(&mut w, creds);
            }
            if let Some(ref assumed) = output.assumed_role_user {
                w.raw("<AssumedRoleUser>");
                w.write_element("AssumedRoleId", &assumed.assumed_role_id);
                w.write_element("Arn", &assumed.arn);
                w.raw("</AssumedRoleUser>");
            }
            w.write_optional_element("Audience", output.audience.as_deref());
            w.write_optional_element("Provider", output.provider.as_deref());
            w.write_optional_element(
                "SubjectFromWebIdentityToken",
                output.subject_from_web_identity_token.as_deref(),
            );
            if let Some(size) = output.packed_policy_size {
                w.write_int_element("PackedPolicySize", size);
            }
            w.end_element("AssumeRoleWithWebIdentityResult");
            w.write_response_metadata(&request_id);
            w.end_element("AssumeRoleWithWebIdentityResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::DecodeAuthorizationMessage => {
            let output = provider.decode_authorization_message(&params)?;
            let mut w = XmlWriter::new();
            w.start_response("DecodeAuthorizationMessage");
            w.start_result("DecodeAuthorizationMessage");
            w.write_optional_element("DecodedMessage", output.decoded_message.as_deref());
            w.end_element("DecodeAuthorizationMessageResult");
            w.write_response_metadata(&request_id);
            w.end_element("DecodeAuthorizationMessageResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        StsOperation::GetFederationToken => {
            let output = provider.get_federation_token(caller_access_key, &params)?;
            let mut w = XmlWriter::new();
            w.start_response("GetFederationToken");
            w.start_result("GetFederationToken");
            if let Some(ref creds) = output.credentials {
                write_credentials_xml(&mut w, creds);
            }
            if let Some(ref fed_user) = output.federated_user {
                w.raw("<FederatedUser>");
                w.write_element("FederatedUserId", &fed_user.federated_user_id);
                w.write_element("Arn", &fed_user.arn);
                w.raw("</FederatedUser>");
            }
            if let Some(size) = output.packed_policy_size {
                w.write_int_element("PackedPolicySize", size);
            }
            w.end_element("GetFederationTokenResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetFederationTokenResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }
    }
}

/// Write the `<Credentials>` XML block.
fn write_credentials_xml(w: &mut XmlWriter, creds: &Credentials) {
    w.raw("<Credentials>");
    w.write_element("AccessKeyId", &creds.access_key_id);
    w.write_element("SecretAccessKey", &creds.secret_access_key);
    w.write_element("SessionToken", &creds.session_token);
    w.write_element("Expiration", &creds.expiration.to_rfc3339());
    w.raw("</Credentials>");
}
