//! Object key storage with versioning support.
//!
//! Provides [`ObjectStore`], an enum dispatching between [`KeyStore`]
//! (un-versioned) and [`VersionedKeyStore`] (versioned). Uses `BTreeMap`
//! internally so keys are always sorted, which is required for correct
//! `ListObjects` / `ListObjectVersions` pagination.

use std::collections::BTreeMap;

use chrono::Utc;
use tracing::debug;
use uuid::Uuid;

use super::object::{ObjectVersion, Owner, S3DeleteMarker, S3Object};

// ---------------------------------------------------------------------------
// List result types
// ---------------------------------------------------------------------------

/// Result of a `ListObjects` / `ListObjectsV2` operation.
#[derive(Debug, Clone)]
pub struct ListResult {
    /// The objects that match the listing criteria.
    pub objects: Vec<S3Object>,
    /// Common prefixes when a delimiter is used.
    pub common_prefixes: Vec<String>,
    /// Whether the result is truncated (more keys available).
    pub is_truncated: bool,
    /// The marker to use for the next page (last key returned).
    pub next_marker: Option<String>,
}

/// Result of a `ListObjectVersions` operation.
#[derive(Debug, Clone)]
pub struct VersionListResult {
    /// Object versions and delete markers.
    pub versions: Vec<VersionListEntry>,
    /// Common prefixes when a delimiter is used.
    pub common_prefixes: Vec<String>,
    /// Whether the result is truncated.
    pub is_truncated: bool,
    /// The key marker for the next page.
    pub next_key_marker: Option<String>,
    /// The version-id marker for the next page.
    pub next_version_id_marker: Option<String>,
}

/// A single entry in a version listing, augmented with `is_latest`.
#[derive(Debug, Clone)]
pub struct VersionListEntry {
    /// The underlying object version or delete marker.
    pub version: ObjectVersion,
    /// Whether this is the latest version for its key.
    pub is_latest: bool,
}

// ---------------------------------------------------------------------------
// ObjectStore (enum dispatch)
// ---------------------------------------------------------------------------

/// Top-level object store that dispatches to either an un-versioned or
/// versioned backing store.
#[derive(Debug)]
pub enum ObjectStore {
    /// Un-versioned storage. Each key maps to exactly one object.
    Unversioned(KeyStore),
    /// Versioned storage. Each key maps to an ordered list of versions.
    Versioned(VersionedKeyStore),
}

impl Default for ObjectStore {
    fn default() -> Self {
        Self::Unversioned(KeyStore::default())
    }
}

impl ObjectStore {
    /// Store an object. Returns the previous object for un-versioned stores.
    pub fn put(&mut self, object: S3Object) -> Option<S3Object> {
        match self {
            Self::Unversioned(ks) => ks.put(object),
            Self::Versioned(vs) => {
                vs.put(object);
                None
            }
        }
    }

    /// Get the current (latest non-delete-marker) object for a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&S3Object> {
        match self {
            Self::Unversioned(ks) => ks.get(key),
            Self::Versioned(vs) => vs.get(key),
        }
    }

    /// Get a specific version of an object by key and version ID.
    #[must_use]
    pub fn get_version(&self, key: &str, version_id: &str) -> Option<&S3Object> {
        match self {
            Self::Unversioned(ks) => {
                // In un-versioned stores, the only valid version_id is "null".
                if version_id == "null" {
                    ks.get(key)
                } else {
                    None
                }
            }
            Self::Versioned(vs) => vs.get_version(key, version_id),
        }
    }

    /// Get a mutable reference to the current (latest non-delete-marker) object.
    /// Used for in-place metadata updates (e.g., retention, legal hold).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut S3Object> {
        match self {
            Self::Unversioned(ks) => ks.get_mut(key),
            Self::Versioned(vs) => vs.get_mut(key),
        }
    }

    /// Get a mutable reference to a specific version by key and version ID.
    /// Used for in-place metadata updates (e.g., retention, legal hold).
    pub fn get_version_mut(&mut self, key: &str, version_id: &str) -> Option<&mut S3Object> {
        match self {
            Self::Unversioned(ks) => {
                if version_id == "null" {
                    ks.get_mut(key)
                } else {
                    None
                }
            }
            Self::Versioned(vs) => vs.get_version_mut(key, version_id),
        }
    }

    /// Check if a specific version ID for a key is a delete marker.
    #[must_use]
    pub fn is_delete_marker(&self, key: &str, version_id: &str) -> bool {
        match self {
            Self::Unversioned(_) => false,
            Self::Versioned(vs) => vs.is_delete_marker(key, version_id),
        }
    }

    /// Delete the object for a key (un-versioned semantics: removes the object).
    pub fn delete(&mut self, key: &str) -> Option<S3Object> {
        match self {
            Self::Unversioned(ks) => ks.delete(key),
            Self::Versioned(_) => None, // Use `delete_versioned` for versioned stores.
        }
    }

    /// Delete an object in a versioned bucket by inserting a delete marker.
    ///
    /// Returns `(Some(version_id), true)` if a delete marker was created and
    /// an existing object was logically hidden, or `(Some(version_id), false)`
    /// if a delete marker was created but no real object existed for that key.
    ///
    /// For un-versioned stores, removes the object directly and returns
    /// `(None, had_object)`.
    pub fn delete_versioned(&mut self, key: &str, owner: &Owner) -> (Option<String>, bool) {
        match self {
            Self::Unversioned(ks) => {
                let had = ks.delete(key).is_some();
                (None, had)
            }
            Self::Versioned(vs) => vs.delete(key, owner),
        }
    }

    /// Delete a specific version of an object.
    pub fn delete_version(&mut self, key: &str, version_id: &str) -> Option<ObjectVersion> {
        match self {
            Self::Unversioned(ks) => {
                if version_id == "null" {
                    ks.delete(key).map(|o| ObjectVersion::Object(Box::new(o)))
                } else {
                    None
                }
            }
            Self::Versioned(vs) => vs.delete_version(key, version_id),
        }
    }

    /// List objects matching a prefix, delimiter, start-after, and max-keys.
    #[must_use]
    pub fn list_objects(
        &self,
        prefix: &str,
        delimiter: &str,
        start_after: &str,
        max_keys: usize,
    ) -> ListResult {
        match self {
            Self::Unversioned(ks) => ks.list_objects(prefix, delimiter, start_after, max_keys),
            Self::Versioned(vs) => vs.list_objects(prefix, delimiter, start_after, max_keys),
        }
    }

    /// List object versions.
    #[must_use]
    pub fn list_object_versions(
        &self,
        prefix: &str,
        delimiter: &str,
        key_marker: &str,
        version_id_marker: &str,
        max_keys: usize,
    ) -> VersionListResult {
        match self {
            Self::Unversioned(ks) => {
                ks.list_object_versions(prefix, delimiter, key_marker, max_keys)
            }
            Self::Versioned(vs) => {
                vs.list_object_versions(prefix, delimiter, key_marker, version_id_marker, max_keys)
            }
        }
    }

    /// Count of non-deleted objects.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Unversioned(ks) => ks.len(),
            Self::Versioned(vs) => vs.len(),
        }
    }

    /// Whether the store contains zero entries (objects, versions, or delete markers).
    ///
    /// This checks for truly empty storage (no entries at all), which is the
    /// correct check for `DeleteBucket`. For listing purposes, use [`Self::len()`]
    /// which only counts non-deleted objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Unversioned(ks) => ks.is_empty(),
            Self::Versioned(vs) => vs.objects.is_empty(),
        }
    }

    /// Transition from un-versioned to versioned storage.
    ///
    /// If already versioned this is a no-op. Existing objects are migrated
    /// into single-element version lists.
    pub fn transition_to_versioned(&mut self) {
        if let Self::Unversioned(ks) = self {
            debug!("transitioning object store from unversioned to versioned");
            let mut vs = VersionedKeyStore::default();
            // Drain the BTreeMap while preserving sort order.
            for (key, obj) in std::mem::take(&mut ks.objects) {
                vs.objects
                    .insert(key, vec![ObjectVersion::Object(Box::new(obj))]);
            }
            *self = Self::Versioned(vs);
        }
    }

    /// Whether the store is in versioned mode.
    #[must_use]
    pub fn is_versioned(&self) -> bool {
        matches!(self, Self::Versioned(_))
    }
}

// ---------------------------------------------------------------------------
// KeyStore (un-versioned)
// ---------------------------------------------------------------------------

/// Un-versioned key store. Each key maps to exactly one `S3Object`.
#[derive(Debug, Default)]
pub struct KeyStore {
    /// Sorted map of object key to object.
    objects: BTreeMap<String, S3Object>,
}

impl KeyStore {
    /// Insert or replace an object. Returns the previous object if any.
    pub fn put(&mut self, object: S3Object) -> Option<S3Object> {
        self.objects.insert(object.key.clone(), object)
    }

    /// Get an object by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&S3Object> {
        self.objects.get(key)
    }

    /// Get a mutable reference to an object by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut S3Object> {
        self.objects.get_mut(key)
    }

    /// Remove an object by key. Returns the removed object if any.
    pub fn delete(&mut self, key: &str) -> Option<S3Object> {
        self.objects.remove(key)
    }

    /// Number of stored objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// List objects matching prefix, delimiter, start-after, and max-keys.
    #[must_use]
    pub fn list_objects(
        &self,
        prefix: &str,
        delimiter: &str,
        start_after: &str,
        max_keys: usize,
    ) -> ListResult {
        list_from_btree(
            self.objects.values(),
            prefix,
            delimiter,
            start_after,
            max_keys,
        )
    }

    /// List object versions (un-versioned: each object is version `"null"`, is_latest = true).
    #[must_use]
    fn list_object_versions(
        &self,
        prefix: &str,
        delimiter: &str,
        key_marker: &str,
        max_keys: usize,
    ) -> VersionListResult {
        let list = self.list_objects(prefix, delimiter, key_marker, max_keys);
        let versions = list
            .objects
            .into_iter()
            .map(|obj| VersionListEntry {
                version: ObjectVersion::Object(Box::new(obj)),
                is_latest: true,
            })
            .collect();
        VersionListResult {
            versions,
            common_prefixes: list.common_prefixes,
            is_truncated: list.is_truncated,
            next_key_marker: list.next_marker,
            next_version_id_marker: None,
        }
    }
}

// ---------------------------------------------------------------------------
// VersionedKeyStore
// ---------------------------------------------------------------------------

/// Versioned key store. Each key maps to an ordered list of versions
/// (newest first). The first entry is the "latest" version for any key.
#[derive(Debug, Default)]
pub struct VersionedKeyStore {
    /// Sorted map of object key to its version list (newest first).
    objects: BTreeMap<String, Vec<ObjectVersion>>,
}

impl VersionedKeyStore {
    /// Insert an object, generating a new version ID and prepending to the
    /// version list.
    pub fn put(&mut self, mut object: S3Object) {
        if object.version_id == "null" {
            object.version_id = generate_version_id();
        }
        debug!(key = %object.key, version = %object.version_id, "storing versioned object");
        let versions = self.objects.entry(object.key.clone()).or_default();
        versions.insert(0, ObjectVersion::Object(Box::new(object)));
    }

    /// Get the current object for a key.
    ///
    /// Returns `None` if the key doesn't exist or if the latest version is a
    /// delete marker (per S3 semantics, the object appears deleted).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&S3Object> {
        self.objects.get(key).and_then(|versions| {
            let latest = versions.first()?;
            // If the latest version is a delete marker, the object is logically deleted.
            latest.as_object()
        })
    }

    /// Get a specific version of an object.
    #[must_use]
    pub fn get_version(&self, key: &str, version_id: &str) -> Option<&S3Object> {
        self.objects.get(key).and_then(|versions| {
            versions
                .iter()
                .find(|v| v.version_id() == version_id)
                .and_then(|v| v.as_object())
        })
    }

    /// Get a mutable reference to the current (latest non-delete-marker) object.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut S3Object> {
        self.objects.get_mut(key).and_then(|versions| {
            let latest = versions.first_mut()?;
            latest.as_object_mut()
        })
    }

    /// Get a mutable reference to a specific version of an object.
    pub fn get_version_mut(&mut self, key: &str, version_id: &str) -> Option<&mut S3Object> {
        self.objects.get_mut(key).and_then(|versions| {
            versions
                .iter_mut()
                .find(|v| v.version_id() == version_id)
                .and_then(|v| v.as_object_mut())
        })
    }

    /// Check if a specific version ID for a key is a delete marker.
    #[must_use]
    pub fn is_delete_marker(&self, key: &str, version_id: &str) -> bool {
        self.objects
            .get(key)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|v| v.version_id() == version_id)
                    .map(ObjectVersion::is_delete_marker)
            })
            .unwrap_or(false)
    }

    /// Delete an object by inserting a delete marker at the front.
    ///
    /// Returns `(version_id_of_marker, had_real_object)`.
    pub fn delete(&mut self, key: &str, owner: &Owner) -> (Option<String>, bool) {
        let version_id = generate_version_id();
        let dm = S3DeleteMarker {
            key: key.to_owned(),
            version_id: version_id.clone(),
            last_modified: Utc::now(),
            owner: owner.clone(),
        };

        let versions = self.objects.entry(key.to_owned()).or_default();
        let had_object = versions.iter().any(|v| v.as_object().is_some());
        versions.insert(0, ObjectVersion::DeleteMarker(dm));
        debug!(key, version_id = %version_id, "inserted delete marker");

        (Some(version_id), had_object)
    }

    /// Remove a specific version (object or delete marker) entirely.
    pub fn delete_version(&mut self, key: &str, version_id: &str) -> Option<ObjectVersion> {
        let versions = self.objects.get_mut(key)?;
        let idx = versions.iter().position(|v| v.version_id() == version_id)?;
        let removed = versions.remove(idx);
        // Clean up empty version lists.
        if versions.is_empty() {
            self.objects.remove(key);
        }
        Some(removed)
    }

    /// Count of keys that have at least one non-delete-marker version as
    /// their latest entry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects
            .values()
            .filter(|versions| versions.first().is_some_and(|v| !v.is_delete_marker()))
            .count()
    }

    /// Whether the store contains zero entries (no versions or delete markers).
    ///
    /// This checks if the underlying BTreeMap is completely empty, which is
    /// the correct semantics for `DeleteBucket` (AWS requires all versions
    /// and delete markers to be removed before bucket deletion).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// List the latest non-delete-marker object for each key.
    #[must_use]
    pub fn list_objects(
        &self,
        prefix: &str,
        delimiter: &str,
        start_after: &str,
        max_keys: usize,
    ) -> ListResult {
        // Build an iterator over the "current" (latest non-DM) object per key.
        let current_objects = self.objects.values().filter_map(|versions| {
            // Only consider keys whose latest entry is NOT a delete marker.
            let latest = versions.first()?;
            if latest.is_delete_marker() {
                return None;
            }
            latest.as_object()
        });

        list_from_btree(current_objects, prefix, delimiter, start_after, max_keys)
    }

    /// List all versions (objects and delete markers).
    #[must_use]
    pub fn list_object_versions(
        &self,
        prefix: &str,
        delimiter: &str,
        key_marker: &str,
        version_id_marker: &str,
        max_keys: usize,
    ) -> VersionListResult {
        let use_delim = !delimiter.is_empty();
        let mut result_versions: Vec<VersionListEntry> = Vec::new();
        let mut common_prefixes: Vec<String> = Vec::new();
        let mut seen_prefixes = std::collections::HashSet::new();
        let mut count = 0usize;
        let mut is_truncated = false;
        let mut last_key: Option<String> = None;
        let mut last_version_id: Option<String> = None;

        // Determine where to start iteration.
        let iter: Box<dyn Iterator<Item = (&String, &Vec<ObjectVersion>)>> =
            if key_marker.is_empty() {
                Box::new(self.objects.iter())
            } else {
                // Start from the key_marker (exclusive unless version_id_marker applies).
                let marker = key_marker.to_owned();
                Box::new(self.objects.range(marker..))
            };

        'outer: for (key, versions) in iter {
            // Skip keys before the marker.
            if !key_marker.is_empty() && key.as_str() < key_marker {
                continue;
            }

            // Filter by prefix.
            if !prefix.is_empty() && !key.starts_with(prefix) {
                // BTreeMap is sorted, so if the key is past the prefix range, stop.
                if key.as_str() > prefix {
                    // Check if we are still in prefix-adjacent territory.
                    // Once key is beyond the prefix lexicographically and doesn't
                    // start with prefix, there can be no more matches.
                    let beyond = !key.starts_with(&prefix[..prefix.len().saturating_sub(1).max(1)]);
                    if beyond {
                        break;
                    }
                }
                continue;
            }

            // Delimiter-based common prefix grouping.
            if use_delim {
                let after_prefix = &key[prefix.len()..];
                if let Some(pos) = after_prefix.find(delimiter) {
                    let cp = format!("{}{}{}", prefix, &after_prefix[..pos], delimiter);
                    if seen_prefixes.insert(cp.clone()) {
                        common_prefixes.push(cp);
                    }
                    continue;
                }
            }

            // For the key_marker key, skip versions until we pass version_id_marker.
            let mut skip_versions = key.as_str() == key_marker && !version_id_marker.is_empty();

            for (idx, version) in versions.iter().enumerate() {
                if skip_versions {
                    if version.version_id() == version_id_marker {
                        skip_versions = false;
                    }
                    continue;
                }

                if count >= max_keys {
                    is_truncated = true;
                    break 'outer;
                }

                let entry = VersionListEntry {
                    version: version.clone(),
                    is_latest: idx == 0,
                };
                last_key = Some(key.clone());
                last_version_id = Some(version.version_id().to_owned());
                result_versions.push(entry);
                count += 1;
            }
        }

        VersionListResult {
            versions: result_versions,
            common_prefixes,
            is_truncated,
            next_key_marker: if is_truncated { last_key } else { None },
            next_version_id_marker: if is_truncated { last_version_id } else { None },
        }
    }
}

// ---------------------------------------------------------------------------
// Shared listing helper
// ---------------------------------------------------------------------------

/// Build a [`ListResult`] from an iterator of `S3Object` references, applying
/// prefix, delimiter, start-after, and max-keys filtering.
fn list_from_btree<'a>(
    objects: impl Iterator<Item = &'a S3Object>,
    prefix: &str,
    delimiter: &str,
    start_after: &str,
    max_keys: usize,
) -> ListResult {
    let use_delim = !delimiter.is_empty();
    let mut result_objects: Vec<S3Object> = Vec::new();
    let mut common_prefixes: Vec<String> = Vec::new();
    let mut seen_prefixes = std::collections::HashSet::new();
    let mut count = 0usize;
    let mut is_truncated = false;

    for obj in objects {
        // Skip keys at or before start_after.
        if !start_after.is_empty() && obj.key.as_str() <= start_after {
            continue;
        }

        // Filter by prefix.
        if !prefix.is_empty() && !obj.key.starts_with(prefix) {
            continue;
        }

        // Delimiter-based grouping.
        if use_delim {
            let after_prefix = &obj.key[prefix.len()..];
            if let Some(pos) = after_prefix.find(delimiter) {
                let cp = format!("{}{}{}", prefix, &after_prefix[..pos], delimiter);
                if seen_prefixes.insert(cp.clone()) {
                    common_prefixes.push(cp);
                }
                continue;
            }
        }

        if count >= max_keys {
            is_truncated = true;
            break;
        }

        result_objects.push(obj.clone());
        count += 1;
    }

    let next_marker = if is_truncated {
        result_objects.last().map(|o| o.key.clone())
    } else {
        None
    };

    ListResult {
        objects: result_objects,
        common_prefixes,
        is_truncated,
        next_marker,
    }
}

/// Generate a unique version ID for versioned objects / delete markers.
fn generate_version_id() -> String {
    Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::object::ObjectMetadata;

    // ---- helpers ----

    fn make_object(key: &str) -> S3Object {
        S3Object {
            key: key.to_owned(),
            version_id: "null".to_owned(),
            etag: format!("\"etag-{key}\""),
            size: 100,
            last_modified: Utc::now(),
            storage_class: "STANDARD".to_owned(),
            metadata: ObjectMetadata::default(),
            owner: Owner::default(),
            checksum: None,
            parts_count: None,
            part_etags: Vec::new(),
        }
    }

    // ---- KeyStore tests ----

    #[test]
    fn test_should_put_and_get_in_keystore() {
        let mut ks = KeyStore::default();
        assert!(ks.is_empty());

        ks.put(make_object("a/b/c"));
        assert_eq!(ks.len(), 1);

        let obj = ks.get("a/b/c");
        assert!(obj.is_some());
        assert_eq!(obj.map(|o| o.key.as_str()), Some("a/b/c"));
    }

    #[test]
    fn test_should_replace_object_in_keystore() {
        let mut ks = KeyStore::default();
        let prev = ks.put(make_object("key1"));
        assert!(prev.is_none());

        let mut replacement = make_object("key1");
        replacement.size = 999;
        let prev = ks.put(replacement);
        assert!(prev.is_some());
        assert_eq!(prev.map(|o| o.size), Some(100));
        assert_eq!(ks.get("key1").map(|o| o.size), Some(999));
    }

    #[test]
    fn test_should_delete_from_keystore() {
        let mut ks = KeyStore::default();
        ks.put(make_object("key1"));
        assert_eq!(ks.len(), 1);

        let removed = ks.delete("key1");
        assert!(removed.is_some());
        assert!(ks.is_empty());
        assert!(ks.delete("key1").is_none());
    }

    #[test]
    fn test_should_list_objects_in_keystore() {
        let mut ks = KeyStore::default();
        for key in ["a", "b", "c", "d", "e"] {
            ks.put(make_object(key));
        }

        let result = ks.list_objects("", "", "", 3);
        assert_eq!(result.objects.len(), 3);
        assert!(result.is_truncated);
        assert_eq!(result.next_marker, Some("c".to_owned()));

        let result = ks.list_objects("", "", "c", 10);
        assert_eq!(result.objects.len(), 2);
        assert!(!result.is_truncated);
    }

    #[test]
    fn test_should_list_with_prefix_and_delimiter() {
        let mut ks = KeyStore::default();
        for key in [
            "photos/2023/jan.jpg",
            "photos/2023/feb.jpg",
            "photos/2024/mar.jpg",
            "docs/readme.txt",
        ] {
            ks.put(make_object(key));
        }

        // List with prefix and delimiter.
        let result = ks.list_objects("photos/", "/", "", 100);
        assert!(result.objects.is_empty());
        assert_eq!(result.common_prefixes.len(), 2);
        assert!(result.common_prefixes.contains(&"photos/2023/".to_owned()));
        assert!(result.common_prefixes.contains(&"photos/2024/".to_owned()));

        // List specific "folder".
        let result = ks.list_objects("photos/2023/", "/", "", 100);
        assert_eq!(result.objects.len(), 2);
        assert!(result.common_prefixes.is_empty());
    }

    // ---- VersionedKeyStore tests ----

    #[test]
    fn test_should_put_and_get_in_versioned_store() {
        let mut vs = VersionedKeyStore::default();
        vs.put(make_object("key1"));

        let obj = vs.get("key1");
        assert!(obj.is_some());
        assert_ne!(obj.map(|o| o.version_id.as_str()), Some("null"));
    }

    #[test]
    fn test_should_stack_versions_newest_first() {
        let mut vs = VersionedKeyStore::default();

        let mut obj1 = make_object("key1");
        obj1.size = 100;
        vs.put(obj1);

        let mut obj2 = make_object("key1");
        obj2.size = 200;
        vs.put(obj2);

        // Latest should be the second one (size=200).
        assert_eq!(vs.get("key1").map(|o| o.size), Some(200));

        // Should have two versions.
        let versions = vs.objects.get("key1");
        assert!(versions.is_some());
        assert_eq!(versions.map(Vec::len), Some(2));
    }

    #[test]
    fn test_should_insert_delete_marker() {
        let mut vs = VersionedKeyStore::default();
        vs.put(make_object("key1"));

        let (dm_version, had_object) = vs.delete("key1", &Owner::default());
        assert!(dm_version.is_some());
        assert!(had_object);

        // Per S3 semantics, when the latest version is a delete marker,
        // get() should return None (object appears deleted).
        let obj = vs.get("key1");
        assert!(obj.is_none());

        // len() counts keys whose latest is not a DM, so this key is "deleted".
        assert_eq!(vs.len(), 0);

        // But is_empty() is false because the BTreeMap still has entries
        // (the original object version + delete marker). This is correct for
        // DeleteBucket: a bucket with delete markers is NOT truly empty.
        assert!(!vs.is_empty());
    }

    #[test]
    fn test_should_delete_specific_version() {
        let mut vs = VersionedKeyStore::default();
        vs.put(make_object("key1"));
        let version_id = vs.get("key1").map(|o| o.version_id.clone());
        assert!(version_id.is_some());

        let version_id = version_id.unwrap_or_default();
        let removed = vs.delete_version("key1", &version_id);
        assert!(removed.is_some());
        assert!(!vs.objects.contains_key("key1"));
    }

    #[test]
    fn test_should_get_version_by_id() {
        let mut vs = VersionedKeyStore::default();
        let mut obj1 = make_object("key1");
        obj1.size = 111;
        vs.put(obj1);
        let v1_id = vs
            .objects
            .get("key1")
            .and_then(|v| v.first())
            .map(|v| v.version_id().to_owned())
            .unwrap_or_default();

        let mut obj2 = make_object("key1");
        obj2.size = 222;
        vs.put(obj2);

        // Retrieve the older version specifically.
        let old = vs.get_version("key1", &v1_id);
        assert!(old.is_some());
        assert_eq!(old.map(|o| o.size), Some(111));
    }

    #[test]
    fn test_should_list_versioned_objects() {
        let mut vs = VersionedKeyStore::default();
        vs.put(make_object("a"));
        vs.put(make_object("b"));
        vs.put(make_object("c"));

        let result = vs.list_objects("", "", "", 10);
        assert_eq!(result.objects.len(), 3);
        assert!(!result.is_truncated);
    }

    #[test]
    fn test_should_list_object_versions() {
        let mut vs = VersionedKeyStore::default();
        vs.put(make_object("key1"));
        vs.put(make_object("key1")); // second version
        vs.put(make_object("key2"));

        let result = vs.list_object_versions("", "", "", "", 100);
        // key1 has 2 versions, key2 has 1.
        assert_eq!(result.versions.len(), 3);
        assert!(!result.is_truncated);

        // First version of key1 should be is_latest=true.
        let first_key1 = result
            .versions
            .iter()
            .find(|e| e.version.key() == "key1" && e.is_latest);
        assert!(first_key1.is_some());
    }

    // ---- ObjectStore tests ----

    #[test]
    fn test_should_default_to_unversioned() {
        let store = ObjectStore::default();
        assert!(!store.is_versioned());
        assert!(store.is_empty());
    }

    #[test]
    fn test_should_transition_to_versioned() {
        let mut store = ObjectStore::default();
        store.put(make_object("existing"));
        assert!(!store.is_versioned());
        assert_eq!(store.len(), 1);

        store.transition_to_versioned();
        assert!(store.is_versioned());
        assert_eq!(store.len(), 1);

        // Existing object should still be retrievable.
        assert!(store.get("existing").is_some());
    }

    #[test]
    fn test_should_return_previous_on_unversioned_put() {
        let mut store = ObjectStore::default();
        let prev = store.put(make_object("k"));
        assert!(prev.is_none());

        let prev = store.put(make_object("k"));
        assert!(prev.is_some());
    }

    #[test]
    fn test_should_not_return_previous_on_versioned_put() {
        let mut store = ObjectStore::Versioned(VersionedKeyStore::default());
        let prev = store.put(make_object("k"));
        assert!(prev.is_none());

        let prev = store.put(make_object("k"));
        assert!(prev.is_none());
    }

    #[test]
    fn test_should_delete_versioned_via_object_store() {
        let mut store = ObjectStore::Versioned(VersionedKeyStore::default());
        store.put(make_object("k"));

        let (dm_id, had) = store.delete_versioned("k", &Owner::default());
        assert!(dm_id.is_some());
        assert!(had);
        // After delete marker, len() should be 0 (key is logically deleted).
        assert_eq!(store.len(), 0);
        // But is_empty() is false (entries still exist: the object + delete marker).
        assert!(!store.is_empty());
    }

    #[test]
    fn test_should_get_version_in_unversioned_store() {
        let mut store = ObjectStore::default();
        store.put(make_object("k"));

        assert!(store.get_version("k", "null").is_some());
        assert!(store.get_version("k", "other-version").is_none());
    }

    #[test]
    fn test_should_delete_version_in_unversioned_store() {
        let mut store = ObjectStore::default();
        store.put(make_object("k"));

        let removed = store.delete_version("k", "null");
        assert!(removed.is_some());
        assert!(store.is_empty());

        // Deleting non-null version should return None.
        store.put(make_object("k2"));
        assert!(store.delete_version("k2", "v123").is_none());
    }

    #[test]
    fn test_should_list_with_pagination() {
        let mut store = ObjectStore::default();
        for i in 0..10 {
            store.put(make_object(&format!("key-{i:02}")));
        }

        let page1 = store.list_objects("", "", "", 3);
        assert_eq!(page1.objects.len(), 3);
        assert!(page1.is_truncated);
        let marker = page1.next_marker.as_deref().unwrap_or("");

        let page2 = store.list_objects("", "", marker, 3);
        assert_eq!(page2.objects.len(), 3);
        assert!(page2.is_truncated);
    }

    #[test]
    fn test_should_transition_preserve_all_objects() {
        let mut store = ObjectStore::default();
        for key in ["alpha", "beta", "gamma"] {
            store.put(make_object(key));
        }
        assert_eq!(store.len(), 3);

        store.transition_to_versioned();
        assert!(store.is_versioned());
        assert_eq!(store.len(), 3);

        for key in ["alpha", "beta", "gamma"] {
            assert!(
                store.get(key).is_some(),
                "missing key after transition: {key}"
            );
        }
    }

    #[test]
    fn test_should_handle_delete_marker_on_nonexistent_key() {
        let mut vs = VersionedKeyStore::default();
        let (dm_version, had_object) = vs.delete("nonexistent", &Owner::default());
        assert!(dm_version.is_some());
        assert!(!had_object);
    }
}
