//! Gateway admission, routing, and explicit runtime health/capability reporting.

use std::{
    convert::Infallible,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use http::{
    Method, Response, StatusCode,
    header::{CONTENT_TYPE, RETRY_AFTER},
};
use http_body_util::BodyExt;
use hyper::{
    body::{Body, Frame, Incoming, SizeHint},
    service::Service,
};
use rustack_core::{
    http::{BodyBudget, BudgetedBody},
    settings::RuntimeBudgets,
};
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    runtime::RuntimeWorkers,
    service::{GatewayBody, ServiceRouter, gateway_body_from_string},
};

/// Process state is independent from whether any service supports snapshots.
#[derive(Debug)]
pub(crate) struct RuntimeStatus {
    ready: AtomicBool,
    workers: Arc<RuntimeWorkers>,
}

impl RuntimeStatus {
    pub(crate) fn drain(&self) {
        self.ready.store(false, Ordering::Release);
    }
}

/// Service capability description: registration is not full AWS compatibility.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capability {
    name: &'static str,
    status: &'static str,
    snapshot: &'static str,
    execution: &'static str,
}

/// Routable services with shared process-level in-flight admission.
#[derive(Clone)]
pub(crate) struct GatewayService {
    services: Arc<Vec<Box<dyn ServiceRouter>>>,
    admission: Arc<Semaphore>,
    budgets: RuntimeBudgets,
    state: Arc<RuntimeStatus>,
    health_only: bool,
}

impl GatewayService {
    pub(crate) fn new(services: Vec<Box<dyn ServiceRouter>>) -> Self {
        let budgets = rustack_core::settings::budgets();
        Self {
            services: Arc::new(services),
            admission: Arc::new(Semaphore::new(budgets.requests)),
            budgets,
            state: Arc::new(RuntimeStatus {
                ready: AtomicBool::new(true),
                workers: Arc::new(RuntimeWorkers::default()),
            }),
            health_only: false,
        }
    }

    pub(crate) fn with_workers(mut self, workers: Arc<RuntimeWorkers>) -> Self {
        self.state = Arc::new(RuntimeStatus {
            ready: AtomicBool::new(true),
            workers,
        });
        self
    }

    pub(crate) fn service_names(&self) -> Vec<&'static str> {
        self.services.iter().map(|service| service.name()).collect()
    }

    pub(crate) fn state(&self) -> Arc<RuntimeStatus> {
        Arc::clone(&self.state)
    }

    /// Reserved connections only serve diagnostics, never additional business load.
    pub(crate) fn health_only(mut self) -> Self {
        self.health_only = true;
        self
    }

    fn diagnostics(&self, req: &http::Request<Incoming>) -> Option<Response<GatewayBody>> {
        let path = req.uri().path();
        if !(is_health_check(req.method(), path)
            || matches!(*req.method(), Method::GET | Method::HEAD)
                && path == "/_rustack/capabilities")
        {
            return None;
        }
        let workers = self.state.workers.diagnostics();
        let ready =
            self.state.ready.load(Ordering::Acquire) && workers.ready && !self.services.is_empty();
        let status = if path == "/_health/live" || path == "/minio/health/live" || ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        let draining = !self.state.ready.load(Ordering::Acquire);
        let mut response = if path == "/_rustack/capabilities" {
            let capabilities: Vec<Capability> = self
                .services
                .iter()
                .map(|service| capability(service.name()))
                .collect();
            json_response(
                status,
                &serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "ready": ready, "services": capabilities, "workers": workers, "localEndpoints": {"apiGateway": "/_aws/execute-api/{apiId}/{stage}", "lambda": "/lambda-url/{functionName}/", "cloudFront": "/_aws/cloudfront/{distributionId}/"}, "limitations": ["No multi-tenant isolation or production IAM enforcement", "Snapshot resources-only services do not retain messages", "routed does not imply complete operation semantics"]}),
            )
        } else {
            let services: std::collections::BTreeMap<_, _> = self
                .services
                .iter()
                .map(|service| {
                    (
                        service.name(),
                        service_status(service.name(), draining, &workers),
                    )
                })
                .collect();
            json_response(
                status,
                &serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "ready": ready, "services": services, "workers": workers}),
            )
        };
        if *req.method() == Method::HEAD {
            *response.body_mut() = gateway_body_from_string("");
        }
        Some(response)
    }
}

impl Service<http::Request<Incoming>> for GatewayService {
    type Response = Response<GatewayBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        if let Some(response) = self.diagnostics(&req) {
            return Box::pin(async { Ok(response) });
        }
        if self.health_only || !self.state.ready.load(Ordering::Acquire) {
            return Box::pin(async {
                Ok(unavailable(
                    "runtime is draining or connection capacity is full",
                ))
            });
        }
        let Ok(permit) = Arc::clone(&self.admission).try_acquire_owned() else {
            return Box::pin(async { Ok(unavailable("request capacity is full")) });
        };
        let selected = self.services.iter().find(|service| service.matches(&req));
        // Object streams and synchronous invocations exceed the control deadline.
        let request_seconds = match selected.map(|service| service.name()) {
            Some("s3") => self.budgets.s3_body_total_seconds,
            Some("lambda")
                if req.uri().path().ends_with("/invocations")
                    || req.uri().path().starts_with("/lambda-url/") =>
            {
                self.budgets.lambda_invoke_seconds
            }
            _ => self.budgets.request_seconds,
        };
        let service_future = selected.map(|service| service.call(req));
        let budget = self.budgets.clone();
        Box::pin(async move {
            let response = match service_future {
                Some(future) => {
                    match tokio::time::timeout(Duration::from_secs(request_seconds), future).await {
                        Ok(Ok(response)) => response,
                        Err(_) => json_response(
                            StatusCode::GATEWAY_TIMEOUT,
                            &serde_json::json!({"error": "request deadline exceeded"}),
                        ),
                        Ok(Err(never)) => match never {},
                    }
                }
                None => json_response(
                    StatusCode::NOT_FOUND,
                    &serde_json::json!({"error": "no service matched the request"}),
                ),
            };
            let Ok(response_budget) = BodyBudget::new(
                budget.s3_object_body_bytes,
                Duration::from_secs(budget.s3_body_total_seconds),
                Duration::from_secs(budget.body_idle_seconds),
            ) else {
                return Ok(json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &serde_json::json!({"error": "invalid response budget"}),
                ));
            };
            Ok(response.map(|body| {
                PermitBody {
                    inner: BudgetedBody::new(body, response_budget),
                    permit: Some(permit),
                }
                .boxed()
            }))
        })
    }
}

struct PermitBody {
    inner: BudgetedBody<GatewayBody>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Body for PermitBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(None) => {
                this.permit.take();
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                this.permit.take();
                Poll::Ready(Some(Err(io::Error::other(error))))
            }
            other => other.map(|frame| frame.map(|result| result.map_err(io::Error::other))),
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

fn capability(name: &'static str) -> Capability {
    let snapshot = crate::snapshot::snapshot_coverage(name);
    let execution = match name {
        "lambda" => "explicit backend required; native is trusted-code-only",
        "sns" | "events" => "SQS targets only; target availability validated",
        _ => "partial AWS compatibility; see service capability documentation",
    };
    Capability {
        name,
        status: "partial",
        snapshot,
        execution,
    }
}

/// Derive the health state of one registered service. Disabled is an explicit
/// capability (e.g. Lambda without an execution backend), not a failure.
fn service_status(
    name: &'static str,
    draining: bool,
    workers: &crate::runtime::WorkerDiagnostics,
) -> &'static str {
    if draining {
        return "draining";
    }
    if let Some(status) = workers.services.get(name) {
        return status;
    }
    if name == "lambda"
        && rustack_core::settings::var("LAMBDA_EXECUTOR")
            .ok()
            .as_deref()
            == Some("disabled")
    {
        return "disabled";
    }
    "running"
}

fn unavailable(message: &str) -> Response<GatewayBody> {
    let mut response = json_response(
        StatusCode::SERVICE_UNAVAILABLE,
        &serde_json::json!({"error": message}),
    );
    response
        .headers_mut()
        .insert(RETRY_AFTER, http::HeaderValue::from_static("1"));
    response
}

fn json_response(status: StatusCode, value: &serde_json::Value) -> Response<GatewayBody> {
    let (status, body) = match serde_json::to_string(value) {
        Ok(body) => (status, body),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "{\"error\":\"diagnostic serialization failed\"}".to_owned(),
        ),
    };
    let mut response = Response::new(gateway_body_from_string(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    response
}

fn is_health_check(method: &Method, path: &str) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
        && matches!(
            path,
            "/_localstack/health"
                | "/_health"
                | "/health"
                | "/_health/live"
                | "/_health/ready"
                | "/minio/health/live"
                | "/minio/health/ready"
                | "/minio/health/cluster"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_support_head_and_distinct_health_paths() {
        assert!(is_health_check(&Method::HEAD, "/_health/ready"));
        assert!(is_health_check(&Method::GET, "/_health/live"));
        assert!(!is_health_check(&Method::POST, "/_health"));
        assert!(!is_health_check(&Method::GET, "/bucket"));
    }

    #[test]
    fn test_should_report_snapshot_limits_without_full_support_claim() {
        assert_eq!(capability("sqs").snapshot, "resources-only");
        assert_eq!(capability("sns").snapshot, "unsupported");
        assert_eq!(capability("lambda").status, "partial");
    }

    #[tokio::test]
    async fn test_should_keep_permit_until_response_body_is_consumed()
    -> Result<(), Box<dyn std::error::Error>> {
        let admission = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&admission).acquire_owned().await?;
        let body = PermitBody {
            inner: BudgetedBody::new(gateway_body_from_string("data"), BodyBudget::control()),
            permit: Some(permit),
        };
        assert_eq!(admission.available_permits(), 0);
        let _ = body.collect().await?;
        assert_eq!(admission.available_permits(), 1);
        Ok(())
    }
}
