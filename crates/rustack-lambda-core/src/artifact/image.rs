//! Bounded Docker image reference grammar and immutable inspection metadata.

use std::{fmt, net::Ipv6Addr, str::FromStr};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use typed_builder::TypedBuilder;
use winnow::{
    ModalResult, Parser,
    combinator::{alt, repeat, separated},
    token::take_while,
};

use super::{ImageExecutionDefaults, MAX_IMAGE_BYTES};

/// Closed Lambda architecture, also used to select a Docker platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Architecture {
    /// Linux amd64 image or `x86_64` function.
    #[serde(rename = "x86_64")]
    X86_64,
    /// Linux aarch64 image or arm64 function.
    #[serde(rename = "arm64")]
    Arm64,
}

impl Architecture {
    /// Lambda wire spelling of this architecture.
    ///
    /// ```
    /// use rustack_lambda_core::artifact::Architecture;
    /// assert_eq!(Architecture::Arm64.as_str(), "arm64");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64 => "arm64",
        }
    }

    /// Linux platform selector for Docker.
    ///
    /// ```
    /// use rustack_lambda_core::artifact::Architecture;
    /// assert_eq!(Architecture::X86_64.docker_platform(), "linux/amd64");
    /// ```
    #[must_use]
    pub const fn docker_platform(self) -> &'static str {
        match self {
            Self::X86_64 => "linux/amd64",
            Self::Arm64 => "linux/arm64",
        }
    }
}

impl FromStr for Architecture {
    type Err = ArtifactError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "x86_64" => Ok(Self::X86_64),
            "arm64" => Ok(Self::Arm64),
            _ => Err(ArtifactError::Architecture),
        }
    }
}

/// Invalid artifact metadata; errors never include the untrusted input.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[non_exhaustive]
pub enum ArtifactError {
    /// Reference does not match the bounded Docker grammar.
    #[error("invalid Docker image reference")]
    Reference,
    /// Digest must be SHA-256 with 64 lowercase hexadecimal digits.
    #[error("invalid SHA-256 image digest")]
    Digest,
    /// Only the two Lambda architectures are supported.
    #[error("unsupported Lambda architecture")]
    Architecture,
    /// Inspection did not describe a supported Linux image.
    #[error("invalid Linux image inspection metadata")]
    Inspection,
}

/// A validated Docker repository, optional tag, and optional SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct ImageReference(String);

impl ImageReference {
    /// Original validated spelling, passed directly to the daemon without a shell.
    ///
    /// ```
    /// use rustack_lambda_core::artifact::ImageReference;
    /// let image: ImageReference = "localhost:5000/team/worker:v1".parse()?;
    /// assert_eq!(image.as_str(), "localhost:5000/team/worker:v1");
    /// # Ok::<(), rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Repository spelling, including an explicit registry, without tag/digest.
    ///
    /// ```
    /// let image: rustack_lambda_core::artifact::ImageReference = "localhost:5000/team/worker:v1".parse()?;
    /// assert_eq!(image.repository(), "localhost:5000/team/worker");
    /// # Ok::<(), rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub fn repository(&self) -> &str {
        let name = self
            .0
            .split_once('@')
            .map_or(self.0.as_str(), |(name, _)| name);
        if name
            .rsplit('/')
            .next()
            .is_some_and(|part| part.contains(':'))
        {
            name.rsplit_once(':').map_or(name, |(name, _)| name)
        } else {
            name
        }
    }

    /// Explicit registry component; short local names do not invent provenance.
    ///
    /// ```
    /// let image: rustack_lambda_core::artifact::ImageReference = "worker:v1".parse()?;
    /// assert_eq!(image.registry(), None);
    /// # Ok::<(), rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub fn registry(&self) -> Option<&str> {
        self.repository()
            .split_once('/')
            .and_then(|(first, _)| is_registry_component(first).then_some(first))
    }

    /// Requested digest suffix, if present, already checked by the constructor.
    ///
    /// ```
    /// let image: rustack_lambda_core::artifact::ImageReference = "worker:v1".parse()?;
    /// assert_eq!(image.digest(), None);
    /// # Ok::<(), rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub fn digest(&self) -> Option<&str> {
        self.0.split_once('@').map(|(_, digest)| digest)
    }
}

fn is_registry_component(first: &str) -> bool {
    first.contains('.')
        || first.contains(':')
        || first == "localhost"
        || first.starts_with('[')
        || first.bytes().any(|byte| byte.is_ascii_uppercase())
}

impl FromStr for ImageReference {
    type Err = ArtifactError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.len() > 1024 || !value.is_ascii() {
            return Err(ArtifactError::Reference);
        }
        let (name_tag, digest) = match value.split_once('@') {
            Some((name, digest)) => (name, Some(digest)),
            None => (value, None),
        };
        if let Some(digest) = digest {
            digest.parse::<Sha256Digest>()?;
        }
        let last_component = name_tag
            .rsplit('/')
            .next()
            .ok_or(ArtifactError::Reference)?;
        let (name, tag) = match last_component.split_once(':') {
            Some((_, tag)) => {
                let (name, _) = name_tag.rsplit_once(':').ok_or(ArtifactError::Reference)?;
                (name, Some(tag))
            }
            None => (name_tag, None),
        };
        if let Some(tag) = tag {
            tag_parser
                .parse(tag)
                .map_err(|_| ArtifactError::Reference)?;
        }
        if name.len() > 255 {
            return Err(ArtifactError::Reference);
        }
        let repository = if let Some((first, remaining)) = name.split_once('/') {
            if is_registry_component(first) {
                validate_registry(first)?;
                remaining
            } else {
                name
            }
        } else {
            name
        };
        repository_parser
            .parse(repository)
            .map_err(|_| ArtifactError::Reference)?;
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for ImageReference {
    type Error = ArtifactError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<ImageReference> for String {
    fn from(value: ImageReference) -> Self {
        value.0
    }
}
impl fmt::Display for ImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn component(input: &mut &str) -> ModalResult<()> {
    (
        take_while(1.., |c: char| c.is_ascii_lowercase() || c.is_ascii_digit()),
        repeat::<_, _, (), _, _>(
            0..,
            (
                alt(("__", "_", ".", take_while(1.., '-'))),
                take_while(1.., |c: char| c.is_ascii_lowercase() || c.is_ascii_digit()),
            ),
        ),
    )
        .void()
        .parse_next(input)
}
fn repository_parser(input: &mut &str) -> ModalResult<()> {
    separated(1.., component, '/').parse_next(input)
}

fn tag_parser(input: &mut &str) -> ModalResult<()> {
    (
        take_while(1, |c: char| c.is_ascii_alphanumeric() || c == '_'),
        take_while(0..=127, |c: char| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
        }),
    )
        .void()
        .parse_next(input)
}

fn digest_parser(input: &mut &str) -> ModalResult<()> {
    (
        "sha256:",
        take_while(64, |c: char| c.is_ascii_digit() || matches!(c, 'a'..='f')),
    )
        .void()
        .parse_next(input)
}

fn validate_registry(registry: &str) -> Result<(), ArtifactError> {
    let (host, port) = if let Some(bracketed) = registry.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']').ok_or(ArtifactError::Reference)?;
        if !host
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b':')
        {
            return Err(ArtifactError::Reference);
        }
        host.parse::<Ipv6Addr>()
            .map_err(|_| ArtifactError::Reference)?;
        let port = if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':').ok_or(ArtifactError::Reference)?)
        };
        (None, port)
    } else {
        let (host, port) = registry
            .split_once(':')
            .map_or((registry, None), |(host, port)| (host, Some(port)));
        (Some(host), port)
    };
    if let Some(host) = host {
        if host.is_empty()
            || host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || label.starts_with('-')
                    || label.ends_with('-')
            })
        {
            return Err(ArtifactError::Reference);
        }
    }
    if let Some(port) = port {
        if port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().map_or(true, |port| port == 0)
        {
            return Err(ArtifactError::Reference);
        }
    }
    Ok(())
}

/// Immutable SHA-256 digest, including its algorithm prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// The validated `sha256:<hex>` value.
    ///
    /// ```
    /// use rustack_lambda_core::artifact::Sha256Digest;
    /// let digest: Sha256Digest = format!("sha256:{}", "a".repeat(64)).parse()?;
    /// assert_eq!(digest.as_str().len(), 71);
    /// # Ok::<(), rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for Sha256Digest {
    type Err = ArtifactError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 71 {
            return Err(ArtifactError::Digest);
        }
        digest_parser
            .parse(value)
            .map_err(|_| ArtifactError::Digest)?;
        Ok(Self(value.to_owned()))
    }
}
impl TryFrom<String> for Sha256Digest {
    type Error = ArtifactError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<Sha256Digest> for String {
    fn from(value: Sha256Digest) -> Self {
        value.0
    }
}

/// Validated primitive fields returned by image inspection.
#[derive(Debug, Clone, TypedBuilder, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct ImageMetadata {
    /// Caller-supplied deployment reference.
    pub requested_reference: ImageReference,
    /// Immutable daemon image content ID.
    pub image_id: Sha256Digest,
    /// Selected platform manifest, separate from an addressable parent image ID.
    pub selected_manifest: Option<Sha256Digest>,
    /// Registry repository and selected manifest digest, when available.
    pub registry_manifest: Option<ImageReference>,
    /// Inspected operating system, required to equal linux.
    pub os: String,
    /// Selected Lambda architecture.
    pub architecture: Architecture,
    /// SHA-256 of the normalized inspected execution metadata.
    pub inspection_hash: Sha256Digest,
    /// Selected image size reported by the daemon, bounded to 10 GiB locally.
    pub size_bytes: u64,
    /// Validated inherited execution defaults, retained without re-inspection.
    pub execution: ImageExecutionDefaults,
}

impl ImageMetadata {
    /// Compute the canonical fingerprint also verified during snapshot restore.
    ///
    /// # Errors
    /// Rejects an invalid inspected size or bounded metadata serialization.
    ///
    /// ```no_run
    /// # fn hash(metadata: &rustack_lambda_core::artifact::ImageMetadata) -> Result<(), rustack_lambda_core::artifact::ArtifactError> {
    /// let hash = rustack_lambda_core::artifact::ImageMetadata::execution_fingerprint(
    ///     &metadata.image_id, metadata.selected_manifest.as_ref(),
    ///     metadata.architecture, metadata.size_bytes, &metadata.execution)?;
    /// # Ok(()) }
    /// ```
    pub fn execution_fingerprint(
        image_id: &Sha256Digest,
        selected_manifest: Option<&Sha256Digest>,
        architecture: Architecture,
        size: u64,
        defaults: &ImageExecutionDefaults,
    ) -> Result<Sha256Digest, ArtifactError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Fingerprint<'a> {
            version: u8,
            image_id: &'a Sha256Digest,
            descriptor_digest: Option<&'a Sha256Digest>,
            architecture: Architecture,
            size: u64,
            defaults: &'a ImageExecutionDefaults,
        }
        if !(1..=MAX_IMAGE_BYTES).contains(&size) {
            return Err(ArtifactError::Inspection);
        }
        // Validated defaults contain <=128KiB of argv/environment, whose worst
        // JSON escape expansion is sixfold; fixed-size identities keep this <1MiB.
        let bytes = serde_json::to_vec(&Fingerprint {
            version: 1,
            image_id,
            descriptor_digest: selected_manifest,
            architecture,
            size,
            defaults,
        })
        .map_err(|_| ArtifactError::Inspection)?;
        format!("sha256:{}", hex::encode(Sha256::digest(bytes))).parse()
    }
}

/// Immutable Linux image metadata that passed all inspection invariants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "ImageMetadata", into = "ImageMetadata")]
#[non_exhaustive]
pub struct ResolvedImage(ImageMetadata);

impl TryFrom<ImageMetadata> for ResolvedImage {
    type Error = ArtifactError;
    fn try_from(metadata: ImageMetadata) -> Result<Self, Self::Error> {
        if metadata.os != "linux"
            || metadata.size_bytes == 0
            || metadata.size_bytes > MAX_IMAGE_BYTES
            || metadata
                .registry_manifest
                .as_ref()
                .is_some_and(|image| !image.as_str().contains('@'))
        {
            return Err(ArtifactError::Inspection);
        }
        if metadata.inspection_hash
            != ImageMetadata::execution_fingerprint(
                &metadata.image_id,
                metadata.selected_manifest.as_ref(),
                metadata.architecture,
                metadata.size_bytes,
                &metadata.execution,
            )?
        {
            return Err(ArtifactError::Inspection);
        }
        if let (Some(selected), Some(registry)) =
            (&metadata.selected_manifest, &metadata.registry_manifest)
        {
            if registry.digest() != Some(selected.as_str()) {
                return Err(ArtifactError::Inspection);
            }
        }
        Ok(Self(metadata))
    }
}
impl From<ResolvedImage> for ImageMetadata {
    fn from(image: ResolvedImage) -> Self {
        image.0
    }
}

impl ResolvedImage {
    #[cfg(test)]
    pub(crate) fn fixture(architecture: Architecture) -> Self {
        let image_id = format!("sha256:{}", "a".repeat(64))
            .parse()
            .expect("valid unit digest");
        let execution = super::ImageExecutionMetadata::builder()
            .entrypoint(vec!["/bootstrap".into()])
            .command(Vec::new())
            .working_directory("/".into())
            .environment(Vec::new())
            .user("1000".into())
            .build()
            .try_into()
            .expect("valid execution defaults");
        let hash =
            ImageMetadata::execution_fingerprint(&image_id, None, architecture, 1024, &execution)
                .expect("unit fingerprint");
        ImageMetadata::builder()
            .requested_reference(
                "rustack-fixture:unit"
                    .parse()
                    .expect("valid unit reference"),
            )
            .image_id(image_id)
            .selected_manifest(None)
            .registry_manifest(None)
            .os("linux".into())
            .architecture(architecture)
            .inspection_hash(hash)
            .size_bytes(1024)
            .execution(execution)
            .build()
            .try_into()
            .expect("valid unit image metadata")
    }
    /// Base64 SHA-256 of canonical inspected execution metadata, independent of a tag.
    ///
    /// # Errors
    /// Rejects a malformed canonical fingerprint before decoding it.
    ///
    /// ```no_run
    /// # fn hash(image: &rustack_lambda_core::artifact::ResolvedImage) -> Result<(), rustack_lambda_core::artifact::ArtifactError> {
    /// let code_hash = image.code_sha256()?;
    /// assert_eq!(code_hash.len(), 44);
    /// # Ok(()) }
    /// ```
    pub fn code_sha256(&self) -> Result<String, ArtifactError> {
        let digest = self
            .0
            .inspection_hash
            .as_str()
            .strip_prefix("sha256:")
            .ok_or(ArtifactError::Inspection)?;
        let bytes = hex::decode(digest).map_err(|_| ArtifactError::Inspection)?;
        Ok(STANDARD.encode(bytes))
    }

    /// Immutable inspected metadata; Invoke must use `image_id` rather than a tag.
    ///
    /// ```
    /// # fn inspect(image: &rustack_lambda_core::artifact::ResolvedImage) {
    /// assert_eq!(image.metadata().os, "linux");
    /// # }
    /// ```
    #[must_use]
    pub fn metadata(&self) -> &ImageMetadata {
        &self.0
    }

    /// Registry manifest reference, or Rustack's explicit local-content extension.
    ///
    /// ```
    /// # fn inspect(image: &rustack_lambda_core::artifact::ResolvedImage) {
    /// assert!(!image.resolved_uri().is_empty());
    /// # }
    /// ```
    #[must_use]
    pub fn resolved_uri(&self) -> String {
        self.0.registry_manifest.as_ref().map_or_else(
            || format!("local://{}", self.0.image_id.as_str()),
            ToString::to_string,
        )
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("worker")]
    #[case("team/worker:v1")]
    #[case("team/foo__bar:Tag_1.2-3")]
    #[case("team/foo---bar:_")]
    #[case("localhost:5000/worker:latest")]
    #[case("registry.example.com/team/worker:V2")]
    #[case("Registry/worker:v1")]
    #[case("127.0.0.1:65535/worker")]
    #[case("[::1]:5000/team/worker:v1")]
    #[case("[2001:db8::1]/worker")]
    fn test_should_parse_docker_references_and_round_trip_saved_values(
        #[case] reference: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let parsed: ImageReference = reference.parse()?;
        assert_eq!(parsed.as_str(), reference);
        let saved = serde_json::to_string(&parsed)?;
        assert_eq!(serde_json::from_str::<ImageReference>(&saved)?, parsed);
        Ok(())
    }

    #[rstest]
    #[case("")]
    #[case("https://registry.example.com/worker")]
    #[case("user@registry.example.com/worker")]
    #[case("worker:$(id)")]
    #[case("worker;id")]
    #[case("worker\n")]
    #[case("worker\0")]
    #[case("team/UPPER")]
    #[case("team//worker")]
    #[case("team/../worker")]
    #[case("team/foo___bar")]
    #[case("team/foo..bar")]
    #[case("worker:")]
    #[case("worker:.tag")]
    #[case("worker:tag:extra")]
    #[case("registry.example.com:0/worker")]
    #[case("registry.example.com:65536/worker")]
    #[case("registry..example.com/worker")]
    #[case("registry.example.com:/worker")]
    #[case("[::1]extra/worker")]
    #[case("[not-ipv6]:5000/worker")]
    #[case("[::ffff:192.0.2.1]/worker")]
    #[case("[fe80::1%eth0]/worker")]
    #[case("worker@sha256:abc")]
    fn test_should_reject_invalid_references_at_parse_and_deserialization(#[case] reference: &str) {
        assert!(reference.parse::<ImageReference>().is_err());
        assert!(
            serde_json::from_value::<ImageReference>(serde_json::Value::String(reference.into()))
                .is_err()
        );
    }

    #[test]
    fn test_should_enforce_reference_tag_and_repository_byte_limits() -> Result<(), ArtifactError> {
        let longest_tag = format!("worker:{}", "t".repeat(128));
        longest_tag.parse::<ImageReference>()?;
        assert!(format!("{longest_tag}t").parse::<ImageReference>().is_err());
        "a".repeat(255).parse::<ImageReference>()?;
        assert!("a".repeat(256).parse::<ImageReference>().is_err());
        assert!("a".repeat(1025).parse::<ImageReference>().is_err());
        assert!("wörker".parse::<ImageReference>().is_err());
        Ok(())
    }

    #[test]
    fn test_should_parse_digest_references_without_confusing_registry_ports_and_tags()
    -> Result<(), ArtifactError> {
        let digest = format!("sha256:{}", "a".repeat(64));
        for reference in [
            format!("worker@{digest}"),
            format!("localhost:5000/team/worker:v1@{digest}"),
            format!("[::1]:5000/worker@{digest}"),
        ] {
            reference.parse::<ImageReference>()?;
        }
        assert!(
            format!("worker@{digest}@{digest}")
                .parse::<ImageReference>()
                .is_err()
        );
        assert!(
            format!("sha256:{}", "A".repeat(64))
                .parse::<Sha256Digest>()
                .is_err()
        );
        assert!(
            format!("sha256:{}", "a".repeat(63))
                .parse::<Sha256Digest>()
                .is_err()
        );
        assert!(
            format!("sha512:{}", "a".repeat(64))
                .parse::<Sha256Digest>()
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_preserve_local_and_registry_image_identity_through_snapshot_round_trips()
    -> Result<(), Box<dyn std::error::Error>> {
        let local = ResolvedImage::fixture(Architecture::Arm64);
        assert_eq!(
            local.resolved_uri(),
            format!("local://sha256:{}", "a".repeat(64))
        );
        let mut metadata = local.metadata().clone();
        let manifest = format!("registry.example.com/worker@sha256:{}", "c".repeat(64));
        metadata.registry_manifest = Some(manifest.parse()?);
        let image = ResolvedImage::try_from(metadata)?;
        assert_eq!(image.resolved_uri(), manifest);
        let saved = serde_json::to_vec(&image)?;
        let restored: ResolvedImage = serde_json::from_slice(&saved)?;
        assert_eq!(restored.metadata().image_id, image.metadata().image_id);
        assert_eq!(restored.metadata().architecture, Architecture::Arm64);
        assert_eq!(restored.resolved_uri(), manifest);
        Ok(())
    }

    #[test]
    fn test_should_revalidate_image_inspection_on_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let image = ResolvedImage::fixture(Architecture::X86_64);
        let saved = serde_json::to_value(&image)?;
        for (field, invalid) in [
            ("os", serde_json::json!("windows")),
            ("architecture", serde_json::json!("riscv64")),
            ("imageId", serde_json::json!("worker:latest")),
            ("registryManifest", serde_json::json!("worker:latest")),
            ("sizeBytes", serde_json::json!(0)),
            (
                "sizeBytes",
                serde_json::json!(10_u64 * 1024 * 1024 * 1024 + 1),
            ),
        ] {
            let mut invalid_image = saved.clone();
            invalid_image
                .as_object_mut()
                .ok_or("image must be an object")?
                .insert(field.into(), invalid);
            assert!(
                serde_json::from_value::<ResolvedImage>(invalid_image).is_err(),
                "{field}"
            );
        }
        Ok(())
    }
    #[rstest::rstest]
    #[case("/imageId", serde_json::json!(format!("sha256:{}", "d".repeat(64))))]
    #[case("/selectedManifest", serde_json::json!(format!("sha256:{}", "d".repeat(64))))]
    #[case("/sizeBytes", serde_json::json!(1025))]
    #[case("/architecture", serde_json::json!("arm64"))]
    #[case("/execution/command", serde_json::json!(["changed-valid-argv"]))]
    #[case("/execution/environment", serde_json::json!(["KEY=changed-valid-value"]))]
    #[case("/execution/user", serde_json::json!("1001"))]
    #[case("/execution/entrypoint", serde_json::json!(["/other"]))]
    #[case("/execution/workingDirectory", serde_json::json!("/other"))]
    #[case("/inspectionHash", serde_json::json!(format!("sha256:{}", "f".repeat(64))))]
    fn test_should_reject_valid_saved_metadata_mutations_with_a_stale_fingerprint(
        #[case] pointer: &str,
        #[case] replacement: serde_json::Value,
    ) {
        let image = ResolvedImage::fixture(Architecture::X86_64);
        let mut saved = serde_json::to_value(image).unwrap();
        *saved.pointer_mut(pointer).unwrap() = replacement;
        assert!(serde_json::from_value::<ResolvedImage>(saved).is_err());
    }

    #[test]
    fn test_should_extract_registry_repository_and_digest_without_confusing_ports() {
        for (reference, repository, registry) in [
            (
                "localhost:5000/team/worker:v1",
                "localhost:5000/team/worker",
                Some("localhost:5000"),
            ),
            (
                "[::1]:5000/team/worker:v1",
                "[::1]:5000/team/worker",
                Some("[::1]:5000"),
            ),
            ("Registry/worker:v1", "Registry/worker", Some("Registry")),
            ("team/worker:v1", "team/worker", None),
            ("worker:v1", "worker", None),
        ] {
            let with_digest = format!("{reference}@sha256:{}", "e".repeat(64));
            let image: ImageReference = with_digest.parse().unwrap();
            assert_eq!(image.repository(), repository);
            assert_eq!(image.registry(), registry);
            assert_eq!(
                image.digest(),
                Some(format!("sha256:{}", "e".repeat(64)).as_str())
            );
        }
    }
}
