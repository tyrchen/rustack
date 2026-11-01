//! Original body ownership and canonical budgets, separate from compact metadata.

use std::{collections::HashSet, iter::once};

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{DockerArchiveProfile, validation::invalid};
use crate::{
    artifact::Architecture,
    error::LambdaServiceError,
    function_config::{validate_provided_settings, validate_snapshot_settings},
    provider::LambdaSnapshot,
    resolver::FunctionName,
    storage::{FunctionRecord, VersionRecordSnapshot},
};

fn digit(byte: u8) -> Result<u8, LambdaServiceError> {
    let (base, offset) = match byte {
        b'A'..=b'Z' => (b'A', 0),
        b'a'..=b'z' => (b'a', 26),
        b'0'..=b'9' => (b'0', 52),
        b'+' => return Ok(62),
        b'/' => return Ok(63),
        _ => return Err(invalid()),
    };
    byte.checked_sub(base)
        .and_then(|value| value.checked_add(offset))
        .ok_or_else(invalid)
}
/// Validate without decoding or producing a second owned body.
fn decoded_len(encoded: &str, maximum: usize) -> Result<usize, LambdaServiceError> {
    let bytes = encoded.as_bytes();
    let maximum_encoded = maximum.div_ceil(3).checked_mul(4).ok_or_else(invalid)?;
    if bytes.is_empty() || bytes.len() > maximum_encoded || !bytes.len().is_multiple_of(4) {
        return Err(invalid());
    }
    let padding = if bytes.ends_with(b"==") {
        2
    } else {
        usize::from(bytes.ends_with(b"="))
    };
    let end = bytes.len().checked_sub(padding).ok_or_else(invalid)?;
    let mut last = 0;
    for byte in bytes.get(..end).ok_or_else(invalid)? {
        last = digit(*byte)?;
    }
    if (padding == 1 && last & 3 != 0) || (padding == 2 && last & 15 != 0) {
        return Err(invalid());
    }
    bytes
        .len()
        .checked_div(4)
        .and_then(|n| n.checked_mul(3))
        .and_then(|n| n.checked_sub(padding))
        .filter(|n| *n <= maximum)
        .ok_or_else(invalid)
}
fn version(
    version: &VersionRecordSnapshot,
    architecture: Architecture,
) -> Result<usize, LambdaServiceError> {
    validate_snapshot_settings(version).map_err(|_| invalid())?;
    validate_provided_settings(version.runtime.as_deref(), version.handler.as_deref())
        .map_err(|_| invalid())?;
    if version.state != "Active"
        || version
            .deployment
            .is_some_and(|state| state.state() != "Active")
        || version.architectures.as_slice() != [architecture.as_str()]
        || version.image_uri.is_some()
        || version.resolved_image.is_some()
        || version.image_config.is_some()
    {
        return Err(invalid());
    }
    let bytes = decoded_len(
        version.zip_bytes_base64.as_deref().ok_or_else(invalid)?,
        DockerArchiveProfile::MAX_ZIP_BYTES,
    )?;
    if u64::try_from(bytes).ok() != Some(version.code_size) || version.code_sha256.len() != 44 {
        return Err(invalid());
    }
    let digest = STANDARD
        .decode(&version.code_sha256)
        .map_err(|_| invalid())?;
    if digest.len() != 32 || STANDARD.encode(digest) != version.code_sha256 {
        return Err(invalid());
    }
    Ok(bytes)
}

pub(super) fn detach(
    snapshot: &mut LambdaSnapshot,
    profile: DockerArchiveProfile,
) -> Result<Vec<String>, LambdaServiceError> {
    detach_with_limit(snapshot, profile, DockerArchiveProfile::MAX_ZIP_DATA_BYTES)
}
pub(super) fn detach_with_limit(
    snapshot: &mut LambdaSnapshot,
    profile: DockerArchiveProfile,
    maximum: usize,
) -> Result<Vec<String>, LambdaServiceError> {
    let mut remaining = maximum.min(DockerArchiveProfile::MAX_ZIP_DATA_BYTES);
    let mut names = HashSet::new();
    let mut identities = HashSet::new();
    for function in &snapshot.functions.functions {
        FunctionName::parse(&function.name).map_err(|_| invalid())?;
        if !names.insert(function.name.as_str()) || !identities.insert(function.resource_id) {
            return Err(invalid());
        }
    }
    let mut bodies = Vec::new();
    for function in &mut snapshot.functions.functions {
        let package = match function.latest.package_type.as_str() {
            "Image" => "Image",
            "Zip" => "Zip",
            _ => return Err(invalid()),
        };
        for row in once(&mut function.latest).chain(function.versions.values_mut()) {
            if row.package_type != package {
                return Err(invalid());
            }
            match row.package_type.as_str() {
                "Image" if row.zip_bytes_base64.is_none() => {}
                "Zip" => {
                    let len = version(row, profile.zip_architecture().ok_or_else(invalid)?)?;
                    remaining = remaining.checked_sub(len).ok_or_else(invalid)?;
                    bodies.push(row.zip_bytes_base64.take().ok_or_else(invalid)?);
                }
                _ => return Err(invalid()),
            }
        }
    }
    Ok(bodies)
}
pub(super) fn reattach(
    snapshot: &mut LambdaSnapshot,
    bodies: Vec<String>,
) -> Result<(), LambdaServiceError> {
    let mut bodies = bodies.into_iter();
    for function in &mut snapshot.functions.functions {
        for row in once(&mut function.latest).chain(function.versions.values_mut()) {
            if row.package_type == "Zip" {
                if row.zip_bytes_base64.is_some() {
                    return Err(invalid());
                }
                row.zip_bytes_base64 = Some(bodies.next().ok_or_else(invalid)?);
            }
        }
    }
    if bodies.next().is_some() {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn record(record: &FunctionRecord) -> Result<(), LambdaServiceError> {
    for row in once(&record.latest).chain(record.versions.values()) {
        if row.package_type != record.latest.package_type {
            return Err(invalid());
        }
        if row.package_type == "Zip" {
            validate_provided_settings(row.runtime.as_deref(), row.handler.as_deref())
                .map_err(|_| invalid())?;
            if row.deployment.state() != "Active"
                || row
                    .zip_bytes
                    .as_ref()
                    .is_none_or(|bytes| bytes.len() > DockerArchiveProfile::MAX_ZIP_BYTES)
            {
                return Err(invalid());
            }
        } else if row.package_type != "Image" || row.zip_bytes.is_some() {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    #[rstest]
    #[case("AA==", 1)]
    #[case("AAA=", 2)]
    #[case("AAAA", 3)]
    fn test_should_measure_canonical_bodies_without_decoding(
        #[case] body: &str,
        #[case] bytes: usize,
    ) {
        assert_eq!(decoded_len(body, bytes).unwrap(), bytes);
        assert!(decoded_len(body, bytes.saturating_sub(1)).is_err());
    }
    #[rstest]
    #[case("")]
    #[case("AA")]
    #[case("AB==")]
    #[case("AAB=")]
    #[case("A===")]
    #[case("AA-A")]
    #[case("AA_A")]
    #[case("AA A")]
    #[case("====")]
    fn test_should_reject_noncanonical_source_without_copying(#[case] body: &str) {
        assert!(decoded_len(body, 64).is_err());
    }
}
