//! Session tag types for STS.

/// A session tag (key-value pair).
#[derive(Debug, Clone)]
pub struct SessionTag {
    /// Tag key.
    pub key: String,
    /// Tag value.
    pub value: String,
}

/// A session created by AssumeRole, AssumeRoleWithSAML, or AssumeRoleWithWebIdentity.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// The role ARN that was assumed.
    pub role_arn: String,
    /// The session name provided by the caller.
    pub session_name: String,
    /// Session tags provided by the caller.
    pub tags: Vec<SessionTag>,
    /// Transitive tag keys: these tags propagate to chained AssumeRole calls.
    pub transitive_tag_keys: Vec<String>,
    /// Tags inherited from the parent session (for chained AssumeRole).
    pub inherited_transitive_tags: Vec<SessionTag>,
    /// The access key ID of the temporary credentials for this session.
    pub access_key_id: String,
    /// Source identity (if provided).
    pub source_identity: Option<String>,
    /// When this session was created (epoch seconds).
    pub created_at: i64,
    /// Duration in seconds (not enforced).
    pub duration_seconds: i32,
    /// External ID (if provided in AssumeRole).
    pub external_id: Option<String>,
    /// Policy ARNs attached to the session.
    pub policy_arns: Vec<String>,
    /// Inline policy JSON.
    pub policy: Option<String>,
}
