//! Shared executor request/response/config types.

use std::{collections::HashMap, path::PathBuf, str::FromStr, time::Duration};

use bytes::Bytes;

/// Selects which execution backend the provider should construct at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutorBackend {
    /// Legacy echo behavior — no real process or container is started.
    #[default]
    Disabled,
    /// Pick the most appropriate backend per invocation: native when the
    /// runtime + arch + bootstrap allow it, Docker otherwise.
    Auto,
    /// Always native; reject invocations that can't run on the host.
    Native,
    /// Always Docker.
    Docker,
    /// Always Squib microVM execution.
    Squib,
}

impl FromStr for ExecutorBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "noop" => Ok(Self::Disabled),
            "auto" => Ok(Self::Auto),
            "native" | "process" => Ok(Self::Native),
            "docker" | "container" => Ok(Self::Docker),
            "squib" | "microvm" => Ok(Self::Squib),
            other => Err(format!("unknown LAMBDA_EXECUTOR value: {other}")),
        }
    }
}

/// Deployment package kind, mirroring the Lambda API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageType {
    /// Unzipped function code on the local filesystem.
    Zip,
    /// Container image referenced by URI.
    Image,
}

impl PackageType {
    /// Parse from the wire string used by the Lambda model.
    #[must_use]
    pub fn from_wire(s: &str) -> Self {
        if s.eq_ignore_ascii_case("Image") {
            Self::Image
        } else {
            Self::Zip
        }
    }
}

/// Everything an executor needs to run one invocation.
#[derive(Debug, Clone)]
pub struct InvokeRequest {
    /// Qualified ARN — used for `Lambda-Runtime-Invoked-Function-Arn`.
    pub function_arn: String,
    /// Bare function name.
    pub function_name: String,
    /// Resolved version string, e.g. `"$LATEST"` or `"3"`.
    pub qualifier: String,
    /// Lambda runtime identifier, e.g. `"provided.al2023"`.
    pub runtime: Option<String>,
    /// Handler string from the function config.
    pub handler: Option<String>,
    /// Architectures the function was created with, e.g. `["x86_64"]`.
    pub architectures: Vec<String>,
    /// Package type — Zip uses `code_root`, Image uses `image_uri`.
    pub package_type: PackageType,
    /// Filesystem path to the unzipped code root (for Zip packages).
    pub code_root: Option<PathBuf>,
    /// Image URI (for Image packages).
    pub image_uri: Option<String>,
    /// User-supplied environment variables.
    pub environment: HashMap<String, String>,
    /// Function timeout. Excess => `Sandbox.Timedout`.
    pub timeout: Duration,
    /// Memory size in MB (used as the container memory cap).
    pub memory_mb: u32,
    /// Raw invocation payload (typically JSON, but not required).
    pub payload: Bytes,
    /// When set, the executor captures the last 4KB of stderr as `log_tail`.
    pub capture_logs: bool,
}

/// Outcome of a successful round-trip with the user code (which may itself
/// have reported a function error).
#[derive(Debug, Clone)]
pub struct InvokeResponse {
    /// HTTP status to mirror back to the caller (typically 200).
    pub status: u16,
    /// Response body produced by the bootstrap.
    pub payload: Bytes,
    /// `Some("Unhandled")` when the bootstrap posted to `/error`.
    pub function_error: Option<String>,
    /// Base64-encoded last 4KB of stderr, if `capture_logs` was set.
    pub log_tail: Option<String>,
    /// What the executor actually invoked (echoes back the qualifier).
    pub executed_version: String,
}

impl InvokeResponse {
    /// Convenience constructor for a plain JSON success response.
    #[must_use]
    pub fn success(payload: Bytes, executed_version: String) -> Self {
        Self {
            status: 200,
            payload,
            function_error: None,
            log_tail: None,
            executed_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_executor_backend_strings() {
        assert_eq!(
            "disabled".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Disabled
        );
        assert_eq!(
            "Auto".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Auto
        );
        assert_eq!(
            "native".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Native
        );
        assert_eq!(
            "DOCKER".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Docker
        );
        assert_eq!(
            "squib".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Squib
        );
        assert_eq!(
            "microvm".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Squib
        );
        assert!("nope".parse::<ExecutorBackend>().is_err());
    }

    #[test]
    fn test_should_parse_package_type_from_wire() {
        assert_eq!(PackageType::from_wire("Image"), PackageType::Image);
        assert_eq!(PackageType::from_wire("image"), PackageType::Image);
        assert_eq!(PackageType::from_wire("Zip"), PackageType::Zip);
        // Anything unrecognized falls back to Zip — matches the provider default.
        assert_eq!(PackageType::from_wire(""), PackageType::Zip);
    }
}
