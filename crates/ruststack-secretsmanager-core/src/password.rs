//! Random password generation with configurable constraints.

use rand::Rng;

use ruststack_secretsmanager_model::error::{SecretsManagerError, SecretsManagerErrorCode};

/// Default password length.
const DEFAULT_LENGTH: i64 = 32;

/// Minimum password length.
const MIN_LENGTH: i64 = 1;

/// Maximum password length.
const MAX_LENGTH: i64 = 4096;

/// Lowercase ASCII letters.
const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";

/// Uppercase ASCII letters.
const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Digits.
const NUMBERS: &str = "0123456789";

/// Punctuation characters used by Secrets Manager.
const PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

/// Space character.
const SPACE: &str = " ";

/// Generate a random password with the given constraints.
///
/// # Errors
///
/// Returns an error if all character types are excluded or the length is invalid.
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub fn generate_random_password(
    password_length: Option<i64>,
    exclude_characters: Option<&str>,
    exclude_lowercase: bool,
    exclude_uppercase: bool,
    exclude_numbers: bool,
    exclude_punctuation: bool,
    include_space: bool,
    require_each_included_type: bool,
) -> Result<String, SecretsManagerError> {
    let length = password_length.unwrap_or(DEFAULT_LENGTH);

    if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            format!("PasswordLength must be between {MIN_LENGTH} and {MAX_LENGTH}."),
        ));
    }

    let length = usize::try_from(length).map_err(|_| {
        SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "Invalid password length.",
        )
    })?;
    let exclude = exclude_characters.unwrap_or("");

    // Build character pools, filtering out excluded characters.
    let mut pools: Vec<(&str, Vec<char>)> = Vec::new();

    if !exclude_lowercase {
        let chars: Vec<char> = LOWERCASE
            .chars()
            .filter(|c| !exclude.contains(*c))
            .collect();
        if !chars.is_empty() {
            pools.push(("lowercase", chars));
        }
    }
    if !exclude_uppercase {
        let chars: Vec<char> = UPPERCASE
            .chars()
            .filter(|c| !exclude.contains(*c))
            .collect();
        if !chars.is_empty() {
            pools.push(("uppercase", chars));
        }
    }
    if !exclude_numbers {
        let chars: Vec<char> = NUMBERS.chars().filter(|c| !exclude.contains(*c)).collect();
        if !chars.is_empty() {
            pools.push(("numbers", chars));
        }
    }
    if !exclude_punctuation {
        let chars: Vec<char> = PUNCTUATION
            .chars()
            .filter(|c| !exclude.contains(*c))
            .collect();
        if !chars.is_empty() {
            pools.push(("punctuation", chars));
        }
    }
    if include_space {
        let chars: Vec<char> = SPACE.chars().filter(|c| !exclude.contains(*c)).collect();
        if !chars.is_empty() {
            pools.push(("space", chars));
        }
    }

    if pools.is_empty() {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "The password length is longer than the number of valid characters available.",
        ));
    }

    // Build a flat character set for random selection.
    let all_chars: Vec<char> = pools
        .iter()
        .flat_map(|(_, chars)| chars.iter().copied())
        .collect();

    if all_chars.is_empty() {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "The password length is longer than the number of valid characters available.",
        ));
    }

    let mut rng = rand::rng();

    if require_each_included_type && length >= pools.len() {
        // Generate ensuring at least one character from each included type.
        let mut password: Vec<char> = Vec::with_capacity(length);

        // Pick one from each pool.
        for (_, pool_chars) in &pools {
            let idx = rng.random_range(0..pool_chars.len());
            password.push(pool_chars[idx]);
        }

        // Fill the rest from the combined set.
        for _ in pools.len()..length {
            let idx = rng.random_range(0..all_chars.len());
            password.push(all_chars[idx]);
        }

        // Shuffle the password to avoid predictable ordering.
        for i in (1..password.len()).rev() {
            let j = rng.random_range(0..=i);
            password.swap(i, j);
        }

        Ok(password.into_iter().collect())
    } else {
        // Just fill randomly.
        let password: String = (0..length)
            .map(|_| {
                let idx = rng.random_range(0..all_chars.len());
                all_chars[idx]
            })
            .collect();
        Ok(password)
    }
}
