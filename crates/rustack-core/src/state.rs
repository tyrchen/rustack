//! Multi-account, multi-region state management.
//!
//! Provides [`AccountRegionStore`], a thread-safe concurrent store that
//! partitions state by AWS account ID and region, matching `LocalStack`'s
//! `AccountRegionBundle` pattern.

use std::sync::Arc;

use dashmap::DashMap;

use crate::types::{AccountId, AwsRegion};

/// Thread-safe, multi-account, multi-region state store.
///
/// Each (account, region) pair gets its own isolated state instance of type `T`.
/// Uses `DashMap` for lock-free concurrent access.
///
/// # Examples
///
/// ```
/// use rustack_core::{AccountRegionStore, AccountId, AwsRegion};
///
/// #[derive(Debug, Default)]
/// struct MyServiceState {
///     counter: std::sync::atomic::AtomicU64,
/// }
///
/// let store = AccountRegionStore::<MyServiceState>::new();
/// let state = store.get_or_create(&AccountId::default(), &AwsRegion::default());
/// state.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
/// ```
#[derive(Debug)]
pub struct AccountRegionStore<T: Default + Send + Sync> {
    inner: DashMap<(AccountId, AwsRegion), Arc<T>>,
}

impl<T: Default + Send + Sync> AccountRegionStore<T> {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Get or create the state for the given account and region.
    ///
    /// If the state does not exist, a new default instance is created atomically.
    #[must_use]
    pub fn get_or_create(&self, account: &AccountId, region: &AwsRegion) -> Arc<T> {
        self.inner
            .entry((account.clone(), region.clone()))
            .or_insert_with(|| Arc::new(T::default()))
            .clone()
    }

    /// Get the state for the given account and region, if it exists.
    #[must_use]
    pub fn get(&self, account: &AccountId, region: &AwsRegion) -> Option<Arc<T>> {
        self.inner
            .get(&(account.clone(), region.clone()))
            .map(|v| v.clone())
    }

    /// Remove the state for the given account and region.
    #[must_use]
    pub fn remove(&self, account: &AccountId, region: &AwsRegion) -> Option<Arc<T>> {
        self.inner
            .remove(&(account.clone(), region.clone()))
            .map(|(_, v)| v)
    }

    /// Reset all state in the store.
    pub fn reset(&self) {
        self.inner.clear();
    }

    /// Number of (account, region) entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<T: Default + Send + Sync> Default for AccountRegionStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct TestState {
        value: std::sync::atomic::AtomicU64,
    }

    #[test]
    fn test_should_create_state_on_first_access() {
        let store = AccountRegionStore::<TestState>::new();
        let account = AccountId::default();
        let region = AwsRegion::default();

        assert!(store.is_empty());
        let state = store.get_or_create(&account, &region);
        assert_eq!(store.len(), 1);
        assert_eq!(state.value.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn test_should_return_same_state_on_subsequent_access() {
        let store = AccountRegionStore::<TestState>::new();
        let account = AccountId::default();
        let region = AwsRegion::default();

        let state1 = store.get_or_create(&account, &region);
        state1.value.store(42, std::sync::atomic::Ordering::Relaxed);

        let state2 = store.get_or_create(&account, &region);
        assert_eq!(state2.value.load(std::sync::atomic::Ordering::Relaxed), 42);
    }

    #[test]
    fn test_should_isolate_different_regions() {
        let store = AccountRegionStore::<TestState>::new();
        let account = AccountId::default();
        let us_east = AwsRegion::new("us-east-1");
        let eu_west = AwsRegion::new("eu-west-1");

        let state_us = store.get_or_create(&account, &us_east);
        state_us
            .value
            .store(1, std::sync::atomic::Ordering::Relaxed);

        let state_eu = store.get_or_create(&account, &eu_west);
        assert_eq!(state_eu.value.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn test_should_reset_all_state() {
        let store = AccountRegionStore::<TestState>::new();
        let _ = store.get_or_create(&AccountId::default(), &AwsRegion::default());
        let _ = store.get_or_create(&AccountId::default(), &AwsRegion::new("eu-west-1"));

        assert_eq!(store.len(), 2);
        store.reset();
        assert!(store.is_empty());
    }
}
