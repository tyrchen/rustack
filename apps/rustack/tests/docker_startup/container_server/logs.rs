//! D97's raw plus decoded-JSON-token privacy profile, bounded for server logs.

use anyhow::{Context, Result, ensure};
use rustack_core::json::{JsonLimits, from_slice};

pub(super) fn check(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= 65536, "server diagnostic byte cap");
    std::str::from_utf8(bytes).context("invalid server diagnostic encoding")?;
    private(bytes)?;
    let mut start = None;
    let mut escaped = false;
    let mut tokens = 0usize;
    let mut decoded = 0usize;
    for (offset, byte) in bytes.iter().enumerate() {
        let Some(begin) = start else {
            if *byte == b'"' {
                start = Some(offset);
            }
            continue;
        };
        if *byte == b'"' && !escaped {
            tokens = tokens
                .checked_add(1)
                .context("server diagnostic token count")?;
            ensure!(tokens <= 4096, "server diagnostic token cap");
            let raw = bytes
                .get(begin..=offset)
                .context("server diagnostic token range")?;
            let value: String = from_slice(raw, JsonLimits::new(65536, 1, 1, 65536, 256)?)?;
            decoded = decoded
                .checked_add(value.len())
                .context("server decoded diagnostic bytes")?;
            ensure!(decoded <= 65536, "server decoded diagnostic byte cap");
            private(value.as_bytes())?;
            start = None;
        }
        escaped = *byte == b'\\' && !escaped;
    }
    ensure!(start.is_none(), "unclosed server diagnostic JSON token");
    Ok(())
}

fn private(bytes: &[u8]) -> Result<()> {
    ensure!(
        ![b"local-key".as_slice(), b"local-secret"]
            .iter()
            .any(|needle| bytes.windows(needle.len()).any(|window| window == *needle)),
        "server diagnostic credential leak"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_raw_and_json_escaped_keys_and_credentials_without_echo() -> Result<()> {
        for bytes in [
            b"local-key".as_slice(),
            b"local-secret",
            br#"{"\u006cocal-key":"ignored"}"#,
            br#"{"credential":"\u006cocal-\u0073ecret"}"#,
            br#"mixed> {"credential":"\u006cocal-key"}"#,
        ] {
            let error = check(bytes)
                .err()
                .context("escaped diagnostic leak accepted")?;
            assert_eq!(error.to_string(), "server diagnostic credential leak");
        }
        Ok(())
    }
    #[test]
    fn test_should_accept_safe_structured_and_mixed_diagnostics_with_escaped_quotes() -> Result<()>
    {
        check(br#"{"message":"quoted \"safe\" content","details":{"ok":true}}"#)?;
        check(br#"plain> "trailing\\" "safe""#)?;
        check(b"plain safe diagnostic\n")?;
        Ok(())
    }
    #[test]
    fn test_should_fail_closed_on_malformed_or_over_budget_diagnostics() {
        for bytes in [b"log \"unterminated".as_slice(), b"\xff", b"\"\x00\""] {
            assert!(check(bytes).is_err());
        }
        assert!(check(&vec![b'a'; 65537]).is_err());
        assert!(check("\"x\" ".repeat(4097).as_bytes()).is_err());
    }
}
