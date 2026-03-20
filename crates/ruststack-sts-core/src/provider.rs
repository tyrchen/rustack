//! Main STS provider implementing all operations.

use std::sync::Arc;

use chrono::Utc;
use ruststack_sts_http::request::{
    get_optional_param, get_required_param, parse_policy_arns, parse_session_tags,
    parse_transitive_tag_keys,
};
use ruststack_sts_model::error::StsError;
use ruststack_sts_model::output::{
    AssumeRoleResponse, AssumeRoleWithSAMLResponse, AssumeRoleWithWebIdentityResponse,
    DecodeAuthorizationMessageResponse, GetAccessKeyInfoResponse, GetCallerIdentityResponse,
    GetFederationTokenResponse, GetSessionTokenResponse,
};
use ruststack_sts_model::types::{AssumedRoleUser, Credentials, FederatedUser};

use crate::config::StsConfig;
use crate::identity::CallerIdentity;
use crate::keygen::{
    CredentialGenerator, account_id_from_access_key, generate_federated_user_id, generate_role_id,
};
use crate::session::SessionRecord;
use crate::state::{CredentialRecord, StsState};
use crate::validation::{
    parse_role_arn, validate_federated_name, validate_role_arn, validate_session_name,
};

/// Main STS provider implementing all operations.
#[derive(Debug)]
pub struct RustStackSts {
    pub(crate) state: Arc<StsState>,
    pub(crate) config: Arc<StsConfig>,
}

impl RustStackSts {
    /// Create a new STS provider.
    #[must_use]
    pub fn new(config: StsConfig) -> Self {
        let state = StsState::new(&config);
        Self {
            state: Arc::new(state),
            config: Arc::new(config),
        }
    }

    /// Handle GetCallerIdentity.
    #[must_use]
    pub fn get_caller_identity(
        &self,
        caller_access_key: Option<&str>,
    ) -> GetCallerIdentityResponse {
        let identity = match caller_access_key {
            Some(key) => self.state.resolve_identity(key),
            None => CallerIdentity::Root {
                account_id: self.config.default_account_id.clone(),
            },
        };

        GetCallerIdentityResponse {
            account: Some(identity.account_id().to_owned()),
            arn: Some(identity.arn()),
            user_id: Some(identity.user_id()),
        }
    }

    /// Handle AssumeRole.
    pub fn assume_role(
        &self,
        caller_access_key: Option<&str>,
        params: &[(String, String)],
    ) -> Result<AssumeRoleResponse, StsError> {
        let role_arn = get_required_param(params, "RoleArn")?;
        validate_role_arn(role_arn)?;

        let session_name = get_required_param(params, "RoleSessionName")?;
        validate_session_name(session_name)?;

        let (account_id, role_name) = parse_role_arn(role_arn)?;

        let duration_seconds = parse_duration_seconds(params, 900, 43200, 3600)?;
        let external_id = get_optional_param(params, "ExternalId").map(str::to_owned);
        let policy = get_optional_param(params, "Policy").map(str::to_owned);
        let source_identity = get_optional_param(params, "SourceIdentity").map(str::to_owned);

        let tags = parse_session_tags(params);
        let transitive_keys = parse_transitive_tag_keys(params);

        let (effective_tags, effective_transitive_keys) = self.state.resolve_session_tags(
            caller_access_key.unwrap_or(""),
            &tags,
            &transitive_keys,
        );

        let cred_gen = CredentialGenerator::new(account_id.clone());
        let creds = cred_gen.generate_temporary();
        let role_id = generate_role_id();

        let now = Utc::now();
        let expiration = now + chrono::Duration::seconds(i64::from(duration_seconds));
        let now_epoch = now.timestamp();

        // Store credential record.
        self.state.credentials.insert(
            creds.access_key_id.clone(),
            CredentialRecord {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: creds.secret_access_key.clone(),
                session_token: Some(creds.session_token.clone()),
                identity: CallerIdentity::AssumedRole {
                    account_id: account_id.clone(),
                    role_name: role_name.clone(),
                    session_name: session_name.to_owned(),
                    role_id: role_id.clone(),
                    session_token: creds.session_token.clone(),
                },
                expiration: Some(expiration.timestamp()),
            },
        );

        // Store session record.
        self.state.sessions.insert(
            creds.session_token.clone(),
            SessionRecord {
                role_arn: role_arn.to_owned(),
                session_name: session_name.to_owned(),
                tags: effective_tags,
                transitive_tag_keys: effective_transitive_keys,
                inherited_transitive_tags: Vec::new(),
                access_key_id: creds.access_key_id.clone(),
                source_identity: source_identity.clone(),
                created_at: now_epoch,
                duration_seconds,
                external_id,
                policy_arns: parse_policy_arns(params),
                policy,
            },
        );

        Ok(AssumeRoleResponse {
            credentials: Some(Credentials {
                access_key_id: creds.access_key_id,
                secret_access_key: creds.secret_access_key,
                session_token: creds.session_token,
                expiration,
            }),
            assumed_role_user: Some(AssumedRoleUser {
                assumed_role_id: format!("{role_id}:{session_name}"),
                arn: format!("arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}"),
            }),
            packed_policy_size: Some(6),
            source_identity,
        })
    }

    /// Handle GetSessionToken.
    pub fn get_session_token(
        &self,
        caller_access_key: Option<&str>,
        params: &[(String, String)],
    ) -> Result<GetSessionTokenResponse, StsError> {
        let identity = match caller_access_key {
            Some(key) => self.state.resolve_identity(key),
            None => CallerIdentity::Root {
                account_id: self.config.default_account_id.clone(),
            },
        };

        let duration_seconds = parse_duration_seconds(params, 900, 129_600, 43200)?;

        let cred_gen = CredentialGenerator::new(identity.account_id().to_owned());
        let creds = cred_gen.generate_temporary();
        let now = Utc::now();
        let expiration = now + chrono::Duration::seconds(i64::from(duration_seconds));

        self.state.credentials.insert(
            creds.access_key_id.clone(),
            CredentialRecord {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: creds.secret_access_key.clone(),
                session_token: Some(creds.session_token.clone()),
                identity,
                expiration: Some(expiration.timestamp()),
            },
        );

        Ok(GetSessionTokenResponse {
            credentials: Some(Credentials {
                access_key_id: creds.access_key_id,
                secret_access_key: creds.secret_access_key,
                session_token: creds.session_token,
                expiration,
            }),
        })
    }

    /// Handle GetAccessKeyInfo.
    pub fn get_access_key_info(
        &self,
        params: &[(String, String)],
    ) -> Result<GetAccessKeyInfoResponse, StsError> {
        let access_key_id = get_required_param(params, "AccessKeyId")?;

        if let Some(record) = self.state.credentials.get(access_key_id) {
            return Ok(GetAccessKeyInfoResponse {
                account: Some(record.identity.account_id().to_owned()),
            });
        }

        let account = account_id_from_access_key(access_key_id)
            .unwrap_or_else(|| self.config.default_account_id.clone());

        Ok(GetAccessKeyInfoResponse {
            account: Some(account),
        })
    }

    /// Handle AssumeRoleWithSAML.
    pub fn assume_role_with_saml(
        &self,
        params: &[(String, String)],
    ) -> Result<AssumeRoleWithSAMLResponse, StsError> {
        let role_arn = get_required_param(params, "RoleArn")?;
        validate_role_arn(role_arn)?;

        let _principal_arn = get_required_param(params, "PrincipalArn")?;
        let _saml_assertion = get_required_param(params, "SAMLAssertion")?;

        let (account_id, role_name) = parse_role_arn(role_arn)?;

        let duration_seconds = parse_duration_seconds(params, 900, 43200, 3600)?;

        let session_name = format!("saml-session-{}", &uuid::Uuid::new_v4().to_string()[..8]);

        let cred_gen = CredentialGenerator::new(account_id.clone());
        let creds = cred_gen.generate_temporary();
        let role_id = generate_role_id();

        let now = Utc::now();
        let expiration = now + chrono::Duration::seconds(i64::from(duration_seconds));

        self.state.credentials.insert(
            creds.access_key_id.clone(),
            CredentialRecord {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: creds.secret_access_key.clone(),
                session_token: Some(creds.session_token.clone()),
                identity: CallerIdentity::AssumedRole {
                    account_id: account_id.clone(),
                    role_name: role_name.clone(),
                    session_name: session_name.clone(),
                    role_id: role_id.clone(),
                    session_token: creds.session_token.clone(),
                },
                expiration: Some(expiration.timestamp()),
            },
        );

        self.state.sessions.insert(
            creds.session_token.clone(),
            SessionRecord {
                role_arn: role_arn.to_owned(),
                session_name: session_name.clone(),
                tags: Vec::new(),
                transitive_tag_keys: Vec::new(),
                inherited_transitive_tags: Vec::new(),
                access_key_id: creds.access_key_id.clone(),
                source_identity: None,
                created_at: now.timestamp(),
                duration_seconds,
                external_id: None,
                policy_arns: parse_policy_arns(params),
                policy: get_optional_param(params, "Policy").map(str::to_owned),
            },
        );

        Ok(AssumeRoleWithSAMLResponse {
            credentials: Some(Credentials {
                access_key_id: creds.access_key_id,
                secret_access_key: creds.secret_access_key,
                session_token: creds.session_token,
                expiration,
            }),
            assumed_role_user: Some(AssumedRoleUser {
                assumed_role_id: format!("{role_id}:{session_name}"),
                arn: format!("arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}"),
            }),
            audience: Some("https://signin.aws.amazon.com/saml".to_owned()),
            issuer: Some("https://idp.example.com".to_owned()),
            subject: Some("user@example.com".to_owned()),
            subject_type: Some("persistent".to_owned()),
            name_qualifier: None,
            packed_policy_size: Some(6),
            source_identity: None,
        })
    }

    /// Handle AssumeRoleWithWebIdentity.
    pub fn assume_role_with_web_identity(
        &self,
        params: &[(String, String)],
    ) -> Result<AssumeRoleWithWebIdentityResponse, StsError> {
        let role_arn = get_required_param(params, "RoleArn")?;
        validate_role_arn(role_arn)?;

        let session_name = get_required_param(params, "RoleSessionName")?;
        validate_session_name(session_name)?;

        let web_identity_token = get_required_param(params, "WebIdentityToken")?;

        let (account_id, role_name) = parse_role_arn(role_arn)?;

        let duration_seconds = parse_duration_seconds(params, 900, 43200, 3600)?;

        // Decode JWT payload (without validation).
        let (subject, audience, issuer) = decode_jwt_claims(web_identity_token);

        let cred_gen = CredentialGenerator::new(account_id.clone());
        let creds = cred_gen.generate_temporary();
        let role_id = generate_role_id();

        let now = Utc::now();
        let expiration = now + chrono::Duration::seconds(i64::from(duration_seconds));

        self.state.credentials.insert(
            creds.access_key_id.clone(),
            CredentialRecord {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: creds.secret_access_key.clone(),
                session_token: Some(creds.session_token.clone()),
                identity: CallerIdentity::AssumedRole {
                    account_id: account_id.clone(),
                    role_name: role_name.clone(),
                    session_name: session_name.to_owned(),
                    role_id: role_id.clone(),
                    session_token: creds.session_token.clone(),
                },
                expiration: Some(expiration.timestamp()),
            },
        );

        self.state.sessions.insert(
            creds.session_token.clone(),
            SessionRecord {
                role_arn: role_arn.to_owned(),
                session_name: session_name.to_owned(),
                tags: Vec::new(),
                transitive_tag_keys: Vec::new(),
                inherited_transitive_tags: Vec::new(),
                access_key_id: creds.access_key_id.clone(),
                source_identity: subject.clone(),
                created_at: now.timestamp(),
                duration_seconds,
                external_id: None,
                policy_arns: parse_policy_arns(params),
                policy: get_optional_param(params, "Policy").map(str::to_owned),
            },
        );

        Ok(AssumeRoleWithWebIdentityResponse {
            credentials: Some(Credentials {
                access_key_id: creds.access_key_id,
                secret_access_key: creds.secret_access_key,
                session_token: creds.session_token,
                expiration,
            }),
            assumed_role_user: Some(AssumedRoleUser {
                assumed_role_id: format!("{role_id}:{session_name}"),
                arn: format!("arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}"),
            }),
            audience,
            provider: issuer,
            subject_from_web_identity_token: subject,
            packed_policy_size: Some(6),
            source_identity: None,
        })
    }

    /// Handle DecodeAuthorizationMessage.
    pub fn decode_authorization_message(
        &self,
        params: &[(String, String)],
    ) -> Result<DecodeAuthorizationMessageResponse, StsError> {
        let _encoded_message = get_required_param(params, "EncodedMessage")?;

        let decoded = serde_json::json!({
            "allowed": false,
            "explicitDeny": false,
            "matchedStatements": { "items": [] },
            "failures": { "items": [] },
            "context": {
                "principal": {
                    "id": "AIDAQWERTYUIOPASDFGHJ",
                    "arn": "arn:aws:iam::000000000000:user/local-user"
                },
                "action": "ec2:RunInstances",
                "resource": "arn:aws:ec2:us-east-1:000000000000:instance/*",
                "conditions": { "items": [] }
            }
        });

        Ok(DecodeAuthorizationMessageResponse {
            decoded_message: Some(decoded.to_string()),
        })
    }

    /// Handle GetFederationToken.
    pub fn get_federation_token(
        &self,
        caller_access_key: Option<&str>,
        params: &[(String, String)],
    ) -> Result<GetFederationTokenResponse, StsError> {
        let name = get_required_param(params, "Name")?;
        validate_federated_name(name)?;

        let duration_seconds = parse_duration_seconds(params, 900, 129_600, 43200)?;

        let caller_identity = match caller_access_key {
            Some(key) => self.state.resolve_identity(key),
            None => CallerIdentity::Root {
                account_id: self.config.default_account_id.clone(),
            },
        };
        let account_id = caller_identity.account_id().to_owned();

        let cred_gen = CredentialGenerator::new(account_id.clone());
        let creds = cred_gen.generate_temporary();
        let federated_user_id = generate_federated_user_id(&account_id, name);

        let now = Utc::now();
        let expiration = now + chrono::Duration::seconds(i64::from(duration_seconds));

        self.state.credentials.insert(
            creds.access_key_id.clone(),
            CredentialRecord {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: creds.secret_access_key.clone(),
                session_token: Some(creds.session_token.clone()),
                identity: CallerIdentity::FederatedUser {
                    account_id: account_id.clone(),
                    federated_user_name: name.to_owned(),
                    federated_user_id: federated_user_id.clone(),
                },
                expiration: Some(expiration.timestamp()),
            },
        );

        Ok(GetFederationTokenResponse {
            credentials: Some(Credentials {
                access_key_id: creds.access_key_id,
                secret_access_key: creds.secret_access_key,
                session_token: creds.session_token,
                expiration,
            }),
            federated_user: Some(FederatedUser {
                arn: format!("arn:aws:sts::{account_id}:federated-user/{name}"),
                federated_user_id,
            }),
            packed_policy_size: Some(6),
        })
    }
}

/// Decode JWT claims without signature verification.
///
/// Returns (subject, audience, issuer) extracted from the JWT payload.
fn decode_jwt_claims(token: &str) -> (Option<String>, Option<String>, Option<String>) {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return (None, None, None);
    }

    let Ok(payload) = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        parts[1].as_bytes(),
    ) else {
        return (None, None, None);
    };

    let claims: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };

    let subject = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let audience = claims
        .get("aud")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let issuer = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    (subject, audience, issuer)
}

/// Parse and validate the DurationSeconds parameter.
fn parse_duration_seconds(
    params: &[(String, String)],
    min: i32,
    max: i32,
    default: i32,
) -> Result<i32, StsError> {
    match get_optional_param(params, "DurationSeconds") {
        Some(s) => {
            let value: i32 = s.parse().map_err(|_| {
                StsError::invalid_parameter_value(format!("Invalid value for DurationSeconds: {s}"))
            })?;
            if value < min || value > max {
                return Err(StsError::invalid_parameter_value(format!(
                    "DurationSeconds must be between {min} and {max}, got {value}"
                )));
            }
            Ok(value)
        }
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> RustStackSts {
        RustStackSts::new(StsConfig::default())
    }

    #[test]
    fn test_should_get_caller_identity_root() {
        let provider = make_provider();
        let result = provider.get_caller_identity(Some("test"));
        assert_eq!(result.account.as_deref(), Some("000000000000"));
        assert_eq!(
            result.arn.as_deref(),
            Some("arn:aws:iam::000000000000:root")
        );
        assert_eq!(result.user_id.as_deref(), Some("000000000000"));
    }

    #[test]
    fn test_should_get_caller_identity_unknown_key() {
        let provider = make_provider();
        let result = provider.get_caller_identity(Some("AKIAUNKNOWNKEY123456"));
        assert_eq!(result.account.as_deref(), Some("000000000000"));
    }

    #[test]
    fn test_should_assume_role() {
        let provider = make_provider();
        let params = vec![
            (
                "RoleArn".to_owned(),
                "arn:aws:iam::123456789012:role/TestRole".to_owned(),
            ),
            ("RoleSessionName".to_owned(), "test-session".to_owned()),
        ];
        let result = provider.assume_role(Some("test"), &params).unwrap();
        let creds = result.credentials.unwrap();
        assert!(creds.access_key_id.starts_with("ASIA"));
        assert!(!creds.secret_access_key.is_empty());
        assert!(!creds.session_token.is_empty());

        let assumed = result.assumed_role_user.unwrap();
        assert!(assumed.arn.contains("assumed-role/TestRole/test-session"));
    }

    #[test]
    fn test_should_reject_invalid_role_arn() {
        let provider = make_provider();
        let params = vec![
            ("RoleArn".to_owned(), "not-a-valid-arn".to_owned()),
            ("RoleSessionName".to_owned(), "test".to_owned()),
        ];
        assert!(provider.assume_role(Some("test"), &params).is_err());
    }

    #[test]
    fn test_should_reject_invalid_session_name() {
        let provider = make_provider();
        let params = vec![
            (
                "RoleArn".to_owned(),
                "arn:aws:iam::123456789012:role/R".to_owned(),
            ),
            ("RoleSessionName".to_owned(), "x".to_owned()),
        ];
        assert!(provider.assume_role(Some("test"), &params).is_err());
    }

    #[test]
    fn test_should_get_session_token() {
        let provider = make_provider();
        let params = vec![];
        let result = provider.get_session_token(Some("test"), &params).unwrap();
        let creds = result.credentials.unwrap();
        assert!(creds.access_key_id.starts_with("ASIA"));
    }

    #[test]
    fn test_should_get_access_key_info() {
        let provider = make_provider();
        let params = vec![("AccessKeyId".to_owned(), "test".to_owned())];
        let result = provider.get_access_key_info(&params).unwrap();
        assert_eq!(result.account.as_deref(), Some("000000000000"));
    }

    #[test]
    fn test_should_get_caller_identity_after_assume_role() {
        let provider = make_provider();
        let params = vec![
            (
                "RoleArn".to_owned(),
                "arn:aws:iam::123456789012:role/TestRole".to_owned(),
            ),
            ("RoleSessionName".to_owned(), "my-session".to_owned()),
        ];
        let assume_result = provider.assume_role(Some("test"), &params).unwrap();
        let creds = assume_result.credentials.unwrap();

        let identity = provider.get_caller_identity(Some(&creds.access_key_id));
        assert_eq!(identity.account.as_deref(), Some("123456789012"));
        assert!(
            identity
                .arn
                .as_ref()
                .unwrap()
                .contains("assumed-role/TestRole/my-session")
        );
    }

    #[test]
    fn test_should_decode_authorization_message() {
        let provider = make_provider();
        let params = vec![(
            "EncodedMessage".to_owned(),
            "some-encoded-message".to_owned(),
        )];
        let result = provider.decode_authorization_message(&params).unwrap();
        assert!(result.decoded_message.is_some());
        let msg = result.decoded_message.unwrap();
        assert!(msg.contains("allowed"));
    }

    #[test]
    fn test_should_get_federation_token() {
        let provider = make_provider();
        let params = vec![("Name".to_owned(), "bob".to_owned())];
        let result = provider
            .get_federation_token(Some("test"), &params)
            .unwrap();
        let creds = result.credentials.unwrap();
        assert!(creds.access_key_id.starts_with("ASIA"));
        let fed_user = result.federated_user.unwrap();
        assert!(fed_user.arn.contains("federated-user/bob"));
    }
}
