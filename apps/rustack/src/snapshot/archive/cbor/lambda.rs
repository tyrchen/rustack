//! Exact-path metadata accounting, shared by the existing CBOR scanner.

#[cfg(feature = "lambda")]
use rustack_lambda_core::provider::DockerArchiveProfile;

use super::{Failure, Key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Path {
    Root,
    Functions,
    Rows,
    Function,
    Versions,
    Version,
    Zip,
    Other,
}
impl Path {
    pub(super) fn shape(self, major: u8) -> Self {
        match (self, major) {
            (Self::Root | Self::Functions | Self::Function | Self::Versions | Self::Version, 5)
            | (Self::Rows, 4)
            | (Self::Zip, 3) => self,
            _ => Self::Other,
        }
    }
    pub(super) fn child(self, key: Option<&Key<'_>>) -> Self {
        match (self, key) {
            (Self::Root, Some(Key::Text("functions"))) => Self::Functions,
            (Self::Functions, Some(Key::Text("functions"))) => Self::Rows,
            (Self::Rows, None) => Self::Function,
            (Self::Function, Some(Key::Text("latest")))
            | (Self::Versions, Some(Key::Unsigned(_))) => Self::Version,
            (Self::Function, Some(Key::Text("versions"))) => Self::Versions,
            (Self::Version, Some(Key::Text("zipBytesBase64"))) => Self::Zip,
            _ => Self::Other,
        }
    }
}

#[derive(Debug)]
pub(super) struct Budget {
    pub(super) metadata: usize,
    members: usize,
    functions: usize,
    revisions: usize,
    zip_remaining: usize,
    zip_maximum: usize,
    pub(super) zip: bool,
}
impl Budget {
    #[cfg(feature = "lambda")]
    pub(super) fn new(profile: DockerArchiveProfile) -> Self {
        Self {
            metadata: DockerArchiveProfile::METADATA_BYTES,
            members: 65_536,
            functions: DockerArchiveProfile::MAX_FUNCTIONS,
            revisions: DockerArchiveProfile::MAX_REVISIONS,
            zip_remaining: DockerArchiveProfile::MAX_ZIP_DATA_BYTES,
            zip_maximum: DockerArchiveProfile::MAX_ZIP_BYTES,
            zip: profile.zip_architecture().is_some(),
        }
    }
    pub(super) fn collection(&self, path: Path, count: usize) -> Result<(), Failure> {
        if count > self.members
            || (path == Path::Rows && count > self.functions)
            || (path == Path::Versions && count > self.revisions)
        {
            return Err(Failure::Collection);
        }
        Ok(())
    }
    pub(super) fn member(&mut self, path: Path, key: Option<&Key<'_>>) -> Result<(), Failure> {
        self.members = self.members.checked_sub(1).ok_or(Failure::Nodes)?;
        if path == Path::Rows {
            self.functions = self.functions.checked_sub(1).ok_or(Failure::Collection)?;
        }
        if path == Path::Versions
            || (path == Path::Function && matches!(key, Some(Key::Text("latest"))))
        {
            self.revisions = self.revisions.checked_sub(1).ok_or(Failure::Collection)?;
        }
        Ok(())
    }
    pub(super) fn blob(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let maximum = self
            .zip_maximum
            .div_ceil(3)
            .checked_mul(4)
            .ok_or(Failure::Scalar)?;
        if bytes.len() > maximum || !bytes.len().is_multiple_of(4) {
            return Err(Failure::Scalar);
        }
        let padding = if bytes.ends_with(b"==") {
            2
        } else {
            usize::from(bytes.ends_with(b"="))
        };
        let decoded = bytes
            .len()
            .checked_div(4)
            .and_then(|n| n.checked_mul(3))
            .and_then(|n| n.checked_sub(padding))
            .ok_or(Failure::Scalar)?;
        if decoded > self.zip_maximum {
            return Err(Failure::Scalar);
        }
        self.zip_remaining = self
            .zip_remaining
            .checked_sub(decoded)
            .ok_or(Failure::Bytes)?;
        Ok(())
    }
}

#[cfg(all(test, feature = "lambda"))]
mod tests {
    use rustack_lambda_core::{artifact::Architecture, provider::DockerArchiveProfile};

    use super::*;
    #[test]
    fn test_should_bound_each_blob_and_aggregate_before_typed_allocation() {
        let mut budget = Budget::new(DockerArchiveProfile::ProvidedZip(Architecture::Arm64));
        budget.zip_maximum = 3;
        budget.zip_remaining = 5;
        assert_eq!(budget.blob(b"AAAA"), Ok(()));
        assert_eq!(budget.blob(b"AAA="), Ok(()));
        assert_eq!(budget.blob(b"AA=="), Err(Failure::Bytes));
        assert_eq!(budget.blob(b"AAAAAAAA"), Err(Failure::Scalar));
        assert_eq!(budget.blob(b"AAA"), Err(Failure::Scalar));
    }
}
