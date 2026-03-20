//! CloudWatch Metrics business logic for `RustStack`.
//!
//! Implements metric storage, alarm management, dashboard CRUD,
//! anomaly detector metadata, metric stream metadata, and insight rules.

pub mod aggregation;
pub mod alarm_store;
pub mod anomaly_store;
pub mod config;
pub mod dashboard_store;
pub mod dimensions;
pub mod handler;
pub mod insight_store;
pub mod metric_store;
pub mod metric_stream_store;
pub mod provider;
pub mod validation;
