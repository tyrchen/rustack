//! Bounded RFC6901 diagnostic locations and retained redacted error sources.

use std::fmt;

use rustack_core::json::JsonError;
use thiserror::Error;

use crate::AslError;

/// Severity of a stable local compiler diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticSeverity {
    /// The definition cannot be installed.
    Error,
    /// Valid definition with a nonfatal structural issue.
    Warning,
}

/// Bounded public diagnostic; Debug never projects source identifiers.
#[derive(Clone)]
#[non_exhaustive]
pub struct Diagnostic {
    severity: DiagnosticSeverity,
    code: &'static str,
    message: &'static str,
    location: String,
}
impl fmt::Debug for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Diagnostic(<redacted>)")
    }
}
impl Diagnostic {
    read!(Diagnostic,severity,DiagnosticSeverity,s=>s.severity,"Read severity.");
    read!(Diagnostic,code,&'static str,s=>s.code,"Read a stable Rustack diagnostic code.");
    read!(Diagnostic,message,&'static str,s=>s.message,"Read the static bounded message.");
    read!(Diagnostic,location,&str,s=>s.location.as_str(),"Read the bounded RFC6901 pointer; empty means root.");
}

#[derive(Error)]
enum Source {
    #[error("invalid definition JSON")]
    Json(#[from] JsonError),
    #[error("invalid compiled component")]
    Asl(#[from] AslError),
}
impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Failed compilation with one diagnostic and an optional typed source.
#[derive(Error)]
#[error("ASL definition compilation failed")]
#[non_exhaustive]
pub struct CompileError {
    diagnostic: Diagnostic,
    #[source]
    source: Option<Source>,
}
impl fmt::Debug for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl CompileError {
    read!(CompileError,diagnostic,&Diagnostic,s=>&s.diagnostic,"Read the diagnostic without formatting its source.");
    pub(super) fn json(location: &Location, error: JsonError) -> Self {
        let limited = matches!(
            error,
            JsonError::Bytes
                | JsonError::Depth
                | JsonError::Members
                | JsonError::String
                | JsonError::Key
        );
        let mut failure = if limited {
            location.error(
                Code::LimitExceeded,
                "definition limits: 1MiB, 64 JSON levels, 65536 members, 1MiB strings, 256KiB keys",
            )
        } else {
            location.error(Code::InvalidJson, "invalid or duplicate definition JSON")
        };
        failure.source = Some(Source::Json(error));
        failure
    }
    pub(super) fn asl(location: &Location, error: AslError) -> Self {
        let message = match &error {
            AslError::InvalidTemplate => {
                "invalid template shape/collision or limit (32 containers, 4096 compiled nodes)"
            }
            AslError::InvalidExpression => {
                "invalid expression or limit (4096 bytes, 1024 nodes, 10 calls, 128 arguments)"
            }
            AslError::InvalidPredicate => {
                "invalid Choice predicate or limit (32 levels, 1024 nodes, 128 children)"
            }
            AslError::InvalidPath | AslError::InvalidReference | AslError::InvalidResultTarget => {
                "invalid path or limit (1024 bytes, 128 segments)"
            }
            AslError::InvalidWait => {
                "invalid STANDARD Wait source (integer seconds at most 31536000)"
            }
            AslError::Json(_) => {
                "invalid component JSON (1MiB, 64 levels, 4096 members, 256KiB strings/keys)"
            }
            AslError::DataLimit => "compiled component exceeds its encoded JSON budget",
            _ => "invalid compiled state component",
        };
        let mut failure = location.error(Code::InvalidComponent, message);
        failure.source = Some(Source::Asl(error));
        failure
    }
}

pub(super) struct Location(String);
impl Location {
    pub(super) fn root() -> Self {
        Self(String::new())
    }
    pub(super) fn field(&self, token: &str) -> Result<Self, CompileError> {
        let extra = token.bytes().filter(|b| matches!(b, b'~' | b'/')).count();
        let length = self
            .0
            .len()
            .saturating_add(1)
            .saturating_add(token.len())
            .saturating_add(extra);
        if length > 4096 {
            return Err(self.error(Code::LimitExceeded, "diagnostic location limit exceeded"));
        }
        let mut value = String::with_capacity(length);
        value.push_str(&self.0);
        value.push('/');
        value.push_str(&token.replace('~', "~0").replace('/', "~1"));
        Ok(Self(value))
    }
    pub(super) fn warning(&self, code: Code, message: &'static str) -> Diagnostic {
        self.diagnostic(DiagnosticSeverity::Warning, code, message)
    }
    fn diagnostic(
        &self,
        severity: DiagnosticSeverity,
        code: Code,
        message: &'static str,
    ) -> Diagnostic {
        Diagnostic {
            severity,
            code: code.as_str(),
            message,
            location: self.0.clone(),
        }
    }
    pub(super) fn error(&self, code: Code, message: &'static str) -> CompileError {
        CompileError {
            diagnostic: self.diagnostic(DiagnosticSeverity::Error, code, message),
            source: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Code {
    InvalidJson,
    InvalidComponent,
    InvalidSchema,
    MissingField,
    UnsupportedField,
    LimitExceeded,
    InvalidStateName,
    MissingTransitionTarget,
    ProfileUnavailable,
    DependencyDisabled,
    InvalidResource,
    NoReachableTerminal,
    UnreachableState,
}
impl Code {
    fn as_str(self) -> &'static str {
        match self {
            Self::InvalidJson => "RUSTACK_INVALID_JSON",
            Self::InvalidComponent => "RUSTACK_INVALID_COMPONENT",
            Self::InvalidSchema => "RUSTACK_INVALID_SCHEMA",
            Self::MissingField => "RUSTACK_MISSING_FIELD",
            Self::UnsupportedField => "RUSTACK_UNSUPPORTED_FIELD",
            Self::LimitExceeded => "RUSTACK_LIMIT_EXCEEDED",
            Self::InvalidStateName => "RUSTACK_INVALID_STATE_NAME",
            Self::MissingTransitionTarget => "RUSTACK_MISSING_TRANSITION_TARGET",
            Self::ProfileUnavailable => "RUSTACK_PROFILE_UNAVAILABLE",
            Self::DependencyDisabled => "RUSTACK_DEPENDENCY_DISABLED",
            Self::InvalidResource => "RUSTACK_INVALID_RESOURCE",
            Self::NoReachableTerminal => "RUSTACK_NO_REACHABLE_TERMINAL",
            Self::UnreachableState => "RUSTACK_UNREACHABLE_STATE",
        }
    }
}
