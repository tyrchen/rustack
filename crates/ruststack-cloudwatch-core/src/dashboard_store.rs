//! Dashboard JSON body storage.

use dashmap::DashMap;

/// A stored dashboard.
#[derive(Debug, Clone)]
pub struct DashboardRecord {
    /// Dashboard name.
    pub dashboard_name: String,
    /// Dashboard ARN.
    pub dashboard_arn: String,
    /// Dashboard body (JSON string).
    pub dashboard_body: String,
    /// Last modified timestamp (epoch seconds as f64).
    pub last_modified: f64,
    /// Size in bytes.
    pub size: i64,
}

/// Dashboard store holding dashboard JSON bodies.
#[derive(Debug, Default)]
pub struct DashboardStore {
    dashboards: DashMap<String, DashboardRecord>,
}

impl DashboardStore {
    /// Create a new dashboard store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dashboards: DashMap::new(),
        }
    }

    /// Store a dashboard (insert or update).
    pub fn put(&self, record: DashboardRecord) {
        self.dashboards
            .insert(record.dashboard_name.clone(), record);
    }

    /// Get a dashboard by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<DashboardRecord> {
        self.dashboards.get(name).map(|r| r.value().clone())
    }

    /// Delete dashboards by name.
    #[must_use]
    pub fn delete(&self, names: &[String]) -> Vec<String> {
        let mut deleted = Vec::new();
        for name in names {
            if self.dashboards.remove(name).is_some() {
                deleted.push(name.clone());
            }
        }
        deleted
    }

    /// List all dashboards, optionally filtered by name prefix.
    #[must_use]
    pub fn list(&self, prefix: Option<&str>) -> Vec<DashboardRecord> {
        self.dashboards
            .iter()
            .filter(|entry| prefix.is_none_or(|p| entry.key().starts_with(p)))
            .map(|entry| entry.value().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dashboard(name: &str) -> DashboardRecord {
        DashboardRecord {
            dashboard_name: name.to_owned(),
            dashboard_arn: format!("arn:aws:cloudwatch::000000000000:dashboard/{name}"),
            dashboard_body: r#"{"widgets":[]}"#.to_owned(),
            last_modified: 1_234_567_890.0,
            size: 15,
        }
    }

    #[test]
    fn test_should_store_and_retrieve_dashboard() {
        let store = DashboardStore::new();
        store.put(make_dashboard("test"));
        let d = store.get("test").unwrap();
        assert_eq!(d.dashboard_name, "test");
    }

    #[test]
    fn test_should_delete_dashboards() {
        let store = DashboardStore::new();
        store.put(make_dashboard("a"));
        store.put(make_dashboard("b"));
        let _ = store.delete(&["a".to_owned()]);
        assert!(store.get("a").is_none());
        assert!(store.get("b").is_some());
    }

    #[test]
    fn test_should_list_with_prefix() {
        let store = DashboardStore::new();
        store.put(make_dashboard("prod-api"));
        store.put(make_dashboard("prod-web"));
        store.put(make_dashboard("dev-api"));
        let prod = store.list(Some("prod-"));
        assert_eq!(prod.len(), 2);
    }
}
