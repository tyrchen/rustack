//! Pure Wait source resolution; monotonic timers remain execution-actor-owned.

use std::{fmt, time::Duration};

use chrono::{DateTime, Timelike, Utc};
use serde_json::Value;

use crate::{
    AslError, EvaluationContext, Payload, ReferencePath,
    payload::{parse_component, validate_component},
};

const MAX_WAIT_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Compiled source for a single STANDARD Wait state, without transition fields.
#[derive(Clone)]
#[non_exhaustive]
pub struct WaitSource(Source);

#[derive(Clone)]
enum Source {
    Seconds(Duration),
    SecondsPath(ReferencePath),
    Timestamp(DateTime<Utc>),
    TimestampPath(ReferencePath),
}

impl fmt::Debug for WaitSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WaitSource(<redacted>)")
    }
}

impl WaitSource {
    /// Preflight source JSON and compile exactly one Wait source member.
    ///
    /// # Errors
    /// Rejects duplicates/unknown fields, invalid static values and source limits.
    ///
    /// ```
    /// let source = rustack_stepfunctions_core::WaitSource::parse(br#"{"Seconds":5}"#)?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, AslError> {
        let value = parse_component(input)?;
        Self::compile(&value)
    }

    /// Compile a borrowed tree before owning any path or timestamp.
    ///
    /// # Errors
    /// Rejects ambiguous/missing sources and unsupported static seconds/dates.
    ///
    /// ```
    /// let source = rustack_stepfunctions_core::WaitSource::compile(
    ///     &serde_json::json!({"SecondsPath":"$.delay"})
    /// )?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn compile(value: &Value) -> Result<Self, AslError> {
        validate_component(value)?;
        let object = value
            .as_object()
            .filter(|v| v.len() == 1)
            .ok_or(AslError::InvalidWait)?;
        let (field, value) = object.iter().next().ok_or(AslError::InvalidWait)?;
        let source = match field.as_str() {
            "Seconds" => Source::Seconds(seconds(value).ok_or(AslError::InvalidWait)?),
            "Timestamp" => Source::Timestamp(
                value
                    .as_str()
                    .and_then(timestamp)
                    .ok_or(AslError::InvalidWait)?,
            ),
            "SecondsPath" => {
                Source::SecondsPath(value.as_str().ok_or(AslError::InvalidWait)?.parse()?)
            }
            "TimestampPath" => {
                Source::TimestampPath(value.as_str().ok_or(AslError::InvalidWait)?.parse()?)
            }
            _ => return Err(AslError::InvalidWait),
        };
        Ok(Self(source))
    }

    /// Resolve once using the actor's supplied wall-time sample; never sleeps.
    ///
    /// The actor adds the returned relative duration to its paired monotonic
    /// sample with checked arithmetic and retains independent execution expiry.
    ///
    /// # Errors
    /// Missing dynamic paths, invalid dynamic values and waits over365 days fail.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{WaitSource,Payload,EvaluationContext};
    /// let data=Payload::parse(b"{}")?;
    /// let source=WaitSource::parse(br#"{"Seconds":5}"#)?;
    /// assert_eq!(source.resolve(&data,&EvaluationContext::from(&data),chrono::Utc::now())?,
    ///            std::time::Duration::from_secs(5));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn resolve(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        now: DateTime<Utc>,
    ) -> Result<Duration, AslError> {
        let target = match &self.0 {
            Source::Seconds(duration) => return Ok(*duration),
            Source::SecondsPath(path) => {
                return seconds(path.lookup(data, context).ok_or(AslError::Missing)?)
                    .ok_or(AslError::WaitValue);
            }
            Source::Timestamp(target) => *target,
            Source::TimestampPath(path) => path
                .lookup(data, context)
                .ok_or(AslError::Missing)?
                .as_str()
                .and_then(timestamp)
                .ok_or(AslError::WaitValue)?,
        };
        if now.nanosecond() >= 1_000_000_000 {
            return Err(AslError::WaitValue);
        }
        if target <= now {
            return Ok(Duration::ZERO);
        }
        let duration = target
            .signed_duration_since(now)
            .to_std()
            .map_err(|_| AslError::WaitValue)?;
        if duration > Duration::from_secs(MAX_WAIT_SECONDS) {
            Err(AslError::WaitValue)
        } else {
            Ok(duration)
        }
    }
}

fn seconds(value: &Value) -> Option<Duration> {
    value
        .as_u64()
        .filter(|n| *n <= MAX_WAIT_SECONDS)
        .map(Duration::from_secs)
}

fn timestamp(value: &str) -> Option<DateTime<Utc>> {
    if value.as_bytes().get(10) != Some(&b'T') || value.ends_with('z') {
        return None;
    }
    let parsed = DateTime::parse_from_rfc3339(value).ok()?;
    if parsed.nanosecond() >= 1_000_000_000 {
        return None;
    }
    parsed
        .with_nanosecond(0)
        .map(|date| date.with_timezone(&Utc))
}

#[cfg(test)]
mod tests;
