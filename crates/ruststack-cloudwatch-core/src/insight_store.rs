//! Insight rule metadata storage (no actual rule evaluation).

use dashmap::DashMap;
use ruststack_cloudwatch_model::types::InsightRule;

/// Insight rule store (metadata only).
#[derive(Debug, Default)]
pub struct InsightStore {
    rules: DashMap<String, InsightRule>,
}

impl InsightStore {
    /// Create a new insight store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: DashMap::new(),
        }
    }

    /// Store an insight rule.
    pub fn put(&self, rule: InsightRule) {
        self.rules.insert(rule.name.clone(), rule);
    }

    /// Delete insight rules by name.
    pub fn delete(&self, names: &[String]) {
        for name in names {
            self.rules.remove(name);
        }
    }

    /// List all insight rules.
    #[must_use]
    pub fn list(&self) -> Vec<InsightRule> {
        self.rules.iter().map(|e| e.value().clone()).collect()
    }
}
