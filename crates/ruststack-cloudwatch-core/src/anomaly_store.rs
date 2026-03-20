//! Anomaly detector metadata storage (no model training).

use dashmap::DashMap;
use ruststack_cloudwatch_model::types::AnomalyDetector;

/// Anomaly detector store (metadata only).
#[derive(Debug, Default)]
pub struct AnomalyStore {
    detectors: DashMap<String, AnomalyDetector>,
}

impl AnomalyStore {
    /// Create a new anomaly store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            detectors: DashMap::new(),
        }
    }

    /// Generate a key for an anomaly detector.
    fn make_key(detector: &AnomalyDetector) -> String {
        if let Some(ref single) = detector.single_metric_anomaly_detector {
            let dims = single
                .dimensions
                .iter()
                .map(|dim| format!("{}={}", dim.name, dim.value))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{}/{}/{}/{}",
                single.namespace.as_deref().unwrap_or(""),
                single.metric_name.as_deref().unwrap_or(""),
                single.stat.as_deref().unwrap_or(""),
                dims,
            )
        } else {
            uuid::Uuid::new_v4().to_string()
        }
    }

    /// Store an anomaly detector.
    pub fn put(&self, detector: AnomalyDetector) {
        let key = Self::make_key(&detector);
        self.detectors.insert(key, detector);
    }

    /// Delete an anomaly detector.
    #[must_use]
    pub fn delete(&self, detector: &AnomalyDetector) -> bool {
        let key = Self::make_key(detector);
        self.detectors.remove(&key).is_some()
    }

    /// List all anomaly detectors.
    #[must_use]
    pub fn list(&self) -> Vec<AnomalyDetector> {
        self.detectors.iter().map(|e| e.value().clone()).collect()
    }
}
