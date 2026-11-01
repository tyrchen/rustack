//! Secret-wrapped opaque tokens and hash-only constant-time equality keys.

use std::{
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use secrecy::{ExposeSecret, SecretString, zeroize::Zeroizing};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::CallbackError;

/// Local opaque callback credential, deliberately nonserializable and noncloneable.
#[non_exhaustive]
pub struct CallbackToken(SecretString);
impl fmt::Debug for CallbackToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackToken(<redacted>)")
    }
}
impl FromStr for CallbackToken {
    type Err = CallbackError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate(value)?;
        Ok(Self(SecretString::from(value)))
    }
}
impl TryFrom<String> for CallbackToken {
    type Error = CallbackError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate(&value)?;
        Ok(Self(SecretString::from(value)))
    }
}
fn validate(value: &str) -> Result<(), CallbackError> {
    if value.len() > 2048
        || value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CallbackError::InvalidToken);
    }
    Ok(())
}
impl CallbackToken {
    /// Expose only when building outbound Task context or a callback request.
    ///
    /// ```
    /// # fn send(token:&rustack_stepfunctions_core::CallbackToken) { let secret=token.expose(); }
    /// ```
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    pub(super) fn generate() -> Result<Self, CallbackError> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(CallbackError::Entropy)?;
        Ok(Self(SecretString::from(hex::encode(bytes.as_ref()))))
    }
    pub(super) fn digest(&self) -> TokenHash {
        TokenHash(Sha256::digest(self.expose().as_bytes()).into())
    }
}
#[derive(Clone, Copy)]
pub(super) struct TokenHash([u8; 32]);
impl Hash for TokenHash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
impl PartialEq for TokenHash {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}
impl Eq for TokenHash {}
impl fmt::Debug for TokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenHash(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_move_owned_token_storage_into_redacted_secret() {
        let value = "a".repeat(64);
        let pointer = value.as_ptr();
        let token = CallbackToken::try_from(value).unwrap();
        assert_eq!(token.expose().as_ptr(), pointer);
        assert_eq!(token.expose(), "a".repeat(64));
        assert!(!format!("{token:?}").contains(token.expose()));
    }

    #[rstest::rstest]
    #[case(String::new())]
    #[case("a".repeat(63))]
    #[case("A".repeat(64))]
    #[case("a".repeat(65))]
    #[case("f".repeat(2049))]
    #[test]
    fn test_should_reject_invalid_owned_credential(#[case] value: String) {
        assert!(matches!(
            CallbackToken::try_from(value),
            Err(CallbackError::InvalidToken)
        ));
    }
}
