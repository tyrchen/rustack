//! Read-only platform selection and executable-image validation before deployment.

use bytes::Bytes;
use http::{Method, StatusCode};
use rustack_lambda_model::types::ImageConfig;
use tokio::time::Instant;

use super::{
    DockerError, DockerPort, Operation, decode, escaped,
    models::{ImageInspect, SystemInfo},
};
use crate::{
    artifact::{
        Architecture, ArtifactError, ImageExecutionDefaults, ImageExecutionMetadata, ImageMetadata,
        ImageReference, ResolvedImage, Sha256Digest,
    },
    function_config::validate_image_config,
};

/// Read-only local resolution; deployment publication remains with its actor.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImageResolver {
    port: DockerPort,
}

impl ImageResolver {
    /// Use an already negotiated bounded Docker port; never discover a daemon.
    ///
    /// ```no_run
    /// # fn resolver(port: rustack_lambda_core::docker::DockerPort) {
    /// let resolver = rustack_lambda_core::docker::ImageResolver::new(port);
    /// # }
    /// ```
    #[must_use]
    pub fn new(port: DockerPort) -> Self {
        Self { port }
    }

    /// Resolve a preloaded image and validate effective entry configuration.
    ///
    /// This never pulls, builds or reads registry credentials. Function settings
    /// and deployment status/CAS are validated and owned by the provider.
    ///
    /// # Errors
    /// Returns wire failure, invalid overrides or invalid/ambiguous inspection
    /// metadata. No error includes image argv, environment or daemon text.
    ///
    /// ```no_run
    /// # async fn example(port: rustack_lambda_core::docker::DockerPort) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// use rustack_lambda_core::{artifact::Architecture, docker::ImageResolver};
    /// let image = ImageResolver::new(port).resolve_local(&"worker:v1".parse()?,
    ///     Architecture::Arm64, None,
    ///     tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn resolve_local(
        &self,
        reference: &ImageReference,
        architecture: Architecture,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
    ) -> Result<ResolvedImage, DockerError> {
        super::check_deadline(deadline)?;
        if let Some(overrides) = overrides {
            validate_image_config(overrides).map_err(|_| DockerError::Configuration)?;
        }
        let root = self.port.inspect_image(reference, deadline).await?;
        let launch_id: Sha256Digest = root.id.as_deref().ok_or_else(invalid)?.parse()?;
        let inspected = if self.port.api_version().minor() >= 49 {
            drop(root);
            let platform = match architecture {
                Architecture::X86_64 => r#"{"os":"linux","architecture":"amd64"}"#,
                Architecture::Arm64 => r#"{"os":"linux","architecture":"arm64"}"#,
            };
            let query = form_urlencoded::Serializer::new(String::new())
                .append_pair("platform", platform)
                .finish();
            let path = format!("/images/{}/json?{query}", escaped(launch_id.as_str()));
            decode(
                self.port
                    .call(
                        Operation::Image,
                        Method::GET,
                        path,
                        Bytes::new(),
                        StatusCode::OK,
                        deadline,
                    )
                    .await?,
                deadline,
            )?
        } else {
            root
        };
        if inspected.descriptor.is_none() {
            let info = self.port.info(deadline).await?;
            verify_classic_store(&inspected, &info)?;
        }
        let result = resolve(reference, architecture, overrides, inspected, &launch_id);
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        result
    }
}

fn verify_classic_store(inspected: &ImageInspect, info: &SystemInfo) -> Result<(), DockerError> {
    let driver = inspected.graph_driver.as_ref().ok_or_else(invalid)?;
    if !matches!(
        driver.name.as_str(),
        "overlay2" | "aufs" | "btrfs" | "devicemapper" | "vfs" | "zfs" | "fuse-overlayfs"
    ) || info.driver.as_deref() != Some(driver.name.as_str())
        || inspected
            .manifests
            .as_ref()
            .is_some_and(|manifests| !manifests.is_empty())
    {
        return Err(invalid());
    }
    let statuses = info.driver_status.as_deref().unwrap_or_default();
    if statuses.len() > 128 {
        return Err(invalid());
    }
    for status in statuses {
        let [key, value] = status.as_slice() else {
            return Err(invalid());
        };
        if key.len() > 256
            || value.len() > 16 * 1024
            || (key.eq_ignore_ascii_case("driver-type") && value == "io.containerd.snapshotter.v1")
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn invalid() -> DockerError {
    ArtifactError::Inspection.into()
}

fn resolve(
    reference: &ImageReference,
    architecture: Architecture,
    overrides: Option<&ImageConfig>,
    inspected: ImageInspect,
    launch_id: &Sha256Digest,
) -> Result<ResolvedImage, DockerError> {
    let docker_arch = match architecture {
        Architecture::X86_64 => "amd64",
        Architecture::Arm64 => "arm64",
    };
    if inspected.os.as_deref() != Some("linux")
        || inspected.architecture.as_deref() != Some(docker_arch)
    {
        return Err(invalid());
    }
    let _: Sha256Digest = inspected.id.as_deref().ok_or_else(invalid)?.parse()?;
    let size = inspected
        .size
        .and_then(|value| u64::try_from(value).ok())
        .filter(|size| (1..=10 * 1024 * 1024 * 1024).contains(size))
        .ok_or_else(invalid)?;
    let descriptor = selected_descriptor(&inspected, docker_arch)?;
    let registry = registry_manifest(reference, &inspected, descriptor.as_ref())?;
    let config = inspected.config.ok_or_else(invalid)?;
    if config
        .volumes
        .as_ref()
        .is_some_and(|volumes| !volumes.is_empty())
    {
        return Err(invalid());
    }
    let defaults: ImageExecutionDefaults = ImageExecutionMetadata::builder()
        .entrypoint(config.entrypoint.unwrap_or_default())
        .command(config.cmd.unwrap_or_default())
        .working_directory(config.working_dir.unwrap_or_default())
        .environment(config.env.unwrap_or_default())
        .user(config.user.unwrap_or_default())
        .build()
        .try_into()?;
    defaults.plan(overrides)?;
    let inspection_hash = ImageMetadata::execution_fingerprint(
        launch_id,
        descriptor.as_ref(),
        architecture,
        size,
        &defaults,
    )?;
    ImageMetadata::builder()
        .requested_reference(reference.clone())
        .image_id(launch_id.clone())
        .selected_manifest(descriptor)
        .registry_manifest(registry)
        .os("linux".into())
        .architecture(architecture)
        .inspection_hash(inspection_hash)
        .size_bytes(size)
        .execution(defaults)
        .build()
        .try_into()
        .map_err(DockerError::from)
}

fn selected_descriptor(
    inspected: &ImageInspect,
    architecture: &str,
) -> Result<Option<Sha256Digest>, DockerError> {
    if inspected
        .manifests
        .as_ref()
        .is_some_and(|manifests| !manifests.is_empty())
    {
        return Err(invalid());
    }
    let Some(descriptor) = &inspected.descriptor else {
        return Ok(None);
    };
    if !matches!(
        descriptor.media_type.as_deref(),
        Some(
            "application/vnd.oci.image.manifest.v1+json"
                | "application/vnd.docker.distribution.manifest.v2+json"
        )
    ) || descriptor.artifact_type.is_some()
        || descriptor.platform.as_ref().is_some_and(|platform| {
            platform.os.as_deref().is_some_and(|os| os != "linux")
                || platform
                    .architecture
                    .as_deref()
                    .is_some_and(|arch| arch != architecture)
        })
        || descriptor.annotations.as_ref().is_some_and(|annotations| {
            annotations
                .get("vnd.docker.reference.type")
                .is_some_and(|kind| kind == "attestation-manifest")
        })
    {
        return Err(invalid());
    }
    Ok(Some(
        descriptor.digest.as_deref().ok_or_else(invalid)?.parse()?,
    ))
}

fn repository_identity(reference: &ImageReference) -> String {
    let (registry, repository) =
        reference
            .registry()
            .map_or(("docker.io", reference.repository()), |registry| {
                (
                    registry,
                    reference
                        .repository()
                        .split_once('/')
                        .map_or("", |(_, repository)| repository),
                )
            });
    let registry = registry.to_ascii_lowercase();
    let registry = match registry.as_str() {
        "index.docker.io" => "docker.io",
        value => value,
    };
    if registry == "docker.io" && !repository.contains('/') {
        format!("{registry}/library/{repository}")
    } else {
        format!("{registry}/{repository}")
    }
}

fn registry_manifest(
    reference: &ImageReference,
    inspected: &ImageInspect,
    descriptor: Option<&Sha256Digest>,
) -> Result<Option<ImageReference>, DockerError> {
    let digests = inspected.repo_digests.as_deref().unwrap_or_default();
    if digests.len() > 128 {
        return Err(invalid());
    }
    let repository = repository_identity(reference);
    let mut matches = Vec::new();
    for candidate in digests {
        let candidate: ImageReference = candidate.parse()?;
        let digest: Sha256Digest = candidate.digest().ok_or_else(invalid)?.parse()?;
        if repository_identity(&candidate) == repository {
            matches.push(digest);
        }
    }
    if let Some(requested) = reference.digest() {
        if descriptor.is_none_or(|digest| digest.as_str() != requested)
            && !matches.iter().any(|digest| digest.as_str() == requested)
        {
            return Err(invalid());
        }
    }
    if reference.registry().is_none() || matches.is_empty() {
        return Ok(None);
    }
    let selected = if let Some(descriptor) = descriptor {
        descriptor
    } else if let Some(requested) = reference.digest() {
        matches
            .iter()
            .find(|digest| digest.as_str() == requested)
            .ok_or_else(invalid)?
    } else {
        matches
            .iter()
            .min_by(|left, right| left.as_str().cmp(right.as_str()))
            .ok_or_else(invalid)?
    };
    Ok(Some(
        format!("{}@{}", reference.repository(), selected.as_str()).parse()?,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn digest(letter: char) -> String {
        format!("sha256:{}", letter.to_string().repeat(64))
    }
    fn inspection() -> Value {
        json!({"Id":digest('a'),"Os":"linux","Architecture":"arm64","Size":1024,
            "Descriptor":{"mediaType":"application/vnd.oci.image.manifest.v1+json", "digest":digest('b'),
                "platform":{"os":"linux","architecture":"arm64"}},
            "RepoDigests":[format!("registry.test/team/worker@{}",digest('c'))],
            "Config":{"Entrypoint":["/entry"],"Cmd":["handler"],"WorkingDir":"/task","Env":["SECRET=secret_value"],"User":"1000"}})
    }
    fn selected(reference: &str, value: Value) -> Result<ResolvedImage, DockerError> {
        resolve(
            &reference.parse()?,
            Architecture::Arm64,
            None,
            serde_json::from_value(value).unwrap(),
            &digest('a').parse()?,
        )
    }

    #[test]
    fn test_should_pin_selected_child_and_preserve_validated_defaults() {
        for suffix in [
            ":v1".to_owned(),
            format!("@{}", digest('c')),
            format!("@{}", digest('b')),
        ] {
            let image =
                selected(&format!("registry.test/team/worker{suffix}"), inspection()).unwrap();
            assert_eq!(image.metadata().image_id.as_str(), digest('a'));
            assert_eq!(
                image.resolved_uri(),
                format!("registry.test/team/worker@{}", digest('b'))
            );
            assert_eq!(
                image.metadata().execution.plan(None).unwrap().entrypoint(),
                ["/entry"]
            );
            assert_eq!(
                image.metadata().execution.metadata().environment,
                ["SECRET=secret_value"]
            );
            assert!(!format!("{image:?}").contains("secret_value"));
            let restored: ResolvedImage =
                serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
            assert_eq!(
                restored.metadata().execution.plan(None).unwrap().command(),
                ["handler"]
            );
        }
        assert!(
            selected(
                &format!("registry.test/team/worker@{}", digest('d')),
                inspection()
            )
            .is_err()
        );
    }

    #[test]
    fn test_should_keep_short_local_provenance_and_ignore_unrelated_repository_digests() {
        let mut value = inspection();
        value["RepoDigests"] = json!([format!("team/worker@{}", digest('c'))]);
        let image = selected("team/worker:v1", value).unwrap();
        assert_eq!(image.resolved_uri(), format!("local://{}", digest('a')));
        let mut value = inspection();
        value["RepoDigests"] = json!([format!("other.test/team/worker@{}", digest('c'))]);
        assert!(
            selected("registry.test/team/worker:v1", value)
                .unwrap()
                .metadata()
                .registry_manifest
                .is_none()
        );
    }

    #[rstest::rstest]
    #[case("docker.io/library/python:v1", "python")]
    #[case("index.docker.io/python:v1", "docker.io/library/python")]
    #[case("Registry.test/team/worker:v1", "registry.test/team/worker")]
    fn test_should_normalize_familiar_repository_evidence(
        #[case] requested: &str,
        #[case] repository: &str,
    ) {
        let mut value = inspection();
        value["RepoDigests"] = json!([format!("{repository}@{}", digest('c'))]);
        let image = selected(requested, value).unwrap();
        assert_eq!(
            image
                .metadata()
                .registry_manifest
                .as_ref()
                .unwrap()
                .digest(),
            Some(digest('b').as_str())
        );
    }

    #[test]
    fn test_should_select_legacy_manifest_evidence_deterministically() {
        let mut value = inspection();
        value.as_object_mut().unwrap().remove("Descriptor");
        value["RepoDigests"] = json!([
            format!("registry.test/team/worker@{}", digest('d')),
            format!("registry.test/team/worker@{}", digest('c'))
        ]);
        let image = selected("registry.test/team/worker:v1", value.clone()).unwrap();
        assert_eq!(
            image.resolved_uri(),
            format!("registry.test/team/worker@{}", digest('c'))
        );
        let image = selected(&format!("registry.test/team/worker@{}", digest('d')), value).unwrap();
        assert_eq!(
            image.resolved_uri(),
            format!("registry.test/team/worker@{}", digest('d'))
        );
    }

    #[rstest::rstest]
    #[case("os")]
    #[case("architecture")]
    #[case("id")]
    #[case("size_zero")]
    #[case("size_negative")]
    #[case("size_large")]
    #[case("descriptor_index")]
    #[case("descriptor_unknown")]
    #[case("descriptor_digest")]
    #[case("descriptor_arch")]
    #[case("descriptor_os")]
    #[case("descriptor_artifact")]
    #[case("descriptor_attestation")]
    #[case("digests_count")]
    #[case("digests_invalid")]
    #[case("digests_tag_only")]
    #[case("config_missing")]
    #[case("config_volumes")]
    #[case("config_nul")]
    #[case("config_empty_executable")]
    fn test_should_reject_incompatible_or_ambiguous_image_metadata(#[case] field: &str) {
        let mut value = inspection();
        match field {
            "os" => value["Os"] = json!("windows"),
            "architecture" => value["Architecture"] = json!("amd64"),
            "id" => value["Id"] = json!("worker:v1"),
            "size_zero" => value["Size"] = json!(0),
            "size_negative" => value["Size"] = json!(-1),
            "size_large" => value["Size"] = json!(10_u64 * 1024 * 1024 * 1024 + 1),
            "descriptor_index" => {
                value["Descriptor"]["mediaType"] = json!("application/vnd.oci.image.index.v1+json");
            }
            "descriptor_unknown" => value["Descriptor"]["mediaType"] = json!("application/unknown"),
            "descriptor_digest" => value["Descriptor"]["digest"] = json!("bad"),
            "descriptor_arch" => value["Descriptor"]["platform"]["architecture"] = json!("amd64"),
            "descriptor_os" => value["Descriptor"]["platform"]["os"] = json!("windows"),
            "descriptor_artifact" => {
                value["Descriptor"]["artifactType"] = json!("application/attestation");
            }
            "descriptor_attestation" => {
                value["Descriptor"]["annotations"] =
                    json!({"vnd.docker.reference.type":"attestation-manifest"});
            }
            "digests_count" => {
                value["RepoDigests"] =
                    json!(vec![
                        format!("registry.test/team/worker@{}", digest('c'));
                        129
                    ]);
            }
            "digests_invalid" => value["RepoDigests"] = json!(["garbage@digest"]),
            "digests_tag_only" => value["RepoDigests"] = json!(["registry.test/team/worker:v1"]),
            "config_missing" => value["Config"] = Value::Null,
            "config_volumes" => value["Config"]["Volumes"] = json!({"/data":{}}),
            "config_nul" => value["Config"]["Cmd"] = json!(["bad\0"]),
            "config_empty_executable" => {
                value["Config"]["Entrypoint"] = json!([]);
                value["Config"]["Cmd"] = json!([]);
            }
            _ => panic!("unknown test case"),
        }
        assert!(selected("registry.test/team/worker:v1", value).is_err());
    }

    #[test]
    fn test_should_hash_only_normalized_base_execution_identity() {
        let base = selected("registry.test/team/worker:v1", inspection()).unwrap();
        let mut value = inspection();
        value["Config"]["Labels"] = json!({"ignored":"changed"});
        value["Created"] = json!("2025-01-01T00:00:00Z");
        value["RepoDigests"] = json!([
            format!("unrelated.test/other@{}", digest('e')),
            format!("registry.test/team/worker@{}", digest('c'))
        ]);
        let same = selected("registry.test/team/worker:v2", value).unwrap();
        assert_eq!(
            base.metadata().inspection_hash,
            same.metadata().inspection_hash
        );
        for field in ["Entrypoint", "Cmd", "Env", "User", "WorkingDir"] {
            let mut value = inspection();
            value["Config"][field] = match field {
                "Entrypoint" | "Cmd" => json!(["changed"]),
                "Env" => json!(["SECRET=changed"]),
                "User" => json!("1001"),
                _ => json!("/changed"),
            };
            let changed = selected("registry.test/team/worker:v1", value).unwrap();
            assert_ne!(
                base.metadata().inspection_hash,
                changed.metadata().inspection_hash
            );
        }
        let mut value = inspection();
        value["Config"]["Entrypoint"] = json!([]);
        value["Config"]["Cmd"] = json!([]);
        let overrides = ImageConfig {
            command: Some(vec!["/provided".into()]),
            ..Default::default()
        };
        let image = resolve(
            &"registry.test/team/worker:v1".parse().unwrap(),
            Architecture::Arm64,
            Some(&overrides),
            serde_json::from_value(value).unwrap(),
            &digest('a').parse().unwrap(),
        )
        .unwrap();
        assert!(image.metadata().execution.plan(None).is_err());
        assert_eq!(
            image
                .metadata()
                .execution
                .plan(Some(&overrides))
                .unwrap()
                .command(),
            ["/provided"]
        );
    }
    #[test]
    fn test_should_require_consistent_classic_evidence_and_reject_manifest_summaries() {
        let mut value = inspection();
        value.as_object_mut().unwrap().remove("Descriptor");
        value["GraphDriver"] = json!({"Name":"overlay2","Data":{}});
        let inspected: ImageInspect = serde_json::from_value(value.clone()).unwrap();
        let classic: SystemInfo = serde_json::from_value(
            json!({"Driver":"overlay2","DriverStatus":[["Backing Filesystem","extfs"]]}),
        )
        .unwrap();
        verify_classic_store(&inspected, &classic).unwrap();
        for statuses in [
            json!({"Driver":"vfs"}),
            json!({"Driver":"vfs","DriverStatus":null}),
        ] {
            let mut vfs = inspected.clone();
            vfs.graph_driver.as_mut().unwrap().name = "vfs".into();
            verify_classic_store(&vfs, &serde_json::from_value(statuses).unwrap()).unwrap();
        }
        for name in ["btrfs", "zfs"] {
            let mut containerd = inspected.clone();
            containerd.graph_driver.as_mut().unwrap().name = name.into();
            let info:SystemInfo = serde_json::from_value(json!({"Driver":name,"DriverStatus":[["driver-type","io.containerd.snapshotter.v1"]]})).unwrap();
            assert!(verify_classic_store(&containerd, &info).is_err());
        }
        for info in [
            json!({"Driver":"overlayfs","DriverStatus":[["driver-type","io.containerd.snapshotter.v1"]]}),
            json!({"Driver":"overlay2","DriverStatus":[["driver-type","io.containerd.snapshotter.v1"]]}),
            json!({"Driver":"overlay2","DriverStatus":[["malformed"]]}),
        ] {
            assert!(
                verify_classic_store(&inspected, &serde_json::from_value(info).unwrap()).is_err()
            );
        }
        value["Manifests"] = json!([{"ID":digest('b'),"Descriptor":{},"Available":true,"Kind":"image","ImageData":null,"AttestationData":null,"Size":{"Content":1,"Total":1}}]);
        let inspected: ImageInspect = serde_json::from_value(value).unwrap();
        assert!(verify_classic_store(&inspected, &classic).is_err());
        assert!(selected_descriptor(&inspected, "arm64").is_err());
    }
}
