//! Gateway service that routes requests to registered AWS services.
//!
//! The gateway holds an ordered list of [`ServiceRouter`] implementations and
//! dispatches each request to the first router whose [`matches`](ServiceRouter::matches)
//! method returns `true`. If no router matches, a 404 response is returned.
//!
//! Health-check endpoints (`/_localstack/health`, `/_health`, `/health`) are
//! intercepted at the gateway level and return a combined status for all
//! registered services.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use hyper::{body::Incoming, service::Service};

use crate::service::{GatewayBody, ServiceRouter, gateway_body_from_string};

/// Gateway that routes incoming HTTP requests to registered service routers.
///
/// Services are tried in registration order; the first whose
/// [`ServiceRouter::matches`] returns `true` handles the request. Register
/// specific services (DynamoDB, etc.) before catch-all services (S3).
pub struct GatewayService {
    services: Arc<Vec<Box<dyn ServiceRouter>>>,
}

impl GatewayService {
    /// Create a new gateway from a list of service routers.
    pub fn new(services: Vec<Box<dyn ServiceRouter>>) -> Self {
        Self {
            services: Arc::new(services),
        }
    }

    /// Return the names of all registered services.
    pub fn service_names(&self) -> Vec<&'static str> {
        self.services.iter().map(|s| s.name()).collect()
    }
}

impl Clone for GatewayService {
    fn clone(&self) -> Self {
        Self {
            services: Arc::clone(&self.services),
        }
    }
}

impl Service<http::Request<Incoming>> for GatewayService {
    type Response = http::Response<GatewayBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        // Intercept health checks at the gateway level.
        if is_health_check(req.method(), req.uri().path()) {
            let services = Arc::clone(&self.services);
            return Box::pin(async move {
                let names: Vec<&str> = services.iter().map(|s| s.name()).collect();
                Ok(health_check_response(&names))
            });
        }

        // Route to the first matching service.
        for svc in self.services.iter() {
            if svc.matches(&req) {
                return svc.call(req);
            }
        }

        // No service matched — return a 404.
        Box::pin(async {
            Ok(http::Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(gateway_body_from_string(
                    r#"{"error":"no service matched the request"}"#,
                ))
                .expect("static 404 response should be valid"))
        })
    }
}

/// Check if the request is a health check probe.
fn is_health_check(method: &http::Method, path: &str) -> bool {
    *method == http::Method::GET
        && (path == "/_localstack/health"
            || path == "/_health"
            || path == "/health"
            || path == "/minio/health/live"
            || path == "/minio/health/ready"
            || path == "/minio/health/cluster")
}

/// Produce a health check response listing all registered services.
fn health_check_response(service_names: &[&str]) -> http::Response<GatewayBody> {
    let entries: Vec<String> = service_names
        .iter()
        .map(|name| format!(r#""{name}":"running""#))
        .collect();
    let body = format!(r#"{{"services":{{{}}}}}"#, entries.join(","));

    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(gateway_body_from_string(body))
        .expect("static health response should be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_detect_health_check_paths() {
        assert!(is_health_check(&http::Method::GET, "/_localstack/health"));
        assert!(is_health_check(&http::Method::GET, "/_health"));
        assert!(is_health_check(&http::Method::GET, "/health"));
        assert!(!is_health_check(&http::Method::POST, "/_health"));
        assert!(!is_health_check(&http::Method::GET, "/mybucket"));
    }

    #[test]
    fn test_should_produce_health_check_response_with_both_services() {
        let names = vec!["s3", "dynamodb"];
        let resp = health_check_response(&names);
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
        );
    }

    #[test]
    fn test_should_produce_health_check_response_with_single_service() {
        let names = vec!["dynamodb"];
        let resp = health_check_response(&names);
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    #[test]
    fn test_should_produce_health_check_response_with_no_services() {
        let names: Vec<&str> = vec![];
        let resp = health_check_response(&names);
        assert_eq!(resp.status(), http::StatusCode::OK);
    }
}
