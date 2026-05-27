//! In-memory CloudFront response cache with snapshot import/export support.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use dashmap::DashMap;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;

use crate::transform::HOP_BY_HOP;

const CACHE_BODY_DIR: &str = "bodies";

/// Snapshot errors raised by the CloudFront response cache.
#[derive(Debug, Error)]
pub enum CacheSnapshotError {
    /// Cache data path escaped the snapshot data directory.
    #[error("invalid CloudFront cache snapshot data path: {path}")]
    InvalidDataPath {
        /// Invalid relative path.
        path: String,
    },

    /// Cache body I/O failed.
    #[error("CloudFront cache snapshot I/O failed at {path}: {source}")]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Cached HTTP status code was invalid.
    #[error("invalid cached CloudFront status code: {status}")]
    InvalidStatus {
        /// Invalid HTTP status code.
        status: u16,
    },
}

/// Serializable CloudFront cache snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudFrontCacheSnapshot {
    /// Cached response entries.
    pub entries: Vec<CloudFrontCacheEntrySnapshot>,
}

/// Serializable CloudFront cache entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudFrontCacheEntrySnapshot {
    /// Cache key.
    pub key: String,
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Snapshot-relative body file.
    pub body_file: String,
    /// Expiration timestamp in Unix seconds.
    pub expires_at_epoch_secs: u64,
}

#[derive(Debug, Clone)]
struct CachedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    expires_at_epoch_secs: u64,
}

/// Shared response cache used by the CloudFront data plane.
#[derive(Debug, Default)]
pub struct ResponseCache {
    entries: DashMap<String, CachedResponse>,
}

impl ResponseCache {
    /// Build an empty response cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the number of live entries, dropping expired entries observed during the count.
    #[must_use]
    pub fn len(&self) -> usize {
        let now = now_epoch_secs();
        self.entries
            .retain(|_, entry| entry.expires_at_epoch_secs > now);
        self.entries.len()
    }

    /// Return whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up a response by cache key.
    pub fn get(&self, key: &str) -> Option<Response<Bytes>> {
        let now = now_epoch_secs();
        let entry = self.entries.get(key)?;
        if entry.expires_at_epoch_secs <= now {
            drop(entry);
            self.entries.remove(key);
            return None;
        }
        build_response(&entry)
    }

    /// Insert a cacheable response.
    pub fn insert(&self, key: String, response: &Response<Bytes>, ttl: Duration) {
        if ttl.is_zero() || response.status() != StatusCode::OK {
            return;
        }
        let Some(expires_at_epoch_secs) = now_epoch_secs().checked_add(ttl.as_secs()) else {
            return;
        };
        let headers = snapshot_headers(response.headers());
        let entry = CachedResponse {
            status: response.status().as_u16(),
            headers,
            body: response.body().clone(),
            expires_at_epoch_secs,
        };
        self.entries.insert(key, entry);
    }

    /// Export cache metadata and response bodies into a snapshot staging directory.
    ///
    /// # Errors
    ///
    /// Returns an error when a response body cannot be written.
    pub async fn export_snapshot(
        &self,
        data_dir: &Path,
    ) -> Result<CloudFrontCacheSnapshot, CacheSnapshotError> {
        let body_dir = data_dir.join(CACHE_BODY_DIR);
        fs::create_dir_all(&body_dir)
            .await
            .map_err(|source| CacheSnapshotError::Io {
                path: body_dir.clone(),
                source,
            })?;

        let now = now_epoch_secs();
        let mut entries: Vec<(String, CachedResponse)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let value = entry.value();
                (value.expires_at_epoch_secs > now).then(|| (entry.key().clone(), value.clone()))
            })
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));

        let mut snapshot_entries = Vec::with_capacity(entries.len());
        for (idx, (key, entry)) in entries.into_iter().enumerate() {
            let body_file = format!("{CACHE_BODY_DIR}/{idx:08}.bin");
            let path = data_file_path(data_dir, &body_file)?;
            fs::write(&path, &entry.body)
                .await
                .map_err(|source| CacheSnapshotError::Io { path, source })?;
            snapshot_entries.push(CloudFrontCacheEntrySnapshot {
                key,
                status: entry.status,
                headers: entry.headers,
                body_file,
                expires_at_epoch_secs: entry.expires_at_epoch_secs,
            });
        }

        Ok(CloudFrontCacheSnapshot {
            entries: snapshot_entries,
        })
    }

    /// Replace the cache with entries from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when a cached response body cannot be read or a status code is invalid.
    pub async fn import_snapshot(
        &self,
        snapshot: CloudFrontCacheSnapshot,
        data_dir: &Path,
    ) -> Result<(), CacheSnapshotError> {
        let now = now_epoch_secs();
        let mut seen = HashSet::with_capacity(snapshot.entries.len());
        let mut imported = Vec::with_capacity(snapshot.entries.len());
        for entry in snapshot.entries {
            if entry.expires_at_epoch_secs <= now || !seen.insert(entry.key.clone()) {
                continue;
            }
            if StatusCode::from_u16(entry.status).is_err() {
                return Err(CacheSnapshotError::InvalidStatus {
                    status: entry.status,
                });
            }
            let path = data_file_path(data_dir, &entry.body_file)?;
            let body = fs::read(&path)
                .await
                .map_err(|source| CacheSnapshotError::Io { path, source })?;
            imported.push((
                entry.key,
                CachedResponse {
                    status: entry.status,
                    headers: entry.headers,
                    body: Bytes::from(body),
                    expires_at_epoch_secs: entry.expires_at_epoch_secs,
                },
            ));
        }

        self.entries.clear();
        for (key, entry) in imported {
            self.entries.insert(key, entry);
        }
        Ok(())
    }
}

/// Return whether the HTTP method may be cached by a behavior.
#[must_use]
pub fn method_is_cacheable(method: &Method, cached_methods: &[String]) -> bool {
    if cached_methods.is_empty() {
        return matches!(*method, Method::GET | Method::HEAD);
    }
    cached_methods
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(method.as_str()))
}

/// Build a deterministic cache key.
#[must_use]
pub fn cache_key(
    distribution_id: &str,
    method: &Method,
    effective_path: &str,
    query: Option<&str>,
) -> String {
    match query {
        Some(query) if !query.is_empty() => {
            format!(
                "{}:{}:{}?{query}",
                distribution_id,
                method.as_str(),
                effective_path
            )
        }
        _ => format!("{}:{}:{effective_path}", distribution_id, method.as_str()),
    }
}

fn build_response(entry: &CachedResponse) -> Option<Response<Bytes>> {
    let status = StatusCode::from_u16(entry.status).ok()?;
    let mut builder = Response::builder().status(status);
    for (name, value) in &entry.headers {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            builder = builder.header(header_name, header_value);
        }
    }
    builder.body(entry.body.clone()).ok()
}

fn snapshot_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&lower.as_str())
            || lower == "x-amz-cf-id"
            || lower == "x-cache"
            || lower == "via"
        {
            continue;
        }
        if let Ok(value) = value.to_str() {
            out.push((lower, value.to_owned()));
        }
    }
    out
}

fn data_file_path(root: &Path, relative: &str) -> Result<PathBuf, CacheSnapshotError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(CacheSnapshotError::InvalidDataPath {
            path: relative.to_owned(),
        });
    }
    if relative_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(CacheSnapshotError::InvalidDataPath {
            path: relative.to_owned(),
        });
    }
    Ok(root.join(relative_path))
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_should_build_cache_key_with_query() {
        let key = cache_key("E123", &Method::GET, "/index.html", Some("v=1"));
        assert_eq!(key, "E123:GET:/index.html?v=1");
    }

    #[test]
    fn test_should_default_get_and_head_as_cacheable() {
        assert!(method_is_cacheable(&Method::GET, &[]));
        assert!(method_is_cacheable(&Method::HEAD, &[]));
        assert!(!method_is_cacheable(&Method::POST, &[]));
    }

    #[tokio::test]
    async fn test_should_export_and_import_cache_snapshot() {
        let cache = ResponseCache::new();
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Bytes::from_static(b"cached"))
            .unwrap();
        cache.insert("E123:GET:/x".to_owned(), &response, Duration::from_mins(1));

        let dir = tempdir().unwrap();
        let snapshot = cache.export_snapshot(dir.path()).await.unwrap();
        assert_eq!(snapshot.entries.len(), 1);

        let restored = ResponseCache::new();
        restored
            .import_snapshot(snapshot, dir.path())
            .await
            .unwrap();
        let hit = restored.get("E123:GET:/x").unwrap();
        assert_eq!(hit.status(), StatusCode::OK);
        assert_eq!(hit.body(), &Bytes::from_static(b"cached"));
    }
}
