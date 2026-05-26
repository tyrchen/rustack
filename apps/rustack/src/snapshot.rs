//! Runtime snapshot loading and saving for the Rustack gateway.

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use archive::{
    ArchiveKind, ArchiveSection, ArchiveStats, SECTION_MANIFEST_CBOR, SECTION_STATE_CBOR,
    from_cbor, get_required_section, pack_data_archive, read_archive, to_cbor, unpack_data_archive,
    validated_relative_path, write_archive,
};
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
use tokio::{fs, io::AsyncWriteExt as _, sync::Semaphore, task::JoinSet};
use tracing::{info, warn};

mod archive;

const SNAPSHOT_SCHEMA_VERSION: u32 = 2;
const SNAPSHOT_ROOT_ENV: &str = "RUSTACK_SNAPSHOT_DIR";
const SNAPSHOT_PERF_FILE_ENV: &str = "RUSTACK_SNAPSHOT_PERF_FILE";
const DEFAULT_SNAPSHOT_ROOT: &str = ".rustack/snapshots";
const MANIFEST_FILE: &str = "manifest.ss.zst";
const META_FILE: &str = "meta.ss.zst";
const DATA_FILE: &str = "data.ss.zst";
const SERVICES_DIR: &str = "services";
const STAGING_DIR: &str = ".staging";
const LOAD_STAGING_PREFIX: &str = ".load";
const SNAPSHOT_SERVICE_PARALLELISM: usize = 4;

/// Registry of services that support runtime snapshots.
#[derive(Default)]
pub(crate) struct RuntimeProviders {
    services: Vec<Arc<dyn SnapshotService>>,
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
        self.services.push(Arc::new(service));
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

    fn services(&self) -> &[Arc<dyn SnapshotService>] {
        &self.services
    }
}

#[async_trait]
trait SnapshotService: Send + Sync {
    fn service_name(&self) -> &'static str;

    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Resource
    }

    async fn save_meta(&self, data_staging_dir: &Path) -> Result<Vec<u8>>;

    async fn load_meta(&self, state_cbor: &[u8], data_staging_dir: &Path) -> Result<()>;

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

    async fn save_meta(&self, data_staging_dir: &Path) -> Result<Vec<u8>> {
        let snapshot = self.provider.export_snapshot(data_staging_dir).await?;
        encode_state(&snapshot)
    }

    async fn load_meta(&self, state_cbor: &[u8], data_staging_dir: &Path) -> Result<()> {
        let snapshot: S3Snapshot = decode_state(state_cbor)?;
        self.provider
            .import_snapshot(snapshot, data_staging_dir)
            .await?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: DynamoDBSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.store.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: StreamStoreSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot().await?)
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: SqsSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: ParameterStoreSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: IamStoreSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: LambdaSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: ApiStoreSnapshot = decode_state(state_cbor)?;
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

    async fn save_meta(&self, _data_staging_dir: &Path) -> Result<Vec<u8>> {
        encode_state(&self.provider.export_snapshot())
    }

    async fn load_meta(&self, state_cbor: &[u8], _data_staging_dir: &Path) -> Result<()> {
        let snapshot: CloudFrontStoreSnapshot = decode_state(state_cbor)?;
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
        let started = Instant::now();
        let dir = self.snapshot_dir();
        if !path_exists(&dir).await? {
            info!(snapshot = %self.name.as_str(), path = %dir.display(), "snapshot not found, starting empty");
            return Ok(());
        }

        let manifest_path = dir.join(MANIFEST_FILE);
        let manifest = read_manifest(&manifest_path).await?;
        if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION {
            bail!(
                "unsupported snapshot schema version {} in {}",
                manifest.schema_version,
                manifest_path.display()
            );
        }

        let load_staging = self.root.join(format!(
            ".{}.{}.{}",
            self.name.as_str(),
            LOAD_STAGING_PREFIX,
            unique_suffix()?
        ));
        remove_dir_if_exists(&load_staging).await?;
        fs::create_dir_all(&load_staging).await.with_context(|| {
            format!(
                "failed to create snapshot load staging dir {}",
                load_staging.display()
            )
        })?;

        let mut join_set = JoinSet::new();
        let semaphore = Arc::new(Semaphore::new(SNAPSHOT_SERVICE_PARALLELISM));
        for service in providers.services() {
            let service_name = service.service_name();
            if let Some(entry) = manifest.services.get(service_name) {
                let service = Arc::clone(service);
                let entry = entry.clone();
                let snapshot_dir = dir.clone();
                let staging_root = load_staging.clone();
                let semaphore = Arc::clone(&semaphore);
                join_set.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .context("snapshot load semaphore closed")?;
                    load_service_snapshot(service, snapshot_dir, staging_root, entry).await
                });
            }
        }
        let load_result = async {
            while let Some(result) = join_set.join_next().await {
                result.context("snapshot load task failed")??;
            }
            Result::<()>::Ok(())
        }
        .await;
        let cleanup_result = remove_dir_if_exists(&load_staging).await;
        load_result?;
        cleanup_result?;

        record_snapshot_timing("load_ms", started.elapsed()).await;
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
        let started = Instant::now();
        fs::create_dir_all(&self.root)
            .await
            .with_context(|| format!("failed to create snapshot root {}", self.root.display()))?;

        let target = self.snapshot_dir();
        let suffix = unique_suffix()?;
        let temp = self
            .root
            .join(format!(".{}.tmp.{suffix}", self.name.as_str()));
        remove_dir_if_exists(&temp).await?;
        fs::create_dir_all(temp.join(SERVICES_DIR))
            .await
            .with_context(|| format!("failed to create snapshot temp dir {}", temp.display()))?;
        fs::create_dir_all(temp.join(STAGING_DIR))
            .await
            .with_context(|| format!("failed to create snapshot staging dir {}", temp.display()))?;

        let mut manifest = SnapshotManifest::new(self.name.as_str(), version)?;
        let mut join_set = JoinSet::new();
        let semaphore = Arc::new(Semaphore::new(SNAPSHOT_SERVICE_PARALLELISM));

        for service in providers.services() {
            let service = Arc::clone(service);
            let snapshot_dir = temp.clone();
            let semaphore = Arc::clone(&semaphore);
            join_set.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .context("snapshot save semaphore closed")?;
                save_service_snapshot(service, snapshot_dir).await
            });
        }
        while let Some(result) = join_set.join_next().await {
            let service = result.context("snapshot save task failed")??;
            manifest.add_service(service.name, service.manifest);
        }
        remove_dir_if_exists(&temp.join(STAGING_DIR)).await?;

        write_manifest(&temp.join(MANIFEST_FILE), &manifest).await?;
        replace_directory(&temp, &target, &self.root, self.name.as_str()).await?;

        record_snapshot_timing("save_ms", started.elapsed()).await;
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
    saved_at_unix_millis: u64,
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

    fn add_service(&mut self, service: String, manifest: SnapshotServiceManifest) {
        self.services.insert(service, manifest);
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
    kind: SnapshotKind,
    meta_file: String,
    data_file: Option<String>,
    meta: ArchiveStats,
    data: Option<ArchiveStats>,
}

#[derive(Debug)]
struct ServiceSaveResult {
    name: String,
    manifest: SnapshotServiceManifest,
}

async fn save_service_snapshot(
    service: Arc<dyn SnapshotService>,
    snapshot_dir: PathBuf,
) -> Result<ServiceSaveResult> {
    let service_name = service.service_name();
    let started = Instant::now();
    let service_dir = snapshot_dir.join(SERVICES_DIR).join(service_name);
    let staging_dir = snapshot_dir.join(STAGING_DIR).join(service_name);
    fs::create_dir_all(&service_dir).await.with_context(|| {
        format!(
            "failed to create snapshot service dir {}",
            service_dir.display()
        )
    })?;
    fs::create_dir_all(&staging_dir).await.with_context(|| {
        format!(
            "failed to create snapshot staging dir {}",
            staging_dir.display()
        )
    })?;

    let state_cbor = service
        .save_meta(&staging_dir)
        .await
        .with_context(|| format!("failed to save snapshot metadata for {service_name}"))?;
    let meta_path = service_dir.join(META_FILE);
    let meta_stats = write_archive(
        &meta_path,
        ArchiveKind::ServiceMeta,
        vec![ArchiveSection::new(SECTION_STATE_CBOR, state_cbor, 1)],
    )
    .await
    .with_context(|| format!("failed to write snapshot metadata archive for {service_name}"))?;

    let data_path = service_dir.join(DATA_FILE);
    let data_stats = pack_data_archive(&staging_dir, &data_path)
        .await
        .with_context(|| format!("failed to write snapshot data archive for {service_name}"))?;
    remove_dir_if_exists(&staging_dir).await?;

    let data_file = data_stats
        .as_ref()
        .map(|_| format!("{SERVICES_DIR}/{service_name}/{DATA_FILE}"));
    let manifest = SnapshotServiceManifest {
        kind: service.snapshot_kind(),
        meta_file: format!("{SERVICES_DIR}/{service_name}/{META_FILE}"),
        data_file,
        meta: meta_stats,
        data: data_stats,
    };
    info!(
        service = service_name,
        elapsed_ms = started.elapsed().as_millis(),
        meta_compressed_bytes = manifest.meta.compressed_bytes,
        data_compressed_bytes = manifest.data.map_or(0, |stats| stats.compressed_bytes),
        "saved service snapshot",
    );
    Ok(ServiceSaveResult {
        name: service_name.to_owned(),
        manifest,
    })
}

async fn load_service_snapshot(
    service: Arc<dyn SnapshotService>,
    snapshot_dir: PathBuf,
    staging_root: PathBuf,
    manifest: SnapshotServiceManifest,
) -> Result<()> {
    let service_name = service.service_name();
    let started = Instant::now();
    let staging_dir = staging_root.join(service_name);
    remove_dir_if_exists(&staging_dir).await?;
    fs::create_dir_all(&staging_dir).await.with_context(|| {
        format!(
            "failed to create snapshot staging dir {}",
            staging_dir.display()
        )
    })?;

    if let Some(data_file) = manifest.data_file.as_ref() {
        let data_path = snapshot_child(&snapshot_dir, data_file)?;
        unpack_data_archive(&data_path, &staging_dir)
            .await
            .with_context(|| format!("failed to load snapshot data archive for {service_name}"))?;
    }

    let meta_path = snapshot_child(&snapshot_dir, &manifest.meta_file)?;
    let sections = read_archive(&meta_path, ArchiveKind::ServiceMeta)
        .await
        .with_context(|| format!("failed to read snapshot metadata archive for {service_name}"))?;
    let state_cbor = get_required_section(&sections, SECTION_STATE_CBOR)?;
    service
        .load_meta(state_cbor, &staging_dir)
        .await
        .with_context(|| format!("failed to load snapshot metadata for {service_name}"))?;

    remove_dir_if_exists(&staging_dir).await?;
    info!(
        service = service_name,
        elapsed_ms = started.elapsed().as_millis(),
        meta_compressed_bytes = manifest.meta.compressed_bytes,
        data_compressed_bytes = manifest.data.map_or(0, |stats| stats.compressed_bytes),
        "loaded service snapshot",
    );
    Ok(())
}

fn encode_state<T>(value: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    to_cbor(value)
}

fn decode_state<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    from_cbor(bytes)
}

async fn write_manifest(path: &Path, manifest: &SnapshotManifest) -> Result<()> {
    let manifest_cbor = to_cbor(manifest)?;
    write_archive(
        path,
        ArchiveKind::Manifest,
        vec![ArchiveSection::new(SECTION_MANIFEST_CBOR, manifest_cbor, 1)],
    )
    .await
    .with_context(|| format!("failed to write snapshot manifest {}", path.display()))?;
    Ok(())
}

async fn read_manifest(path: &Path) -> Result<SnapshotManifest> {
    let sections = read_archive(path, ArchiveKind::Manifest)
        .await
        .with_context(|| format!("failed to read snapshot manifest {}", path.display()))?;
    let manifest_cbor = get_required_section(&sections, SECTION_MANIFEST_CBOR)?;
    from_cbor(manifest_cbor)
}

async fn record_snapshot_timing(metric: &str, elapsed: Duration) {
    let Ok(path) = std::env::var(SNAPSHOT_PERF_FILE_ENV) else {
        return;
    };
    let line = format!("{metric}={}\n", elapsed.as_millis());
    let result = async {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        file.write_all(line.as_bytes()).await
    }
    .await;
    if let Err(error) = result {
        warn!(path = %path, error = %error, "failed to record snapshot timing");
    }
}

fn snapshot_root() -> PathBuf {
    std::env::var(SNAPSHOT_ROOT_ENV)
        .map_or_else(|_| PathBuf::from(DEFAULT_SNAPSHOT_ROOT), PathBuf::from)
}

fn snapshot_child(root: &Path, relative: &str) -> Result<PathBuf> {
    Ok(root.join(validated_relative_path(relative)?))
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

fn now_unix_millis() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_millis();
    u64::try_from(millis).context("current UNIX millis do not fit in u64")
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
        let path = snapshot_child(Path::new("/tmp/snapshot"), "services/s3/meta.ss.zst");
        assert!(path.is_ok());
    }

    #[tokio::test]
    async fn test_should_round_trip_manifest_archive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(MANIFEST_FILE);
        let mut manifest = SnapshotManifest::new("dev", "test-version")?;
        manifest.add_service(
            "s3".to_owned(),
            SnapshotServiceManifest {
                kind: SnapshotKind::Data,
                meta_file: "services/s3/meta.ss.zst".to_owned(),
                data_file: Some("services/s3/data.ss.zst".to_owned()),
                meta: ArchiveStats {
                    compressed_bytes: 10,
                    uncompressed_bytes: 20,
                },
                data: Some(ArchiveStats {
                    compressed_bytes: 30,
                    uncompressed_bytes: 40,
                }),
            },
        );

        write_manifest(&path, &manifest).await?;
        let loaded = read_manifest(&path).await?;
        assert_eq!(loaded.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(loaded.services.contains_key("s3"));
        Ok(())
    }
}
