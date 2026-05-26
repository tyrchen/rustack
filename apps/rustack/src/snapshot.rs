//! Runtime snapshot loading and saving for the Rustack gateway.

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
#[cfg(feature = "apigatewayv2")]
use rustack_apigatewayv2_core::{provider::RustackApiGatewayV2, storage::ApiStoreSnapshot};
#[cfg(feature = "cloudfront")]
use rustack_cloudfront_core::{RustackCloudFront, store::CloudFrontStoreSnapshot};
#[cfg(feature = "dynamodb")]
use rustack_dynamodb_core::{provider::RustackDynamoDB, snapshot::DynamoDBSnapshot};
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::storage::{StreamStore, StreamStoreSnapshot};
#[cfg(feature = "iam")]
use rustack_iam_core::{provider::RustackIam, store::IamStoreSnapshot};
#[cfg(feature = "lambda")]
use rustack_lambda_core::provider::{LambdaSnapshot, RustackLambda};
#[cfg(feature = "s3")]
use rustack_s3_core::{RustackS3, snapshot::S3Snapshot};
#[cfg(feature = "sqs")]
use rustack_sqs_core::provider::{RustackSqs, SqsSnapshot};
#[cfg(feature = "ssm")]
use rustack_ssm_core::{provider::RustackSsm, storage::ParameterStoreSnapshot};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::fs;
use tracing::{info, warn};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_ROOT_ENV: &str = "RUSTACK_SNAPSHOT_DIR";
const DEFAULT_SNAPSHOT_ROOT: &str = ".rustack/snapshots";
const MANIFEST_FILE: &str = "manifest.json";

/// Registry of services that support runtime snapshots.
#[derive(Default)]
pub(crate) struct RuntimeProviders {
    services: Vec<Box<dyn SnapshotService>>,
}

impl RuntimeProviders {
    #[cfg(feature = "s3")]
    pub(crate) fn register_s3(&mut self, provider: Arc<RustackS3>) {
        self.register(S3SnapshotService { provider });
    }

    #[cfg(feature = "dynamodb")]
    pub(crate) fn register_dynamodb(&mut self, provider: Arc<RustackDynamoDB>) {
        self.register(DynamoDBSnapshotService { provider });
    }

    #[cfg(feature = "dynamodbstreams")]
    pub(crate) fn register_dynamodb_streams(&mut self, store: Arc<StreamStore>) {
        self.register(DynamoDBStreamsSnapshotService { store });
    }

    #[cfg(feature = "sqs")]
    pub(crate) fn register_sqs(&mut self, provider: Arc<RustackSqs>) {
        self.register(SqsSnapshotService { provider });
    }

    #[cfg(feature = "ssm")]
    pub(crate) fn register_ssm(&mut self, provider: Arc<RustackSsm>) {
        self.register(SsmSnapshotService { provider });
    }

    #[cfg(feature = "iam")]
    pub(crate) fn register_iam(&mut self, provider: Arc<RustackIam>) {
        self.register(IamSnapshotService { provider });
    }

    #[cfg(feature = "lambda")]
    pub(crate) fn register_lambda(&mut self, provider: Arc<RustackLambda>) {
        self.register(LambdaSnapshotService { provider });
    }

    #[cfg(feature = "apigatewayv2")]
    pub(crate) fn register_apigatewayv2(&mut self, provider: Arc<RustackApiGatewayV2>) {
        self.register(ApiGatewayV2SnapshotService { provider });
    }

    #[cfg(feature = "cloudfront")]
    pub(crate) fn register_cloudfront(&mut self, provider: Arc<RustackCloudFront>) {
        self.register(CloudFrontSnapshotService { provider });
    }

    fn register<T>(&mut self, service: T)
    where
        T: SnapshotService + 'static,
    {
        self.services.push(Box::new(service));
    }

    /// Stop stateful provider background workers after snapshot save.
    pub(crate) async fn shutdown(&self) {
        for service in &self.services {
            if let Err(error) = service.shutdown().await {
                warn!(
                    service = service.service_name(),
                    error = %error,
                    "snapshot service shutdown failed",
                );
            }
        }
    }

    fn services(&self) -> &[Box<dyn SnapshotService>] {
        &self.services
    }
}

#[async_trait]
trait SnapshotService: Send + Sync {
    fn service_name(&self) -> &'static str;

    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Resource
    }

    async fn save_state(&self, state_file: &Path, data_dir: &Path) -> Result<()>;

    async fn save_data(&self, _data_dir: &Path) -> Result<()> {
        Ok(())
    }

    async fn load_data(&self, _data_dir: &Path) -> Result<()> {
        Ok(())
    }

    async fn load_state(&self, state_file: &Path, data_dir: &Path) -> Result<()>;

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "s3")]
struct S3SnapshotService {
    provider: Arc<RustackS3>,
}

#[cfg(feature = "s3")]
#[async_trait]
impl SnapshotService for S3SnapshotService {
    fn service_name(&self) -> &'static str {
        "s3"
    }

    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Data
    }

    async fn save_state(&self, state_file: &Path, data_dir: &Path) -> Result<()> {
        let snapshot = self.provider.export_snapshot(data_dir).await?;
        write_json(state_file, &snapshot).await
    }

    async fn load_state(&self, state_file: &Path, data_dir: &Path) -> Result<()> {
        let snapshot: S3Snapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot, data_dir).await?;
        Ok(())
    }
}

#[cfg(feature = "dynamodb")]
struct DynamoDBSnapshotService {
    provider: Arc<RustackDynamoDB>,
}

#[cfg(feature = "dynamodb")]
#[async_trait]
impl SnapshotService for DynamoDBSnapshotService {
    fn service_name(&self) -> &'static str {
        "dynamodb"
    }

    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Data
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: DynamoDBSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot)?;
        Ok(())
    }
}

#[cfg(feature = "dynamodbstreams")]
struct DynamoDBStreamsSnapshotService {
    store: Arc<StreamStore>,
}

#[cfg(feature = "dynamodbstreams")]
#[async_trait]
impl SnapshotService for DynamoDBStreamsSnapshotService {
    fn service_name(&self) -> &'static str {
        "dynamodbstreams"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.store.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: StreamStoreSnapshot = read_json(state_file).await?;
        self.store.import_snapshot(snapshot);
        Ok(())
    }
}

#[cfg(feature = "sqs")]
struct SqsSnapshotService {
    provider: Arc<RustackSqs>,
}

#[cfg(feature = "sqs")]
#[async_trait]
impl SnapshotService for SqsSnapshotService {
    fn service_name(&self) -> &'static str {
        "sqs"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot().await?).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: SqsSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        self.provider.shutdown_all().await;
        Ok(())
    }
}

#[cfg(feature = "ssm")]
struct SsmSnapshotService {
    provider: Arc<RustackSsm>,
}

#[cfg(feature = "ssm")]
#[async_trait]
impl SnapshotService for SsmSnapshotService {
    fn service_name(&self) -> &'static str {
        "ssm"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: ParameterStoreSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot);
        Ok(())
    }
}

#[cfg(feature = "iam")]
struct IamSnapshotService {
    provider: Arc<RustackIam>,
}

#[cfg(feature = "iam")]
#[async_trait]
impl SnapshotService for IamSnapshotService {
    fn service_name(&self) -> &'static str {
        "iam"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: IamStoreSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot);
        Ok(())
    }
}

#[cfg(feature = "lambda")]
struct LambdaSnapshotService {
    provider: Arc<RustackLambda>,
}

#[cfg(feature = "lambda")]
#[async_trait]
impl SnapshotService for LambdaSnapshotService {
    fn service_name(&self) -> &'static str {
        "lambda"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: LambdaSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        self.provider.shutdown().await;
        Ok(())
    }
}

#[cfg(feature = "apigatewayv2")]
struct ApiGatewayV2SnapshotService {
    provider: Arc<RustackApiGatewayV2>,
}

#[cfg(feature = "apigatewayv2")]
#[async_trait]
impl SnapshotService for ApiGatewayV2SnapshotService {
    fn service_name(&self) -> &'static str {
        "apigatewayv2"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: ApiStoreSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot);
        Ok(())
    }
}

#[cfg(feature = "cloudfront")]
struct CloudFrontSnapshotService {
    provider: Arc<RustackCloudFront>,
}

#[cfg(feature = "cloudfront")]
#[async_trait]
impl SnapshotService for CloudFrontSnapshotService {
    fn service_name(&self) -> &'static str {
        "cloudfront"
    }

    async fn save_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        write_json(state_file, &self.provider.export_snapshot()).await
    }

    async fn load_state(&self, state_file: &Path, _data_dir: &Path) -> Result<()> {
        let snapshot: CloudFrontStoreSnapshot = read_json(state_file).await?;
        self.provider.import_snapshot(snapshot);
        Ok(())
    }
}

/// Validated runtime snapshot configuration.
#[derive(Debug, Clone)]
pub(crate) struct SnapshotConfig {
    name: SnapshotName,
    root: PathBuf,
}

impl SnapshotConfig {
    /// Build snapshot configuration from a CLI-provided name.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is invalid.
    pub(crate) fn from_name(name: String) -> Result<Self> {
        Ok(Self {
            name: SnapshotName::try_from(name)?,
            root: snapshot_root(),
        })
    }

    /// Load an existing snapshot into the given providers.
    ///
    /// Missing snapshot directories are treated as an empty starting state so
    /// `rustack --snapshot name` can create the snapshot on shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot exists but cannot be parsed or applied.
    pub(crate) async fn load(&self, providers: &RuntimeProviders) -> Result<()> {
        let dir = self.snapshot_dir();
        if !path_exists(&dir).await? {
            info!(snapshot = %self.name.as_str(), path = %dir.display(), "snapshot not found, starting empty");
            return Ok(());
        }

        let manifest_path = dir.join(MANIFEST_FILE);
        let manifest: SnapshotManifest = read_json(&manifest_path).await?;
        if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION {
            bail!(
                "unsupported snapshot schema version {} in {}",
                manifest.schema_version,
                manifest_path.display()
            );
        }

        for service in providers.services() {
            let service_name = service.service_name();
            if let Some(entry) = manifest.services.get(service_name) {
                let state_file = snapshot_child(&dir, &entry.file)?;
                let data_dir = dir.join("data").join(service_name);
                service
                    .load_data(&data_dir)
                    .await
                    .with_context(|| format!("failed to load snapshot data for {service_name}"))?;
                service
                    .load_state(&state_file, &data_dir)
                    .await
                    .with_context(|| format!("failed to load snapshot state for {service_name}"))?;
            }
        }

        info!(snapshot = %self.name.as_str(), path = %dir.display(), "loaded runtime snapshot");
        Ok(())
    }

    /// Save provider state into this snapshot, replacing any prior contents.
    ///
    /// # Errors
    ///
    /// Returns an error if state export, JSON serialization, or atomic directory
    /// replacement fails.
    pub(crate) async fn save(&self, providers: &RuntimeProviders, version: &str) -> Result<()> {
        fs::create_dir_all(&self.root)
            .await
            .with_context(|| format!("failed to create snapshot root {}", self.root.display()))?;

        let target = self.snapshot_dir();
        let suffix = unique_suffix()?;
        let temp = self
            .root
            .join(format!(".{}.tmp.{suffix}", self.name.as_str()));
        remove_dir_if_exists(&temp).await?;
        fs::create_dir_all(temp.join("resources"))
            .await
            .with_context(|| format!("failed to create snapshot temp dir {}", temp.display()))?;
        fs::create_dir_all(temp.join("data"))
            .await
            .with_context(|| format!("failed to create snapshot data dir {}", temp.display()))?;

        let mut manifest = SnapshotManifest::new(self.name.as_str(), version)?;

        for service in providers.services() {
            let service_name = service.service_name();
            let file = format!("resources/{service_name}.json");
            let state_file = temp.join(&file);
            let data_dir = temp.join("data").join(service_name);
            fs::create_dir_all(&data_dir).await.with_context(|| {
                format!("failed to create snapshot data dir {}", data_dir.display())
            })?;
            service
                .save_state(&state_file, &data_dir)
                .await
                .with_context(|| format!("failed to save snapshot state for {service_name}"))?;
            service
                .save_data(&data_dir)
                .await
                .with_context(|| format!("failed to save snapshot data for {service_name}"))?;
            manifest.add_service(service_name, &file, service.snapshot_kind());
        }

        write_json(&temp.join(MANIFEST_FILE), &manifest).await?;
        replace_directory(&temp, &target, &self.root, self.name.as_str()).await?;

        info!(snapshot = %self.name.as_str(), path = %target.display(), "saved runtime snapshot");
        Ok(())
    }

    fn snapshot_dir(&self) -> PathBuf {
        self.root.join(self.name.as_str())
    }
}

#[derive(Debug, Clone)]
struct SnapshotName(String);

impl SnapshotName {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SnapshotName {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let invalid = value.is_empty()
            || value.len() > 64
            || value == "."
            || value == ".."
            || value.contains("..")
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if invalid {
            bail!(
                "invalid snapshot name '{value}'; use 1-64 ASCII letters, digits, '.', '_' or '-'"
            );
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotManifest {
    schema_version: u32,
    snapshot_name: String,
    created_by: String,
    rustack_version: String,
    saved_at_unix_millis: u128,
    services: BTreeMap<String, SnapshotServiceManifest>,
}

impl SnapshotManifest {
    fn new(snapshot_name: &str, rustack_version: &str) -> Result<Self> {
        Ok(Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            snapshot_name: snapshot_name.to_owned(),
            created_by: "rustack".to_owned(),
            rustack_version: rustack_version.to_owned(),
            saved_at_unix_millis: now_unix_millis()?,
            services: BTreeMap::new(),
        })
    }

    fn add_service(&mut self, service: &str, file: &str, kind: SnapshotKind) {
        self.services.insert(
            service.to_owned(),
            SnapshotServiceManifest {
                file: file.to_owned(),
                kind,
            },
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum SnapshotKind {
    Resource,
    Data,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotServiceManifest {
    file: String,
    kind: SnapshotKind,
}

fn snapshot_root() -> PathBuf {
    std::env::var(SNAPSHOT_ROOT_ENV)
        .map_or_else(|_| PathBuf::from(DEFAULT_SNAPSHOT_ROOT), PathBuf::from)
}

fn snapshot_child(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("snapshot manifest path must be a relative child path: {relative}");
    }
    Ok(root.join(path))
}

async fn read_json<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let bytes = fs::read(path)
        .await
        .with_context(|| format!("failed to read snapshot file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse snapshot file {}", path.display()))
}

async fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize snapshot file {}", path.display()))?;
    fs::write(path, bytes)
        .await
        .with_context(|| format!("failed to write snapshot file {}", path.display()))
}

async fn path_exists(path: &Path) -> Result<bool> {
    match fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect snapshot path {}", path.display())),
    }
}

async fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove directory {}", path.display()))
        }
    }
}

async fn replace_directory(temp: &Path, target: &Path, root: &Path, name: &str) -> Result<()> {
    let suffix = unique_suffix()?;
    let backup = root.join(format!(".{name}.bak.{suffix}"));
    remove_dir_if_exists(&backup).await?;

    let had_target = path_exists(target).await?;
    if had_target {
        fs::rename(target, &backup).await.with_context(|| {
            format!(
                "failed to move existing snapshot {} to backup {}",
                target.display(),
                backup.display()
            )
        })?;
    }

    match fs::rename(temp, target).await {
        Ok(()) => {
            if had_target {
                remove_dir_if_exists(&backup).await?;
            }
            Ok(())
        }
        Err(error) => {
            if had_target {
                let _ = fs::rename(&backup, target).await;
            }
            Err(error).with_context(|| {
                format!(
                    "failed to replace snapshot {} with {}",
                    target.display(),
                    temp.display()
                )
            })
        }
    }
}

fn unique_suffix() -> Result<String> {
    Ok(format!("{}-{}", std::process::id(), now_unix_millis()?))
}

fn now_unix_millis() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_accept_valid_snapshot_name() {
        let name = SnapshotName::try_from("dev.snapshot-1".to_owned());
        assert!(name.is_ok());
    }

    #[test]
    fn test_should_reject_snapshot_name_with_path_traversal() {
        let name = SnapshotName::try_from("../prod".to_owned());
        assert!(name.is_err());
    }

    #[test]
    fn test_should_reject_empty_snapshot_name() {
        let name = SnapshotName::try_from(String::new());
        assert!(name.is_err());
    }

    #[test]
    fn test_should_reject_manifest_path_traversal() {
        let path = snapshot_child(Path::new("/tmp/snapshot"), "../outside.json");
        assert!(path.is_err());
    }

    #[test]
    fn test_should_accept_manifest_child_path() {
        let path = snapshot_child(Path::new("/tmp/snapshot"), "resources/s3.json");
        assert!(path.is_ok());
    }
}
