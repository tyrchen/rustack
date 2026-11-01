//! Incremental pull metadata; peer messages never escape as diagnostics.

use std::fmt;

use rustack_core::json::{JsonLimits, from_slice};
use serde::{Deserialize, de::IgnoredAny};

use crate::docker::DockerError;

pub(in crate::docker) const MAX_TOTAL: usize = 16 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
const MAX_RECORDS: usize = 65_536;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    error: Option<IgnoredAny>,
    error_detail: Option<IgnoredAny>,
}
impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Record(<redacted>)")
    }
}
#[derive(Default)]
pub(in crate::docker) struct Progress {
    line: Vec<u8>,
    bytes: usize,
    records: usize,
}
impl fmt::Debug for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Progress(<redacted>)")
    }
}
impl Progress {
    pub(in crate::docker) fn push(&mut self, bytes: &[u8]) -> Result<(), DockerError> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or(DockerError::TooLarge)?;
        if self.bytes > MAX_TOTAL {
            return Err(DockerError::TooLarge);
        }
        for byte in bytes {
            if *byte == b'\n' {
                if self.line.last() == Some(&b'\r') {
                    self.line.pop();
                }
                if self.line.is_empty() || self.line.len() > MAX_LINE {
                    return Err(DockerError::Protocol);
                }
                if self
                    .line
                    .iter()
                    .copied()
                    .find(|byte| !byte.is_ascii_whitespace())
                    != Some(b'{')
                {
                    return Err(DockerError::Protocol);
                }
                let record: Record =
                    from_slice(&self.line, JsonLimits::new(MAX_LINE, 8, 128, 8192, 256)?)?;
                if record.error.is_some() || record.error_detail.is_some() {
                    return Err(DockerError::PullFailed);
                }
                self.records = self.records.checked_add(1).ok_or(DockerError::TooLarge)?;
                if self.records > MAX_RECORDS {
                    return Err(DockerError::TooLarge);
                }
                self.line.clear();
            } else {
                if self.line.len() > MAX_LINE || (self.line.len() == MAX_LINE && *byte != b'\r') {
                    return Err(DockerError::TooLarge);
                }
                if self.line.len() == self.line.capacity() {
                    let extra = (MAX_LINE + 1)
                        .saturating_sub(self.line.len())
                        .min(16 * 1024);
                    self.line
                        .try_reserve_exact(extra)
                        .map_err(|_| DockerError::Allocation)?;
                }
                self.line.push(*byte);
            }
        }
        Ok(())
    }
    pub(in crate::docker) fn finish(self) -> Result<(), DockerError> {
        if self.records == 0 || !self.line.is_empty() {
            return Err(DockerError::Protocol);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
