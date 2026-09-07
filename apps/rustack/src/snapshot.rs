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
#[cfg(feature = "cloudfront-dataplane")]
use rustack_cloudfront_dataplane::{CloudFrontCacheSnapshot, DataPlane};
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

    #[cfg(feature = "cloudfront-dataplane")]
    pub(crate) fn register_cloudfront_cache(&mut self, plane: DataPlane) {
        self.register(CloudFrontCacheSnapshotService { plane });
    }

    fn register<T>(&mut self, service: T)
    where
        T: SnapshotService + 'static,
    {
        self.services.push(Arc::new(service));
    }

    /// Explicit persistence coverage; workers are owned by the runtime supervisor.
    pub(crate) fn coverage(&self) -> Vec<(&'static str, &'static str)> {
        self.services
            .iter()
            .map(|service| {
                (
                    service.service_name(),
                    snapshot_coverage(service.service_name()),
                )
            })
            .collect()
    }

    fn services(&self) -> &[Arc<dyn SnapshotService>] {
        &self.services
    }
}

/// Human-facing coverage is not the binary archive's resource/data encoding kind.
pub(crate) fn snapshot_coverage(service: &str) -> &'static str {
    match service {
        "s3" | "dynamodb" => "resources-and-data",
        "dynamodbstreams" => "resources-and-records",
        "lambda" => "resources-and-code",
        "cloudfront-cache" | "cloudfront-dataplane" => "cache-data",
        "sqs" | "ssm" | "iam" | "apigatewayv2" | "cloudfront" => "resources-only",
        _ => "unsupported",
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

#[cfg(feature = "cloudfront-dataplane")]
struct CloudFrontCacheSnapshotService {
    plane: DataPlane,
}

#[cfg(feature = "cloudfront-dataplane")]
#[async_trait]
impl SnapshotService for CloudFrontCacheSnapshotService {
    fn service_name(&self) -> &'static str {
        "cloudfront-cache"
    }

    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Data
    }

    async fn save_meta(&self, data_staging_dir: &Path) -> Result<Vec<u8>> {
        let snapshot = self
            .plane
            .export_cache_snapshot(data_staging_dir)
            .await
            .context("export CloudFront data-plane cache snapshot")?;
        encode_state(&snapshot)
    }

    async fn load_meta(&self, state_cbor: &[u8], data_staging_dir: &Path) -> Result<()> {
        let snapshot: CloudFrontCacheSnapshot = decode_state(state_cbor)?;
        self.plane
            .import_cache_snapshot(snapshot, data_staging_dir)
            .await
            .context("import CloudFront data-plane cache snapshot")?;
        Ok(())
    }
}

/// Validated runtime snapshot configuration.
#[derive(Debug, Clone)]
pub(crate) struct SnapshotConfig {
    name: SnapshotName,
    root: PathBuf,
}

/// Advisory file lock retained for the full lifetime of a named snapshot runtime.
#[derive(Debug)]
#[allow(clippy::disallowed_types)] // OS advisory lock requires the blocking std File API.
pub(crate) struct SnapshotLease {
    _file: std::fs::File,
}

impl SnapshotConfig {
    /// Acquire an OS-released exclusive lease before loading or serving a named snapshot.
    ///
    /// The blocking std file API is required for advisory locking and is isolated
    /// inside a blocking worker; tokio offers no equivalent lock surface.
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    pub(crate) async fn acquire_lease(&self) -> Result<SnapshotLease> {
        let root = self.root.clone();
        let name = self.name.as_str().to_owned();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root).context("create snapshot root for lease")?;
            let lock_path = root.join(format!(".{name}.lock"));
            if std::fs::symlink_metadata(&lock_path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                bail!("snapshot lock path must not be a symbolic link");
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)
                .context("open snapshot lease")?;
            file.try_lock()
                .context("snapshot is already owned by another runtime")?;
            Ok(SnapshotLease { _file: file })
        })
        .await
        .context("snapshot lease task failed")?
    }

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
        cleanup_stale_staging(&self.root, self.name.as_str()).await?;
        recover_snapshot(&self.root, &dir, self.name.as_str()).await?;
        if !path_exists(&dir).await? {
            info!(snapshot = %self.name.as_str(), path = %dir.display(), "snapshot not found, starting empty");
            return Ok(());
        }

        let manifest_path = dir.join(MANIFEST_FILE);
        let manifest = read_manifest(&manifest_path).await?;
        if manifest.snapshot_name != self.name.as_str() {
            bail!(
                "snapshot manifest names {} but was requested as {}; refusing to import a renamed \
                 or copied directory",
                manifest.snapshot_name,
                self.name.as_str()
            );
        }
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
    let Ok(path) = rustack_core::settings::var(SNAPSHOT_PERF_FILE_ENV) else {
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
    rustack_core::settings::var(SNAPSHOT_ROOT_ENV)
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
    recover_snapshot(root, target, name).await?;
    validate_generation(temp, name)
        .await
        .context("validate prepared snapshot")?;
    sync_tree(temp).await?;
    let backup = root.join(format!(".{name}.previous"));
    let had_target = path_exists(target).await?;
    if had_target {
        fs::rename(target, &backup)
            .await
            .context("preserve previous committed snapshot")?;
        sync_directory(root).await?;
    }
    if let Err(error) = fs::rename(temp, target).await {
        if had_target {
            fs::rename(&backup, target).await.with_context(|| {
                format!(
                    "snapshot publication failed ({error}); recovery also failed; previous \
                     snapshot retained at {}",
                    backup.display()
                )
            })?;
            sync_directory(root).await?;
        }
        return Err(error).context("publish prepared snapshot");
    }
    sync_directory(root).await?;
    if had_target {
        remove_dir_if_exists(&backup).await?;
        sync_directory(root).await?;
    }
    Ok(())
}

/// Recover an interrupted publish, including the legacy randomly named backups.
async fn recover_snapshot(root: &Path, target: &Path, name: &str) -> Result<()> {
    if !path_exists(root).await? {
        return Ok(());
    }
    let previous = root.join(format!(".{name}.previous"));
    let backup = if path_exists(&previous).await? {
        Some(previous)
    } else if !path_exists(target).await? {
        let mut entries = fs::read_dir(root)
            .await
            .context("inspect snapshot recovery candidates")?;
        let prefix = format!(".{name}.bak.");
        let mut candidate = None;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                if candidate.is_some() {
                    bail!(
                        "multiple legacy snapshot backups require explicit operator recovery; \
                         refusing empty startup"
                    );
                }
                candidate = Some(entry.path());
            }
        }
        candidate
    } else {
        None
    };
    let Some(backup) = backup else {
        return Ok(());
    };
    if path_exists(target).await? && validate_generation(target, name).await.is_ok() {
        remove_dir_if_exists(&backup).await?;
        sync_directory(root).await?;
        return Ok(());
    }
    validate_generation(&backup, name)
        .await
        .context("previous snapshot is not a valid recovery generation")?;
    if path_exists(target).await? {
        let failed = root.join(format!(".{name}.failed.{}", unique_suffix()?));
        fs::rename(target, failed)
            .await
            .context("retain invalid snapshot for diagnosis")?;
    }
    fs::rename(&backup, target)
        .await
        .context("recover previous committed snapshot")?;
    sync_directory(root).await?;
    warn!(
        snapshot = name,
        "recovered previous committed snapshot after interrupted publication"
    );
    Ok(())
}

async fn cleanup_stale_staging(root: &Path, name: &str) -> Result<()> {
    let mut entries = fs::read_dir(root)
        .await
        .context("inspect snapshot staging leftovers")?;
    let prefixes = [
        format!(".{name}.tmp."),
        format!(".{name}.{LOAD_STAGING_PREFIX}."),
    ];
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if prefixes.iter().any(|prefix| file_name.starts_with(prefix))
            && entry.file_type().await?.is_dir()
        {
            remove_dir_if_exists(&entry.path()).await?;
        }
    }
    Ok(())
}

async fn validate_generation(directory: &Path, name: &str) -> Result<()> {
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).await?;
    if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION || manifest.snapshot_name != name {
        bail!("snapshot generation name/schema mismatch");
    }
    for entry in manifest.services.values() {
        let meta = read_archive(
            &snapshot_child(directory, &entry.meta_file)?,
            ArchiveKind::ServiceMeta,
        )
        .await?;
        get_required_section(&meta, SECTION_STATE_CBOR)?;
        if let Some(data) = &entry.data_file {
            read_archive(&snapshot_child(directory, data)?, ArchiveKind::ServiceData).await?;
        }
    }
    Ok(())
}

async fn sync_tree(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            if kind.is_symlink() {
                bail!("snapshot generation contains a symbolic link");
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                fs::File::open(entry.path()).await?.sync_all().await?;
            } else {
                bail!("snapshot generation contains a non-regular entry");
            }
        }
        directories.push(directory);
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory).await?;
    }
    Ok(())
}

async fn sync_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(directory)
        .await
        .context("open snapshot directory for durability")?
        .sync_all()
        .await
        .context("sync snapshot directory")?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
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

    async fn generation(root: &Path, folder: &str, version: &str) -> Result<PathBuf> {
        let directory = root.join(folder);
        fs::create_dir_all(&directory).await?;
        write_manifest(
            &directory.join(MANIFEST_FILE),
            &SnapshotManifest::new("dev", version)?,
        )
        .await?;
        Ok(directory)
    }

    #[tokio::test]
    async fn test_should_recover_gap_between_snapshot_renames() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = generation(root.path(), "dev", "old").await?;
        fs::rename(&target, root.path().join(".dev.previous")).await?;
        recover_snapshot(root.path(), &target, "dev").await?;
        assert_eq!(
            read_manifest(&target.join(MANIFEST_FILE))
                .await?
                .rustack_version,
            "old"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_keep_valid_new_generation_after_publish_crash() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = generation(root.path(), "dev", "new").await?;
        let previous = generation(root.path(), ".dev.previous", "old").await?;
        recover_snapshot(root.path(), &target, "dev").await?;
        assert_eq!(
            read_manifest(&target.join(MANIFEST_FILE))
                .await?
                .rustack_version,
            "new"
        );
        assert!(!path_exists(&previous).await?);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_recover_old_when_published_generation_is_invalid() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = generation(root.path(), "dev", "new").await?;
        generation(root.path(), ".dev.previous", "old").await?;
        fs::write(target.join(MANIFEST_FILE), b"corrupt").await?;
        recover_snapshot(root.path(), &target, "dev").await?;
        assert_eq!(
            read_manifest(&target.join(MANIFEST_FILE))
                .await?
                .rustack_version,
            "old"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_refuse_empty_start_with_invalid_recovery_generation() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join(".dev.previous")).await?;
        let config = SnapshotConfig {
            root: root.path().to_owned(),
            name: SnapshotName::try_from("dev".to_owned())?,
        };
        assert!(config.load(&RuntimeProviders::default()).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_recover_legacy_backup_and_refuse_ambiguous_backups() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("dev");
        generation(root.path(), ".dev.bak.legacy", "old").await?;
        recover_snapshot(root.path(), &target, "dev").await?;
        fs::rename(&target, root.path().join(".dev.bak.one")).await?;
        generation(root.path(), ".dev.bak.two", "other").await?;
        assert!(recover_snapshot(root.path(), &target, "dev").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_exclusively_lease_named_snapshot_until_owner_drops() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = SnapshotConfig {
            root: root.path().to_owned(),
            name: SnapshotName::try_from("dev".to_owned())?,
        };
        let lease = config.acquire_lease().await?;
        assert!(config.acquire_lease().await.is_err());
        drop(lease);
        let _next = config.acquire_lease().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_publish_valid_generation_and_preserve_old_on_invalid_prepare() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        let target = generation(root.path(), "dev", "old").await?;
        let staged = generation(root.path(), ".dev.tmp.new", "new").await?;
        replace_directory(&staged, &target, root.path(), "dev").await?;
        assert_eq!(
            read_manifest(&target.join(MANIFEST_FILE))
                .await?
                .rustack_version,
            "new"
        );
        let invalid = root.path().join(".dev.tmp.invalid");
        fs::create_dir(&invalid).await?;
        assert!(
            replace_directory(&invalid, &target, root.path(), "dev")
                .await
                .is_err()
        );
        assert_eq!(
            read_manifest(&target.join(MANIFEST_FILE))
                .await?
                .rustack_version,
            "new"
        );
        Ok(())
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
