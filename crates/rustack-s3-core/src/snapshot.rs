//! Snapshot support for S3 buckets, metadata, multipart uploads, and object bodies.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt as _;

use crate::{
    provider::RustackS3,
    state::{
        bucket::{
            BucketEncryption, CorsRuleConfig, ObjectLockConfiguration, OwnershipControlsConfig,
            PublicAccessBlockConfig, S3Bucket, VersioningStatus, WebsiteConfig,
        },
        multipart::MultipartUpload,
        object::{ObjectVersion, Owner, S3DeleteMarker, S3Object},
    },
};

/// Errors raised while exporting or importing S3 snapshots.
#[derive(Debug, Error)]
pub enum S3SnapshotError {
    /// Bucket referenced in the bucket list disappeared during export.
    #[error("bucket disappeared during snapshot export: {bucket}")]
    BucketDisappeared {
        /// Bucket name.
        bucket: String,
    },
    /// Object data could not be read through the storage backend.
    #[error("failed to read S3 object data for {bucket}/{key}@{version_id}: {source}")]
    ReadObject {
        /// Bucket name.
        bucket: String,
        /// Object key.
        key: String,
        /// Version identifier.
        version_id: String,
        /// Source error.
        #[source]
        source: Box<crate::error::S3ServiceError>,
    },
    /// Multipart part data could not be read through the storage backend.
    #[error("failed to read S3 multipart data for {bucket}/{upload_id}/{part_number}: {source}")]
    ReadPart {
        /// Bucket name.
        bucket: String,
        /// Multipart upload identifier.
        upload_id: String,
        /// Part number.
        part_number: u32,
        /// Source error.
        #[source]
        source: Box<crate::error::S3ServiceError>,
    },
    /// Object data could not be written through the storage backend.
    #[error("failed to restore S3 object data for {bucket}/{key}@{version_id}: {source}")]
    WriteObject {
        /// Bucket name.
        bucket: String,
        /// Object key.
        key: String,
        /// Version identifier.
        version_id: String,
        /// Source error.
        #[source]
        source: Box<crate::error::S3ServiceError>,
    },
    /// Multipart part data could not be written through the storage backend.
    #[error("failed to restore S3 multipart data for {bucket}/{upload_id}/{part_number}: {source}")]
    WritePart {
        /// Bucket name.
        bucket: String,
        /// Multipart upload identifier.
        upload_id: String,
        /// Part number.
        part_number: u32,
        /// Source error.
        #[source]
        source: Box<crate::error::S3ServiceError>,
    },
    /// File-system I/O failed.
    #[error("S3 snapshot I/O failed at {path}: {source}")]
    Io {
        /// Path being accessed.
        path: String,
        /// Source error.
        #[source]
        source: std::io::Error,
    },
    /// Snapshot metadata referenced a data path outside its data directory.
    #[error("invalid S3 snapshot data path: {path}")]
    InvalidDataPath {
        /// Invalid relative path.
        path: String,
    },
}

/// Serializable S3 service snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Snapshot {
    /// Bucket snapshots.
    pub buckets: Vec<S3BucketSnapshot>,
}

/// Serializable S3 bucket snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3BucketSnapshot {
    /// Bucket name.
    pub name: String,
    /// Bucket region.
    pub region: String,
    /// Creation date.
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// Bucket owner.
    pub owner: Owner,
    /// Whether the object store is versioned internally.
    pub object_store_versioned: bool,
    /// Object versions and delete markers.
    pub object_versions: Vec<S3ObjectVersionSnapshot>,
    /// In-progress multipart uploads.
    pub multipart_uploads: Vec<S3MultipartUploadSnapshot>,
    /// Bucket versioning status.
    pub versioning: VersioningStatus,
    /// Bucket encryption settings.
    pub encryption: Option<BucketEncryption>,
    /// CORS rules.
    pub cors_rules: Option<Vec<CorsRuleConfig>>,
    /// Lifecycle configuration.
    pub lifecycle: Option<rustack_s3_model::types::BucketLifecycleConfiguration>,
    /// Bucket policy JSON.
    pub policy: Option<String>,
    /// Bucket tags.
    pub tags: Vec<(String, String)>,
    /// Canned ACL.
    pub acl: crate::state::object::CannedAcl,
    /// Notification configuration.
    pub notification_configuration: Option<rustack_s3_model::types::NotificationConfiguration>,
    /// Logging configuration.
    pub logging: Option<serde_json::Value>,
    /// Public access block settings.
    pub public_access_block: Option<PublicAccessBlockConfig>,
    /// Ownership controls.
    pub ownership_controls: Option<OwnershipControlsConfig>,
    /// Whether object lock is enabled.
    pub object_lock_enabled: bool,
    /// Object lock configuration.
    pub object_lock_configuration: Option<ObjectLockConfiguration>,
    /// Transfer acceleration status.
    pub accelerate: Option<String>,
    /// Request payment configuration.
    pub request_payment: String,
    /// Static website hosting configuration.
    pub website: Option<WebsiteConfig>,
    /// Replication configuration.
    pub replication: Option<serde_json::Value>,
    /// Analytics configuration.
    pub analytics: Option<serde_json::Value>,
    /// Metrics configuration.
    pub metrics: Option<serde_json::Value>,
    /// Inventory configuration.
    pub inventory: Option<serde_json::Value>,
    /// Intelligent-tiering configuration.
    pub intelligent_tiering: Option<serde_json::Value>,
}

/// Object version snapshot with external body file reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum S3ObjectVersionSnapshot {
    /// Real object version.
    Object {
        /// Object metadata.
        object: Box<S3Object>,
        /// Body file relative to the S3 data directory.
        body_file: String,
    },
    /// Delete marker version.
    DeleteMarker(S3DeleteMarker),
}

/// Multipart upload snapshot with external part body file references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3MultipartUploadSnapshot {
    /// Multipart upload metadata.
    pub upload: MultipartUpload,
    /// Part number to body file relative to the S3 data directory.
    pub part_body_files: HashMap<u32, String>,
}

impl RustackS3 {
    /// Export S3 state and object bodies into a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if object bodies cannot be read or written.
    pub async fn export_snapshot(&self, data_dir: &Path) -> Result<S3Snapshot, S3SnapshotError> {
        create_dir_all(data_dir).await?;
        let mut buckets = Vec::new();
        let mut body_index = 0usize;

        for bucket_name in self.state.snapshot_bucket_names() {
            let bucket = self.state.get_bucket(&bucket_name).map_err(|_| {
                S3SnapshotError::BucketDisappeared {
                    bucket: bucket_name.clone(),
                }
            })?;
            let (object_store_versioned, versions) = bucket.objects.read().snapshot_versions();
            let mut object_versions = Vec::with_capacity(versions.len());

            for version in versions {
                match version {
                    ObjectVersion::Object(object) => {
                        let body_file = format!("objects/{body_index}.bin");
                        body_index = body_index.saturating_add(1);
                        let data = self
                            .storage
                            .read_object(&bucket.name, &object.key, &object.version_id, None)
                            .await
                            .map_err(|source| S3SnapshotError::ReadObject {
                                bucket: bucket.name.clone(),
                                key: object.key.clone(),
                                version_id: object.version_id.clone(),
                                source: Box::new(source),
                            })?;
                        write_data_file(data_dir, &body_file, &data).await?;
                        object_versions.push(S3ObjectVersionSnapshot::Object { object, body_file });
                    }
                    ObjectVersion::DeleteMarker(marker) => {
                        object_versions.push(S3ObjectVersionSnapshot::DeleteMarker(marker));
                    }
                }
            }

            let mut multipart_uploads = Vec::new();
            for entry in &bucket.multipart_uploads {
                let upload = entry.value().clone();
                let mut part_body_files = HashMap::new();
                for part_number in upload.parts.keys() {
                    let body_file = format!("parts/{body_index}.bin");
                    body_index = body_index.saturating_add(1);
                    let data = self
                        .storage
                        .read_part(&bucket.name, &upload.upload_id, *part_number)
                        .await
                        .map_err(|source| S3SnapshotError::ReadPart {
                            bucket: bucket.name.clone(),
                            upload_id: upload.upload_id.clone(),
                            part_number: *part_number,
                            source: Box::new(source),
                        })?;
                    write_data_file(data_dir, &body_file, &data).await?;
                    part_body_files.insert(*part_number, body_file);
                }
                multipart_uploads.push(S3MultipartUploadSnapshot {
                    upload,
                    part_body_files,
                });
            }

            buckets.push(S3BucketSnapshot {
                name: bucket.name.clone(),
                region: bucket.region.clone(),
                creation_date: bucket.creation_date,
                owner: bucket.owner.clone(),
                object_store_versioned,
                object_versions,
                multipart_uploads,
                versioning: *bucket.versioning.read(),
                encryption: bucket.encryption.read().clone(),
                cors_rules: bucket.cors_rules.read().clone(),
                lifecycle: bucket.lifecycle.read().clone(),
                policy: bucket.policy.read().clone(),
                tags: bucket.tags.read().clone(),
                acl: *bucket.acl.read(),
                notification_configuration: bucket.notification_configuration.read().clone(),
                logging: bucket.logging.read().clone(),
                public_access_block: bucket.public_access_block.read().clone(),
                ownership_controls: bucket.ownership_controls.read().clone(),
                object_lock_enabled: *bucket.object_lock_enabled.read(),
                object_lock_configuration: bucket.object_lock_configuration.read().clone(),
                accelerate: bucket.accelerate.read().clone(),
                request_payment: bucket.request_payment.read().clone(),
                website: bucket.website.read().clone(),
                replication: bucket.replication.read().clone(),
                analytics: bucket.analytics.read().clone(),
                metrics: bucket.metrics.read().clone(),
                inventory: bucket.inventory.read().clone(),
                intelligent_tiering: bucket.intelligent_tiering.read().clone(),
            });
        }

        Ok(S3Snapshot { buckets })
    }

    /// Import S3 state and object bodies from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if object body files cannot be read or restored.
    pub async fn import_snapshot(
        &self,
        snapshot: S3Snapshot,
        data_dir: &Path,
    ) -> Result<(), S3SnapshotError> {
        self.reset();

        for bucket_snapshot in snapshot.buckets {
            let bucket = build_bucket_from_snapshot(&bucket_snapshot);
            let mut versions = Vec::with_capacity(bucket_snapshot.object_versions.len());

            for version in bucket_snapshot.object_versions {
                match version {
                    S3ObjectVersionSnapshot::Object { object, body_file } => {
                        let data = read_data_file(data_dir, &body_file).await?;
                        self.storage
                            .write_object(&bucket.name, &object.key, &object.version_id, data)
                            .await
                            .map_err(|source| S3SnapshotError::WriteObject {
                                bucket: bucket.name.clone(),
                                key: object.key.clone(),
                                version_id: object.version_id.clone(),
                                source: Box::new(source),
                            })?;
                        versions.push(ObjectVersion::Object(object));
                    }
                    S3ObjectVersionSnapshot::DeleteMarker(marker) => {
                        versions.push(ObjectVersion::DeleteMarker(marker));
                    }
                }
            }

            bucket
                .objects
                .write()
                .replace_from_snapshot(bucket_snapshot.object_store_versioned, versions);

            for multipart in bucket_snapshot.multipart_uploads {
                for (part_number, body_file) in &multipart.part_body_files {
                    let data = read_data_file(data_dir, body_file).await?;
                    self.storage
                        .write_part(
                            &bucket.name,
                            &multipart.upload.upload_id,
                            *part_number,
                            data,
                        )
                        .await
                        .map_err(|source| S3SnapshotError::WritePart {
                            bucket: bucket.name.clone(),
                            upload_id: multipart.upload.upload_id.clone(),
                            part_number: *part_number,
                            source: Box::new(source),
                        })?;
                }
                bucket
                    .multipart_uploads
                    .insert(multipart.upload.upload_id.clone(), multipart.upload);
            }

            self.state.insert_snapshot_bucket(bucket);
        }

        Ok(())
    }
}

fn build_bucket_from_snapshot(snapshot: &S3BucketSnapshot) -> S3Bucket {
    let bucket = S3Bucket::new(
        snapshot.name.clone(),
        snapshot.region.clone(),
        snapshot.owner.clone(),
    );
    let mut bucket = bucket;
    bucket.creation_date = snapshot.creation_date;
    *bucket.versioning.write() = snapshot.versioning;
    (*bucket.encryption.write()).clone_from(&snapshot.encryption);
    (*bucket.cors_rules.write()).clone_from(&snapshot.cors_rules);
    (*bucket.lifecycle.write()).clone_from(&snapshot.lifecycle);
    (*bucket.policy.write()).clone_from(&snapshot.policy);
    (*bucket.tags.write()).clone_from(&snapshot.tags);
    *bucket.acl.write() = snapshot.acl;
    (*bucket.notification_configuration.write()).clone_from(&snapshot.notification_configuration);
    (*bucket.logging.write()).clone_from(&snapshot.logging);
    (*bucket.public_access_block.write()).clone_from(&snapshot.public_access_block);
    (*bucket.ownership_controls.write()).clone_from(&snapshot.ownership_controls);
    *bucket.object_lock_enabled.write() = snapshot.object_lock_enabled;
    (*bucket.object_lock_configuration.write()).clone_from(&snapshot.object_lock_configuration);
    (*bucket.accelerate.write()).clone_from(&snapshot.accelerate);
    (*bucket.request_payment.write()).clone_from(&snapshot.request_payment);
    (*bucket.website.write()).clone_from(&snapshot.website);
    (*bucket.replication.write()).clone_from(&snapshot.replication);
    (*bucket.analytics.write()).clone_from(&snapshot.analytics);
    (*bucket.metrics.write()).clone_from(&snapshot.metrics);
    (*bucket.inventory.write()).clone_from(&snapshot.inventory);
    (*bucket.intelligent_tiering.write()).clone_from(&snapshot.intelligent_tiering);
    bucket
}

async fn create_dir_all(path: &Path) -> Result<(), S3SnapshotError> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| S3SnapshotError::Io {
            path: path.display().to_string(),
            source,
        })
}

async fn write_data_file(root: &Path, relative: &str, data: &[u8]) -> Result<(), S3SnapshotError> {
    let path = data_file_path(root, relative)?;
    if let Some(parent) = path.parent() {
        create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|source| S3SnapshotError::Io {
            path: path.display().to_string(),
            source,
        })?;
    file.write_all(data)
        .await
        .map_err(|source| S3SnapshotError::Io {
            path: path.display().to_string(),
            source,
        })
}

async fn read_data_file(root: &Path, relative: &str) -> Result<bytes::Bytes, S3SnapshotError> {
    let path = data_file_path(root, relative)?;
    tokio::fs::read(&path)
        .await
        .map(bytes::Bytes::from)
        .map_err(|source| S3SnapshotError::Io {
            path: path.display().to_string(),
            source,
        })
}

fn data_file_path(root: &Path, relative: &str) -> Result<PathBuf, S3SnapshotError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(S3SnapshotError::InvalidDataPath {
            path: relative.to_owned(),
        });
    }
    Ok(root.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_data_file_path_traversal() {
        let path = data_file_path(Path::new("/tmp/s3"), "../outside.bin");
        assert!(path.is_err());
    }

    #[test]
    fn test_should_accept_data_file_child_path() {
        let path = data_file_path(Path::new("/tmp/s3"), "objects/body.bin");
        assert!(path.is_ok());
    }
}
