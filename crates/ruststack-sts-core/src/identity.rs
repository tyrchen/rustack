//! Caller identity types and resolution.

/// The identity of a caller, resolved from their access key.
#[derive(Debug, Clone)]
pub enum CallerIdentity {
    /// Root account credentials.
    Root {
        /// Account ID (e.g., "000000000000").
        account_id: String,
    },
    /// IAM user credentials.
    User {
        /// Account ID.
        account_id: String,
        /// User name.
        user_name: String,
        /// User ID (unique identifier).
        user_id: String,
    },
    /// Assumed role session.
    AssumedRole {
        /// Account ID.
        account_id: String,
        /// Role name (extracted from role ARN).
        role_name: String,
        /// Session name (provided by caller in AssumeRole).
        session_name: String,
        /// Role session ID (unique identifier, e.g., "AROAQWERTYUIOPASDFGHJ").
        role_id: String,
        /// Session token reference for tag lookup.
        session_token: String,
    },
    /// Federated user session.
    FederatedUser {
        /// Account ID.
        account_id: String,
        /// Federated user name.
        federated_user_name: String,
        /// Federated user ID.
        federated_user_id: String,
    },
}

impl CallerIdentity {
    /// Return the account ID for this identity.
    #[must_use]
    pub fn account_id(&self) -> &str {
        match self {
            Self::Root { account_id }
            | Self::User { account_id, .. }
            | Self::AssumedRole { account_id, .. }
            | Self::FederatedUser { account_id, .. } => account_id,
        }
    }

    /// Return the ARN for this identity.
    #[must_use]
    pub fn arn(&self) -> String {
        match self {
            Self::Root { account_id } => {
                format!("arn:aws:iam::{account_id}:root")
            }
            Self::User {
                account_id,
                user_name,
                ..
            } => {
                format!("arn:aws:iam::{account_id}:user/{user_name}")
            }
            Self::AssumedRole {
                account_id,
                role_name,
                session_name,
                ..
            } => {
                format!("arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}")
            }
            Self::FederatedUser {
                account_id,
                federated_user_name,
                ..
            } => {
                format!("arn:aws:sts::{account_id}:federated-user/{federated_user_name}")
            }
        }
    }

    /// Return the user ID for this identity.
    #[must_use]
    pub fn user_id(&self) -> String {
        match self {
            Self::Root { account_id } => account_id.clone(),
            Self::User { user_id, .. } => user_id.clone(),
            Self::AssumedRole {
                role_id,
                session_name,
                ..
            } => {
                format!("{role_id}:{session_name}")
            }
            Self::FederatedUser {
                account_id,
                federated_user_name,
                ..
            } => {
                format!("{account_id}:{federated_user_name}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_return_root_arn() {
        let id = CallerIdentity::Root {
            account_id: "123456789012".to_owned(),
        };
        assert_eq!(id.arn(), "arn:aws:iam::123456789012:root");
        assert_eq!(id.account_id(), "123456789012");
        assert_eq!(id.user_id(), "123456789012");
    }

    #[test]
    fn test_should_return_assumed_role_arn() {
        let id = CallerIdentity::AssumedRole {
            account_id: "123456789012".to_owned(),
            role_name: "TestRole".to_owned(),
            session_name: "my-session".to_owned(),
            role_id: "AROA1234567890EXAMPLE".to_owned(),
            session_token: "token".to_owned(),
        };
        assert_eq!(
            id.arn(),
            "arn:aws:sts::123456789012:assumed-role/TestRole/my-session"
        );
        assert_eq!(id.user_id(), "AROA1234567890EXAMPLE:my-session");
    }

    #[test]
    fn test_should_return_federated_user_arn() {
        let id = CallerIdentity::FederatedUser {
            account_id: "123456789012".to_owned(),
            federated_user_name: "bob".to_owned(),
            federated_user_id: "123456789012:bob".to_owned(),
        };
        assert_eq!(id.arn(), "arn:aws:sts::123456789012:federated-user/bob");
    }
}
