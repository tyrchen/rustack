//! API Gateway v2 core business logic for RustStack.
//!
//! This crate implements the complete API Gateway v2 control plane, including
//! CRUD operations for APIs, routes, integrations, stages, deployments,
//! authorizers, models, domain names, VPC links, tags, and API mappings.
//!
//! The execution engine handles request routing and proxying to backend
//! integrations (Lambda proxy, HTTP proxy, and mock).

pub mod config;
pub mod error;
pub mod execution;
pub mod handler;
pub mod provider;
pub mod storage;
