//! Bounded local Docker wire service and strict attach framing.
//!
//! This layer owns HTTP connections, not container policy or lifetime. Callers
//! retain preallocated identities and reservations through reconciliation.

mod connection;
mod environment;
mod framing;
mod image;
mod lease;
mod logs;
mod mutation;
mod ownership;
mod pin;
mod plan;
mod pool;
mod types;

use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

/// Generated Docker model contracts, without the upstream network client.
pub use bollard_stubs::models;
use bytes::Bytes;
use connection::exchange;
pub use environment::{
    EnvironmentCapacity, EnvironmentConfig, EnvironmentError, EnvironmentHandle,
    EnvironmentManager, EnvironmentPhase, EnvironmentPort, EnvironmentResponse,
    EnvironmentSettings, EnvironmentStatus,
};
pub use framing::{DockerOutputReader, MAX_DOCKER_RECORD, OutputChannel, OutputChunk};
use http::{Method, StatusCode};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
pub use image::ImageResolver;
pub use lease::{ImageLeaseConfig, ImageLeaseManager, ImageLeasePort, ImageLeaseStatus};
pub use logs::{FunctionLogs, LogSnapshot};
use models::{
    ContainerCreateBody, ContainerCreateResponse, ContainerInspectResponse, ContainerSummary,
    ContainerWaitResponse, ImageInspect, SystemInfo, SystemVersion,
};
pub use ownership::{DirectoryLease, DirectoryLeaseError};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
pub use pin::{ImagePin, ImagePinScope};
pub use plan::{ContainerPolicy, EnvironmentPlan, NetworkSelection, RelayBudgets};
pub use pool::{EnvironmentPool, PoolConfig, PoolPort, PoolSettings, PoolStatus};
use rustack_core::json::{JsonLimits, from_slice};
use rustack_lambda_runtime::diagnostics::runtime_dispatch;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{ReadHalf, WriteHalf, split},
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout_at},
};
use tracing::instrument::WithSubscriber;
use typed_builder::TypedBuilder;
pub use types::{ApiVersion, DockerError, DockerKey, DockerSocket};
use uuid::Uuid;

use crate::artifact::{Architecture, ImageReference};

const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
enum Operation {
    Version,
    Info,
    Image,
    TagPin,
    InspectPin,
    RemovePin,
    Create,
    Start,
    Inspect,
    Kill,
    Wait,
    Remove,
    List,
    Attach,
}
impl Operation {
    fn cleanup(self) -> bool {
        matches!(
            self,
            Self::Inspect
                | Self::Kill
                | Self::Remove
                | Self::List
                | Self::InspectPin
                | Self::RemovePin
        )
    }
}

struct RawRequest {
    mutation: Option<mutation::MutationGuard>,
    operation: Operation,
    method: Method,
    target: String,
    body: Bytes,
    expected: StatusCode,
}
impl fmt::Debug for RawRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerRequest")
            .field("operation", &self.operation)
            .field("bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}
enum RawReply {
    Body { status: StatusCode, bytes: Bytes },
    Upgrade(TokioIo<Upgraded>),
}
impl fmt::Debug for RawReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Body { status, bytes } => formatter
                .debug_struct("DockerReply")
                .field("status", status)
                .field("bytes", &bytes.len())
                .finish(),
            Self::Upgrade(_) => formatter.write_str("DockerUpgrade(<owned>)"),
        }
    }
}
#[derive(Debug)]
struct Envelope {
    request: RawRequest,
    deadline: Instant,
    reply: oneshot::Sender<Result<RawReply, DockerError>>,
}

#[derive(Debug, Clone)]
struct RawPort {
    business: mpsc::Sender<Envelope>,
    cleanup: mpsc::Sender<Envelope>,
    waits: mpsc::Sender<Envelope>,
    stop: watch::Sender<bool>,
}
impl RawPort {
    async fn request(
        &self,
        request: RawRequest,
        deadline: Instant,
    ) -> Result<RawReply, DockerError> {
        if *self.stop.borrow() {
            return Err(DockerError::Stopped);
        }
        check_deadline(deadline)?;
        if request.target.len() > 4096 || request.body.len() > MAX_BODY_BYTES {
            return Err(DockerError::TooLarge);
        }
        let sender = if matches!(request.operation, Operation::Wait) {
            &self.waits
        } else if request.operation.cleanup() {
            &self.cleanup
        } else {
            &self.business
        };
        let (reply, response) = oneshot::channel();
        sender
            .try_send(Envelope {
                request,
                deadline,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DockerError::Busy,
                mpsc::error::TrySendError::Closed(_) => DockerError::Stopped,
            })?;
        let result = timeout_at(deadline, response)
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker)?;
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        result
    }
}

/// A strict upgraded attach stream whose HTTP driver has already joined.
#[non_exhaustive]
pub struct DockerAttachment(TokioIo<Upgraded>);
impl fmt::Debug for DockerAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DockerAttachment(<owned>)")
    }
}

/// Strict stdout/stderr reader for a split upgraded Docker socket.
pub type AttachmentReader = DockerOutputReader<ReadHalf<TokioIo<Upgraded>>>;
/// Raw container stdin; the owning relay writer supplies framing/IO deadlines.
pub type AttachmentWriter = WriteHalf<TokioIo<Upgraded>>;

impl DockerAttachment {
    /// Transfer both socket halves into retained environment IO pumps.
    ///
    /// # Errors
    /// Rejects a zero/over60s record budget. Dropping either returned owner does
    /// not prove the external container has exited.
    ///
    /// ```no_run
    /// # fn split(attach: rustack_lambda_core::docker::DockerAttachment) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let (reader, writer) = attach.split(std::time::Duration::from_secs(5))?;
    /// # Ok(()) }
    /// ```
    pub fn split(
        self,
        budget: Duration,
    ) -> Result<(AttachmentReader, AttachmentWriter), DockerError> {
        let (reader, writer) = split(self.0);
        Ok((DockerOutputReader::new(reader, budget)?, writer))
    }
}

/// Fixed ownership-label filter; authorization to reclaim remains above wire IO.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct OwnerFilter {
    owner: Uuid,
    run: Option<Uuid>,
    environment: Option<Uuid>,
}
impl OwnerFilter {
    /// Validate nonnil UUID scope before issuing a label-filtered list.
    ///
    /// # Errors
    /// Rejects nil identities or an environment without a run scope.
    ///
    /// ```
    /// let filter = rustack_lambda_core::docker::OwnerFilter::new(uuid::Uuid::new_v4(), None, None)?;
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(
        owner: Uuid,
        run: Option<Uuid>,
        environment: Option<Uuid>,
    ) -> Result<Self, DockerError> {
        if owner.is_nil()
            || run.is_some_and(|id| id.is_nil())
            || environment.is_some_and(|id| id.is_nil())
            || (environment.is_some() && run.is_none())
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            owner,
            run,
            environment,
        })
    }
    fn encoded(self) -> Result<String, DockerError> {
        let mut labels = vec![format!("rustack.lambda.ownerId={}", self.owner)];
        if let Some(run) = self.run {
            labels.push(format!("rustack.lambda.runId={run}"));
        }
        if let Some(environment) = self.environment {
            labels.push(format!("rustack.lambda.environmentId={environment}"));
        }
        let filters = BTreeMap::from([("label", labels)]);
        let filters = serde_json::to_string(&filters).map_err(|_| DockerError::Schema)?;
        Ok(form_urlencoded::Serializer::new(String::new())
            .append_pair("all", "true")
            .append_pair("limit", "64")
            .append_pair("filters", &filters)
            .finish())
    }
}

/// Cloneable typed request port; it does not own service shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DockerPort {
    raw: RawPort,
    version: ApiVersion,
}

macro_rules! container_method {
    ($name:ident, $method:expr, $suffix:literal, $status:expr, $operation:ident, $result:ty, $decode:ident) => {
        #[doc = concat!("Issue the scoped container ", stringify!($operation), " operation.")]
        ///
        /// # Errors
        /// Returns bounded admission, deadline, transport, status or schema failure.
        ///
        #[doc = concat!("```no_run\n# async fn example(port: &rustack_lambda_core::docker::DockerPort, key: &rustack_lambda_core::docker::DockerKey) {\nlet result = port.", stringify!($name), "(key, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;\n# }\n```")]
        pub async fn $name(&self, key: &DockerKey, deadline: Instant) -> Result<$result, DockerError> {
            let path = format!("/containers/{}{}", escaped(key.as_str()), $suffix);
            let reply = self.call(Operation::$operation, $method, path, Bytes::new(), $status, deadline).await?;
            $decode(reply, deadline)
        }
    };
}

impl DockerPort {
    /// Negotiated API version, immutable after successful connection startup.
    ///
    /// ```no_run
    /// # fn inspect(port: &rustack_lambda_core::docker::DockerPort) {
    /// let version = port.api_version();
    /// # }
    /// ```
    #[must_use]
    pub fn api_version(&self) -> ApiVersion {
        self.version
    }

    /// Inspect bounded daemon capabilities; validate execution policy above IO.
    ///
    /// # Errors
    /// Returns admission, deadline, transport or bounded metadata/schema failure.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort) {
    /// let info = port.info(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn info(&self, deadline: Instant) -> Result<SystemInfo, DockerError> {
        decode(
            self.call(
                Operation::Info,
                Method::GET,
                "/info".into(),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }

    /// Inspect an already validated image reference without pulling it.
    ///
    /// # Errors
    /// Returns missing-image status, deadline, transport or metadata failure.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort, image: &rustack_lambda_core::artifact::ImageReference) {
    /// let info = port.inspect_image(image, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn inspect_image(
        &self,
        image: &ImageReference,
        deadline: Instant,
    ) -> Result<ImageInspect, DockerError> {
        decode(
            self.call(
                Operation::Image,
                Method::GET,
                format!("/images/{}/json", escaped(image.as_str())),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }

    /// Create under a preallocated name retained by the environment owner.
    ///
    /// # Errors
    /// Returns bounded request/schema, status, deadline or transport failure.
    /// A failed/dropped response still requires name/label reconciliation.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort, name: &rustack_lambda_core::docker::DockerKey, body: &rustack_lambda_core::docker::models::ContainerCreateBody) {
    /// let created = port.create_container(name, body, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn create_container(
        &self,
        name: &DockerKey,
        body: &ContainerCreateBody,
        deadline: Instant,
    ) -> Result<ContainerCreateResponse, DockerError> {
        self.create_container_inner(name, body, None, deadline, None)
            .await
    }

    /// Create from an immutable image ID using an explicit captured platform.
    ///
    /// # Errors
    /// Returns bounded request, status, deadline or transport failure; the owner
    /// must reconcile uncertain creation under the preallocated name/labels.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort, key: &rustack_lambda_core::docker::DockerKey, body: &rustack_lambda_core::docker::models::ContainerCreateBody) {
    /// let result = port.create_container_on_platform(key,body,
    ///     rustack_lambda_core::artifact::Architecture::Arm64,
    ///     tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn create_container_on_platform(
        &self,
        name: &DockerKey,
        body: &ContainerCreateBody,
        architecture: Architecture,
        deadline: Instant,
    ) -> Result<ContainerCreateResponse, DockerError> {
        self.create_container_inner(name, body, Some(architecture), deadline, None)
            .await
    }

    async fn create_container_inner(
        &self,
        name: &DockerKey,
        body: &ContainerCreateBody,
        architecture: Option<Architecture>,
        deadline: Instant,
        mutation: Option<mutation::MutationGuard>,
    ) -> Result<ContainerCreateResponse, DockerError> {
        let query = {
            let mut query = form_urlencoded::Serializer::new(String::new());
            query.append_pair("name", name.as_str());
            if let Some(architecture) = architecture {
                query.append_pair("platform", architecture.docker_platform());
            }
            query.finish()
        };
        decode(
            self.raw
                .request(
                    RawRequest {
                        mutation,
                        operation: Operation::Create,
                        method: Method::POST,
                        target: format!("/v{}/containers/create?{query}", self.version),
                        body: encode(body)?,
                        expected: StatusCode::CREATED,
                    },
                    deadline,
                )
                .await?,
            deadline,
        )
    }

    container_method!(
        start_container,
        Method::POST,
        "/start",
        StatusCode::NO_CONTENT,
        Start,
        (),
        unit
    );
    container_method!(
        inspect_container,
        Method::GET,
        "/json",
        StatusCode::OK,
        Inspect,
        ContainerInspectResponse,
        decode
    );
    container_method!(
        kill_container,
        Method::POST,
        "/kill?signal=SIGKILL",
        StatusCode::NO_CONTENT,
        Kill,
        (),
        unit
    );
    container_method!(
        wait_container,
        Method::POST,
        "/wait?condition=not-running",
        StatusCode::OK,
        Wait,
        ContainerWaitResponse,
        decode
    );
    container_method!(
        remove_container,
        Method::DELETE,
        "?force=true&v=false",
        StatusCode::NO_CONTENT,
        Remove,
        (),
        unit
    );

    /// List only a fixed ownership-label scope for reconciliation.
    ///
    /// # Errors
    /// Returns admission, deadline, status, transport or metadata-budget failure.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort, filter: rustack_lambda_core::docker::OwnerFilter) {
    /// let found = port.list_containers(filter, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn list_containers(
        &self,
        filter: OwnerFilter,
        deadline: Instant,
    ) -> Result<Vec<ContainerSummary>, DockerError> {
        decode(
            self.call(
                Operation::List,
                Method::GET,
                format!("/containers/json?{}", filter.encoded()?),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }

    /// Establish binary attach before starting the relay; no log replay.
    ///
    /// # Errors
    /// Returns invalid upgrade, status, deadline or joined transport failure.
    ///
    /// ```no_run
    /// # async fn example(port: &rustack_lambda_core::docker::DockerPort, relay: &rustack_lambda_core::docker::DockerKey) {
    /// let attached = port.attach(relay, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn attach(
        &self,
        relay: &DockerKey,
        deadline: Instant,
    ) -> Result<DockerAttachment, DockerError> {
        self.attach_inner(relay, true, deadline).await
    }

    /// Attach only live function output before start, without stdin or replay.
    ///
    /// # Errors
    /// Returns admission, upgrade, status, deadline or joined transport failure.
    ///
    /// ```no_run
    /// # async fn logs(port: &rustack_lambda_core::docker::DockerPort, function: &rustack_lambda_core::docker::DockerKey) {
    /// let attached = port.attach_logs(function, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn attach_logs(
        &self,
        function: &DockerKey,
        deadline: Instant,
    ) -> Result<DockerAttachment, DockerError> {
        self.attach_inner(function, false, deadline).await
    }

    async fn attach_inner(
        &self,
        key: &DockerKey,
        stdin: bool,
        deadline: Instant,
    ) -> Result<DockerAttachment, DockerError> {
        let reply = self
            .call(
                Operation::Attach,
                Method::POST,
                format!(
                    "/containers/{}/attach?stdin={stdin}&stdout=true&stderr=true&stream=true&\
                     logs=false",
                    escaped(key.as_str())
                ),
                Bytes::new(),
                StatusCode::SWITCHING_PROTOCOLS,
                deadline,
            )
            .await?;
        match reply {
            RawReply::Upgrade(io) => Ok(DockerAttachment(io)),
            RawReply::Body { .. } => Err(DockerError::Protocol),
        }
    }

    async fn call(
        &self,
        operation: Operation,
        method: Method,
        path: String,
        body: Bytes,
        expected: StatusCode,
        deadline: Instant,
    ) -> Result<RawReply, DockerError> {
        let target = format!("/v{}{path}", self.version);
        self.raw
            .request(
                RawRequest {
                    mutation: None,
                    operation,
                    method,
                    target,
                    body,
                    expected,
                },
                deadline,
            )
            .await
    }
}

/// Owner of bounded request scheduling and all in-flight management drivers.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockerWire {
    port: DockerPort,
    owner: Owner,
}

impl DockerWire {
    /// Verify the explicit socket and negotiate a bounded API response.
    ///
    /// # Errors
    /// Returns unavailable socket, malformed/incompatible API, IO or supervision
    /// failure. Dropped startup waiters retain cleanup through the owner task.
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let socket = "unix:///var/run/docker.sock".parse()?;
    /// let mut wire = rustack_lambda_core::docker::DockerWire::connect(socket,
    ///     tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await?;
    /// wire.shutdown().await?;
    /// # Ok(()) }
    /// ```
    pub async fn connect(socket: DockerSocket, deadline: Instant) -> Result<Self, DockerError> {
        tokio::runtime::Handle::try_current().map_err(|_| DockerError::Worker)?;
        let path = socket.verified(deadline).await?;
        let (mut owner, raw) = Owner::start(path);
        let result = async {
            let response = raw
                .request(
                    RawRequest {
                        mutation: None,
                        operation: Operation::Version,
                        method: Method::GET,
                        target: "/version".into(),
                        body: Bytes::new(),
                        expected: StatusCode::OK,
                    },
                    deadline,
                )
                .await?;
            let version: SystemVersion = decode(response, deadline)?;
            ApiVersion::negotiate(
                version.api_version.as_deref().ok_or(DockerError::Version)?,
                version
                    .min_api_version
                    .as_deref()
                    .ok_or(DockerError::Version)?,
            )
        }
        .await;
        match result {
            Ok(version) => Ok(Self {
                port: DockerPort { raw, version },
                owner,
            }),
            Err(error) => {
                owner.shutdown().await?;
                Err(error)
            }
        }
    }

    /// Clone a typed port; the original wire keeps shutdown responsibility.
    ///
    /// ```no_run
    /// # fn port(wire: &rustack_lambda_core::docker::DockerWire) {
    /// let port = wire.port();
    /// # }
    /// ```
    #[must_use]
    pub fn port(&self) -> DockerPort {
        self.port.clone()
    }

    /// Stop admission and join every retained management request/driver.
    ///
    /// # Errors
    /// Caches task supervision failures. Exported stream owners and external
    /// containers must be joined/removed by their environment owner separately.
    ///
    /// ```no_run
    /// # async fn shutdown(wire: &mut rustack_lambda_core::docker::DockerWire) {
    /// let settled = wire.shutdown().await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<(), DockerError> {
        self.owner.shutdown().await
    }
}

#[derive(Debug)]
struct Owner {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), DockerError>>>,
    settlement: Option<Result<(), DockerError>>,
}
impl Owner {
    fn start(socket: PathBuf) -> (Self, RawPort) {
        let (business, requests) = mpsc::channel(32);
        let (cleanup, priority) = mpsc::channel(8);
        let (waits, pending_waits) = mpsc::channel(8);
        let (stop, _) = watch::channel(false);
        let actor = Actor::builder()
            .socket(socket)
            .business(requests)
            .cleanup(priority)
            .waits(pending_waits)
            .control(stop.clone())
            .build();
        let task = tokio::spawn(actor.run().with_subscriber(runtime_dispatch()));
        (
            Self {
                stop: stop.clone(),
                task: Some(task),
                settlement: None,
            },
            RawPort {
                business,
                cleanup,
                waits,
                stop,
            },
        )
    }
    async fn shutdown(&mut self) -> Result<(), DockerError> {
        self.stop.send_replace(true);
        if let Some(result) = &self.settlement {
            return result.clone();
        }
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .map_err(|_| DockerError::Worker)
                .and_then(|result| result),
            None => Ok(()),
        };
        self.task.take();
        self.settlement = Some(result.clone());
        result
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[derive(Debug, TypedBuilder)]
struct Actor {
    socket: PathBuf,
    business: mpsc::Receiver<Envelope>,
    cleanup: mpsc::Receiver<Envelope>,
    waits: mpsc::Receiver<Envelope>,
    control: watch::Sender<bool>,
    #[builder(default)]
    jobs: JoinSet<(Lane, bool)>,
    #[builder(default)]
    counts: Counts,
}

#[derive(Debug, Clone, Copy)]
enum Lane {
    Business,
    Cleanup,
    Wait,
}
#[derive(Debug, Default)]
struct Counts {
    business: usize,
    cleanup: usize,
    waits: usize,
}
impl Counts {
    fn get_mut(&mut self, lane: Lane) -> &mut usize {
        match lane {
            Lane::Business => &mut self.business,
            Lane::Cleanup => &mut self.cleanup,
            Lane::Wait => &mut self.waits,
        }
    }
}
impl Actor {
    async fn run(mut self) -> Result<(), DockerError> {
        let mut stop = self.control.subscribe();
        let mut outcome = loop {
            tokio::select! {
                biased;
                () = connection::stopped(&mut stop) => break Ok(()),
                job = self.jobs.join_next(), if !self.jobs.is_empty() => match job {
                    Some(Ok((lane, false))) => {
                        let count = self.counts.get_mut(lane);
                        if let Some(value) = count.checked_sub(1) { *count = value; } else { break Err(DockerError::Worker); }
                    }
                    _ => break Err(DockerError::Worker),
                },
                envelope = self.cleanup.recv(), if self.counts.cleanup < 2 => match envelope { Some(envelope) => self.start(envelope, Lane::Cleanup), None => break Err(DockerError::Stopped) },
                envelope = self.waits.recv(), if self.counts.waits < 32 => match envelope { Some(envelope) => self.start(envelope, Lane::Wait), None => break Err(DockerError::Stopped) },
                envelope = self.business.recv(), if self.counts.business < 6 => match envelope { Some(envelope) => self.start(envelope, Lane::Business), None => break Err(DockerError::Stopped) },
            }
        };
        self.control.send_replace(true);
        self.business.close();
        self.cleanup.close();
        self.waits.close();
        while let Ok(envelope) = self.business.try_recv() {
            let _ = envelope.reply.send(Err(DockerError::Stopped));
        }
        while let Ok(envelope) = self.cleanup.try_recv() {
            let _ = envelope.reply.send(Err(DockerError::Stopped));
        }
        while let Ok(envelope) = self.waits.try_recv() {
            let _ = envelope.reply.send(Err(DockerError::Stopped));
        }
        while let Some(result) = self.jobs.join_next().await {
            if !matches!(result, Ok((_, false))) {
                outcome = Err(DockerError::Worker);
            }
        }
        outcome
    }
    fn start(&mut self, envelope: Envelope, lane: Lane) {
        if *self.control.borrow() {
            let _ = envelope.reply.send(Err(DockerError::Stopped));
            return;
        }
        if envelope.reply.is_closed() {
            return;
        }
        if Instant::now() >= envelope.deadline {
            let _ = envelope.reply.send(Err(DockerError::Deadline));
            return;
        }
        let socket = self.socket.clone();
        let stopped = self.control.subscribe();
        *self.counts.get_mut(lane) += 1;
        self.jobs.spawn(
            async move {
                let result = exchange(&socket, envelope.request, envelope.deadline, stopped).await;
                let failed = matches!(&result, Err(DockerError::Worker));
                let _ = envelope.reply.send(result);
                (lane, failed)
            }
            .with_subscriber(runtime_dispatch()),
        );
    }
}

fn escaped(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}
fn check_deadline(deadline: Instant) -> Result<(), DockerError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(DockerError::Deadline)?;
    if remaining > Duration::from_secs(600) {
        return Err(DockerError::Configuration);
    }
    Ok(())
}
fn decode<T: DeserializeOwned>(reply: RawReply, deadline: Instant) -> Result<T, DockerError> {
    if Instant::now() >= deadline {
        return Err(DockerError::Deadline);
    }
    let result = match reply {
        RawReply::Body { bytes, .. } => from_slice(
            &bytes,
            JsonLimits::new(MAX_BODY_BYTES, 32, 4096, 16 * 1024, 256)?,
        )
        .map_err(DockerError::from),
        RawReply::Upgrade(_) => Err(DockerError::Protocol),
    };
    if Instant::now() >= deadline {
        return Err(DockerError::Deadline);
    }
    result
}
fn unit(reply: RawReply, deadline: Instant) -> Result<(), DockerError> {
    if Instant::now() >= deadline {
        return Err(DockerError::Deadline);
    }
    match reply {
        RawReply::Body { bytes, .. } if bytes.is_empty() => Ok(()),
        _ => Err(DockerError::Protocol),
    }
}

#[derive(Default)]
struct Encoded {
    bytes: Vec<u8>,
    failure: Option<DockerError>,
}
impl fmt::Debug for Encoded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Encoded")
            .field("bytes", &self.bytes.len())
            .field("failure", &self.failure)
            .finish()
    }
}
impl Write for Encoded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_BODY_BYTES);
        if length.is_none() {
            self.failure = Some(DockerError::TooLarge);
            return Err(io::Error::other("Docker byte budget exceeded"));
        }
        if self.bytes.try_reserve(bytes.len()).is_err() {
            self.failure = Some(DockerError::Allocation);
            return Err(io::Error::other("Docker allocation failed"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(value: &impl Serialize) -> Result<Bytes, DockerError> {
    let mut output = Encoded::default();
    if serde_json::to_writer(&mut output, value).is_err() {
        return Err(output.failure.unwrap_or(DockerError::Schema));
    }
    Ok(Bytes::from(output.bytes))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use lease::tests as image_fixture;
