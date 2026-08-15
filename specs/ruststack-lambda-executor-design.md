# Rustack Lambda: Real Function Execution (Native + Docker Backends)

**Date:** 2026-04-19
**Status:** Draft / Implementation Plan
**Depends on:** [ruststack-lambda-design.md](./ruststack-lambda-design.md)
**Scope:** Replace the stubbed `Invoke` echo response with a real execution
engine. Two backends: a **native** process backend that runs `provided.*`
Rust/Go/C++ bootstraps directly on the host, and a **docker** backend that runs
any supported runtime in an AWS Lambda base image. Both share a single
in-process **Lambda Runtime API** server.

---

## Table of Contents

1. [Motivation](#1-motivation)
2. [Goals and Non-Goals](#2-goals-and-non-goals)
3. [Architecture](#3-architecture)
4. [Lambda Runtime API Server](#4-lambda-runtime-api-server)
5. [Executor Trait & Backend Selection](#5-executor-trait--backend-selection)
6. [Native Backend](#6-native-backend)
7. [Docker Backend](#7-docker-backend)
8. [Code Lifecycle: Zip → Extracted Layout](#8-code-lifecycle-zip--extracted-layout)
9. [Warm Pool, Idle Reaper, Shutdown](#9-warm-pool-idle-reaper-shutdown)
10. [Configuration](#10-configuration)
11. [Provider/Handler Integration](#11-providerhandler-integration)
12. [Phased Implementation Plan](#12-phased-implementation-plan)
13. [Integration Test Plan](#13-integration-test-plan)
14. [Risks & Open Questions](#14-risks--open-questions)

---

## 1. Motivation

Today `RustackLambda::invoke` (provider.rs:638-682) is a stub:

- Validates the function exists.
- Returns 204 for `DryRun`.
- Returns `DockerNotAvailable` if `LAMBDA_DOCKER_ENABLED=false`.
- If true, returns a hard-coded JSON echo. **No container is ever started.**

Stored zips are persisted to disk but never extracted (storage.rs:602-626). No
runtime API server exists. There is no execution backend at all.

Goal: produce a usable `aws lambda invoke` end-to-end against rustack, with
Rust `provided.*` lambdas working **without Docker** on a developer's host.

## 2. Goals and Non-Goals

### Goals

1. **Real synchronous invocation** for `provided.al2` / `provided.al2023`
   functions (Rust, Go, C++ bootstraps) using a native process backend on
   matching host arch + OS.
2. **Real synchronous invocation** for any AWS-supported runtime via a Docker
   backend using the official `public.ecr.aws/lambda/*` images (Runtime
   Interface Emulator built in).
3. **Async (`Event`) invocation** with an internal queue + background worker
   (single attempt, fire-and-forget).
4. **Function error propagation** — `X-Amz-Function-Error: Unhandled` set when
   the bootstrap reports an error via `/runtime/invocation/{id}/error`.
5. **Warm reuse** of running bootstraps within a configurable idle window.
6. **Backend selection that's automatic but overridable** — pick native when
   possible, Docker otherwise; honour `LAMBDA_EXECUTOR=native|docker|disabled`
   when set.
7. **Integration tests** built around a workspace-built Rust echo bootstrap.

### Non-Goals

1. CloudWatch log persistence (`LogResult` header is best-effort tail of stderr
   only when requested).
2. Multi-tenancy isolation / sandboxing of the native backend (it's a dev tool;
   user code runs as the rustack process user).
3. VPC / IAM / KMS-encrypted env / SnapStart / Layers / EFS — all no-op.
4. Cold-start budget enforcement, concurrent execution limits, throttling.
5. Function URL invocation path (orthogonal — covered in the parent spec).
6. External S3 endpoints as code source (rustack's own S3 is supported via the in-process bridge — see [ruststack-lambda-s3-code-design](./ruststack-lambda-s3-code-design.md)).

## 3. Architecture

```text
                           +-------------------------------+
                           |  RustackLambda (provider)     |
                           |   - FunctionStore             |
                           |   - LambdaConfig              |
                           |   - executor: Arc<dyn Executor>|
                           +--------------+----------------+
                                          |
                        async invoke(name, qualifier, payload, type)
                                          v
                           +--------------+----------------+
                           |  Executor (trait)             |
                           |    invoke(req) -> Resp        |
                           |    shutdown()                 |
                           +-+----------+----------+-------+
                             |          |          |
                    NoopExecutor  NativeExecutor  DockerExecutor
                                       |               |
                                       v               v
                           +-----------+---+    +------+--------------+
                           | InstancePool  |    |   Same pool shape   |
                           |  per fn-key   |    |   (boxed backend)   |
                           +---+-----------+    +---------------------+
                               |
                               v
                        +------+--------------------------------+
                        | Instance                              |
                        |  - RuntimeApiSocket (127.0.0.1:rand)  |
                        |  - Job queue (mpsc)                   |
                        |  - Pending map (req_id -> oneshot)    |
                        |  - Backend handle (process / container)|
                        |  - last_used: Instant                 |
                        +---------------------------------------+
```

The same `Instance` machinery (port + queue + pending map) is shared between
backends. The only difference is **how** we start the bootstrap that polls the
runtime API socket.

## 4. Lambda Runtime API Server

The Lambda Runtime API is the contract every `provided.*` runtime, plus the
RIE built into AWS base images, speaks. Rustack will host one Runtime API
**socket per running instance** (i.e. per warm process or container), to keep
invocation routing trivial — the runtime polls and gets exactly the
invocations destined for that instance.

### 4.1 Endpoints

All paths are under `/2018-06-01/runtime/`:

| Path | Method | Purpose |
|------|--------|---------|
| `/invocation/next` | GET | Long-poll for the next invocation. Returns body + headers below. |
| `/invocation/{id}/response` | POST | Bootstrap delivers the success response. |
| `/invocation/{id}/error` | POST | Bootstrap delivers a function error. |
| `/init/error` | POST | Bootstrap reports init-time failure. |

Response headers on `/invocation/next`:

- `Lambda-Runtime-Aws-Request-Id`: UUID for this invocation.
- `Lambda-Runtime-Deadline-Ms`: Unix ms when timeout fires.
- `Lambda-Runtime-Invoked-Function-Arn`: qualified function ARN.
- `Lambda-Runtime-Trace-Id`: synthetic X-Ray trace header.
- `Content-Type`: passed through (`application/json` by default).

### 4.2 Crate placement

A new submodule `rustack_lambda_core::executor::runtime_api` holds:

- `RuntimeApiSocket` — owns a hyper server bound to `127.0.0.1:0` and a
  `tokio::sync::mpsc::Sender<Job>` for handing invocations to it.
- `Job { request_id, payload, deadline, function_arn, response_tx }`.
- `RuntimeApiHandle { addr: SocketAddr, send_invocation, shutdown_tx }`.

The socket runs on its own tokio task; shutdown is signalled with a
`tokio::sync::watch::Sender<bool>` consumed by the accept loop.

### 4.3 Concurrency model

A single instance handles **one in-flight invocation at a time** (matches AWS
behavior — concurrency = number of instances). Long-polling on `/next` blocks
until a job is queued; the bootstrap then runs, posts to `/response` (or
`/error`), the corresponding `oneshot` is fired, and the orchestrator reuses
the instance for the next job.

Implementation: each instance owns:

```rust
struct Instance {
    addr: SocketAddr,                                            // for runtime api
    job_tx: mpsc::Sender<Job>,                                   // to runtime
    pending: Arc<DashMap<String, oneshot::Sender<RuntimeResult>>>,
    shutdown_tx: watch::Sender<bool>,
    backend_handle: BackendHandle,                               // process or container
    last_used: Mutex<Instant>,
    busy: AtomicBool,
}
```

`pending` is shared with the runtime API task: when `/response` or `/error` is
posted, the task removes the entry and fires the oneshot.

## 5. Executor Trait & Backend Selection

### 5.1 Trait

```rust
#[async_trait::async_trait]
pub trait Executor: std::fmt::Debug + Send + Sync + 'static {
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError>;
    async fn shutdown(&self);
}
```

Object-safe — the provider holds `Arc<dyn Executor>` so the binary can swap
backends at startup without a generic explosion through the call graph.

### 5.2 Request / Response

```rust
pub struct InvokeRequest {
    pub function_arn: String,
    pub function_name: String,
    pub qualifier: String,                           // resolved version, e.g. "$LATEST" or "3"
    pub runtime: Option<String>,                     // e.g. "provided.al2023"
    pub handler: Option<String>,                     // e.g. "bootstrap"
    pub architectures: Vec<String>,                  // ["x86_64"] or ["arm64"]
    pub package_type: PackageType,                   // Zip | Image
    pub code_root: Option<PathBuf>,                  // unzipped dir (Zip)
    pub image_uri: Option<String>,                   // (Image)
    pub environment: HashMap<String, String>,
    pub timeout: Duration,
    pub memory_mb: u32,
    pub payload: Bytes,
    pub capture_logs: bool,
}

pub struct InvokeResponse {
    pub status: u16,                                 // 200 success / 200 with FunctionError
    pub payload: Bytes,
    pub function_error: Option<String>,              // "Unhandled" if user code errored
    pub log_tail: Option<String>,                    // base64 last 4KB of stderr (when requested)
    pub executed_version: String,
}
```

### 5.3 Backend selection

`LambdaConfig` gains:

```rust
pub enum ExecutorBackend {
    Disabled,   // current echo behavior — kept for tests
    Auto,      // pick native if it can, else docker
    Native,
    Docker,
}
```

Resolved at startup from `LAMBDA_EXECUTOR` (`disabled|auto|native|docker`).
When the env var is unset, the rustack server defaults to `native` —
zero-setup real execution for `provided.*` Rust/Go/C++ lambdas. Setting the
legacy `LAMBDA_DOCKER_ENABLED=true` still wins and maps to
`LAMBDA_EXECUTOR=docker`. The `LambdaConfig::default()` builder (used in
unit tests) stays on `disabled` so library tests don't spawn real
processes.

Auto rule, evaluated **per invocation**:

```text
if package_type == Image:
    docker (only)
elif runtime in {provided.al2, provided.al2023}
     and host_arch matches one of req.architectures
     and host_os == "linux" OR (we're on macOS and a darwin bootstrap is detected):
    native
else:
    docker
```

Detection: the native backend probes the extracted code root for an executable
file named `bootstrap`. It uses the first 4 bytes (`\x7fELF`, `\xCF\xFA\xED\xFE`,
`\xFE\xED\xFA\xCE`, `\xCE\xFA\xED\xFE`) plus `architectures` membership to
decide if it can run it on the host. Mismatch → fall through to docker (or
return `InvalidRuntimeForHost` if Docker is also disabled).

## 6. Native Backend

### 6.1 Spawning a bootstrap

For an instance about to run function `name@$LATEST`:

1. Resolve `code_root = {storage_dir}/{name}/{version}/extracted`.
2. Validate `bootstrap` exists, has the correct magic bytes for host arch/OS,
   and `chmod +x` it (idempotent on already-executable files).
3. Bind a `RuntimeApiSocket` on `127.0.0.1:0`, get its port.
4. `tokio::process::Command::new(code_root.join("bootstrap"))`
   - `current_dir(code_root)`
   - `env_clear()`
   - `env(...)` with the standard Lambda env (see below) plus user env vars
   - `stdout(Stdio::piped())`, `stderr(Stdio::piped())`
   - `kill_on_drop(true)`
5. Spawn two tasks to capture stdout/stderr line-by-line into a bounded
   ring-buffer (`VecDeque<String>` with max ~4KB, used for `LogResult`).

### 6.2 Standard Lambda env vars

| Var | Value |
|------|-------|
| `AWS_LAMBDA_RUNTIME_API` | `127.0.0.1:<port>` |
| `AWS_LAMBDA_FUNCTION_NAME` | function name |
| `AWS_LAMBDA_FUNCTION_VERSION` | resolved version |
| `AWS_LAMBDA_FUNCTION_MEMORY_SIZE` | memory_mb |
| `AWS_LAMBDA_FUNCTION_TIMEOUT` | timeout secs |
| `AWS_LAMBDA_LOG_GROUP_NAME` | `/aws/lambda/<name>` |
| `AWS_LAMBDA_LOG_STREAM_NAME` | UUID |
| `AWS_REGION`, `AWS_DEFAULT_REGION` | from config |
| `AWS_ACCESS_KEY_ID` | `test` (matches Rustack defaults) |
| `AWS_SECRET_ACCESS_KEY` | `test` |
| `AWS_SESSION_TOKEN` | `test` |
| `_HANDLER` | function handler |
| `LAMBDA_TASK_ROOT` | code_root |
| `LAMBDA_RUNTIME_DIR` | code_root |
| `TZ` | `:UTC` |

User env vars from the function config are merged on top (allowed to override
none of `AWS_LAMBDA_RUNTIME_API`, `_HANDLER`, `LAMBDA_TASK_ROOT`).

### 6.3 Lifecycle

- Spawned process **must** call `/next` within 5 s or we mark the instance
  unhealthy and reap it (init failure).
- On invoke: send a `Job` into the instance's queue. Wait on the oneshot
  with the function timeout.
- On timeout: kill the process, return `FunctionError: "Unhandled"` with body
  `{"errorMessage":"Task timed out after N.N seconds","errorType":"Sandbox.Timedout"}`.
- On bootstrap exit during invocation: same as timeout but with
  `errorType: "Runtime.ExitError"`.

## 7. Docker Backend

### 7.1 Dependency choice

Use **`bollard`** (~0.18) — pure-Rust async Docker client, already named in
the parent spec, listed as `~0.18` in workspace deps when this phase lands.
Falls back to `DOCKER_HOST` env, `unix:///var/run/docker.sock` default.

### 7.2 Image selection

| Runtime | Image |
|---------|-------|
| `provided.al2023` | `public.ecr.aws/lambda/provided:al2023` |
| `provided.al2` | `public.ecr.aws/lambda/provided:al2` |
| `python3.12` | `public.ecr.aws/lambda/python:3.12` |
| `nodejs20.x` | `public.ecr.aws/lambda/nodejs:20` |
| `Image` package | `image_uri` directly |
| anything else | `InvalidRuntime` for now |

Add more in later phases.

### 7.3 Container creation

For each new instance:

1. Pull image (cache hit if seen).
2. Bind the RuntimeApiSocket on the host on `127.0.0.1:0`.
3. Resolve the host address visible from inside the container:
   - `host.docker.internal` on Docker Desktop (Linux & macOS, recent versions
     resolve via the magic IP `host-gateway`).
   - On Linux engines without the magic name, add
     `--add-host=host.docker.internal:host-gateway` to host config.
4. Create the container:
   - Image: per the table above.
   - Cmd: handler (e.g. `bootstrap`) — base images use this as `_HANDLER`.
   - Env: same map as the native backend, but `AWS_LAMBDA_RUNTIME_API` =
     `host.docker.internal:<host_port>`.
   - HostConfig: `binds: ["{code_root}:/var/task:ro"]`, `memory: memory_mb*MiB`,
     `network_mode: "bridge"` (or `LAMBDA_DOCKER_NETWORK`).
   - Labels: `rustack.lambda.function=<name>`, `rustack.lambda.version=<v>`,
     `rustack.lambda.instance=<uuid>` for cleanup.
5. Start container.
6. Same liveness rule as native: must hit `/next` within 5 s.

### 7.4 Cleanup

- On idle reap or shutdown: `stop` (with 1 s grace) then `remove`.
- On startup, list containers with label `rustack.lambda.instance` and remove
  them — handles unclean prior shutdown.

## 8. Code Lifecycle: Zip → Extracted Layout

`store_zip_code` (storage.rs:602) currently writes only `code.zip`. Add
extraction:

```text
{code_dir}/{function_name}/{version}/
    code.zip                       # raw bytes (kept for GetFunction)
    extracted/                     # new — unzipped contents
        bootstrap                  # for provided.* runtimes
        lib/...
```

Changes:

1. After writing `code.zip`, open it with the `zip` crate, iterate entries,
   write each to `extracted/`. Preserve unix mode bits (so `bootstrap` stays
   executable). Reject entries that escape `extracted/` (path traversal).
2. Return `extracted_dir` as the `code_path` (it's the existing field's
   semantic meaning per the parent spec). The raw zip path is recoverable
   via `dir.join("code.zip")` if needed.
3. `cleanup_code` already removes the whole `{name}` tree — no change.

`update_function_code` follows the same path on the `$LATEST` slot.

`PublishVersion` does not duplicate code on disk; the published `VersionRecord`
clones `code_path` from `$LATEST` (already the case in the existing
`publish_version` impl).

## 9. Warm Pool, Idle Reaper, Shutdown

### 9.1 Pool key

`(function_name, qualifier_resolved)` — distinct $LATEST and version 3 use
distinct pools because their code can diverge.

### 9.2 Acquire / release

```text
acquire(key):
    pool = pools.entry(key).or_default();
    if let Some(idle) = pool.pop() and not idle.is_dead():
        idle.busy = true; return idle
    new_instance = backend.start(key, fn_config); return new_instance

release(instance):
    instance.last_used = now()
    instance.busy = false
    pool[key].push(instance)        // bounded — overflow → drop_and_kill
```

Pool max per key: `LAMBDA_MAX_WARM_INSTANCES` (default 1; tests use 1 by
default for determinism, prod can go higher).

### 9.3 Idle reaper

Spawn one background task per `Executor`. Every 30 s, scan all pools and
remove instances where `now - last_used > LAMBDA_IDLE_TIMEOUT_SECS` (default
600). Killed instances are detached on a tokio task to avoid blocking the
reaper.

Reaper exits when the executor's `shutdown()` is called via a
`watch::Sender<bool>`.

### 9.4 Shutdown

`Executor::shutdown` walks the pool, kills all instances (process or
container), drops the runtime API sockets, awaits the reaper task. Wired into
the rustack server's `tokio::signal::ctrl_c` shutdown branch (main.rs:489).

## 10. Configuration

`LambdaConfig` gains:

```rust
pub executor: ExecutorBackend,                     // env: LAMBDA_EXECUTOR
pub max_warm_instances: usize,                     // env: LAMBDA_MAX_WARM_INSTANCES
pub idle_timeout: Duration,                        // env: LAMBDA_IDLE_TIMEOUT_SECS
pub init_timeout: Duration,                        // env: LAMBDA_INIT_TIMEOUT_SECS
pub docker_image_overrides: HashMap<String,String>,// env: LAMBDA_DOCKER_IMAGE_<RUNTIME>
pub docker_network: Option<String>,                // env: LAMBDA_DOCKER_NETWORK
```

Existing `docker_enabled` is preserved. If the user sets only
`LAMBDA_DOCKER_ENABLED=true` (legacy), it maps to `executor=Docker`.

## 11. Provider/Handler Integration

### 11.1 Provider

`RustackLambda::invoke` becomes `async`. Signature:

```rust
pub async fn invoke(
    &self,
    function_ref: &str,
    qualifier: Option<&str>,
    payload: &[u8],
    invocation_type: InvocationType,
) -> Result<InvokeOutcome, LambdaServiceError>;

pub enum InvokeOutcome {
    DryRun,                                       // 204
    Async { request_id: String },                  // 202
    Sync(InvokeResponse),                          // 200 + payload + optional FunctionError
}
```

DryRun and missing-function checks stay; Async pushes the job onto an internal
mpsc consumed by a `tokio::spawn`-ed worker that calls `executor.invoke`
fire-and-forget.

### 11.2 Handler

`handler.rs` lines 200-247 — replace the three `provider.invoke(...)` calls
with one awaited call, then map `InvokeOutcome`:

- `DryRun` → 204
- `Async` → 202 with empty body
- `Sync` → 200 with payload, optional `X-Amz-Function-Error`,
  `X-Amz-Executed-Version`, `X-Amz-Log-Result`.

### 11.3 App wiring

`apps/rustack/src/main.rs:985` builds `RustackLambda::new(...)`. We add a
factory `RustackLambda::from_env_with_executor(config)` that constructs the
`Executor` based on `config.executor` and stores it. Shutdown hook added to
`serve` so ctrl-c calls `lambda_provider.shutdown().await` for active
executors.

## 12. Phased Implementation Plan

Each phase is independently shippable and gated by tests.

### Phase 1 — Code extraction + executor trait scaffolding

- Add `zip = "2"` workspace dep.
- `storage::FunctionStore::store_zip_code` extracts into `extracted/`,
  preserves unix mode, rejects traversal. Returns extracted dir as
  `code_path`.
- New module `executor::{mod, types, error, noop, runtime_api}`.
- `Executor` trait + `NoopExecutor` (current echo behavior, lets existing
  tests pass).
- `RustackLambda` holds `Arc<dyn Executor>`; `invoke` becomes async,
  delegates to the executor.
- Update `handler.rs` Invoke arm to await the new async signature.
- Add `LAMBDA_EXECUTOR` parsing to `LambdaConfig`; default `Disabled` for
  this phase.
- Tests: store_zip_code extracts a real zip with a `bootstrap` file +
  permissions; provider Invoke still works (echoes via NoopExecutor).

### Phase 2 — Lambda Runtime API server

- Implement `RuntimeApiSocket` on `127.0.0.1:0` using `hyper` directly (no new
  deps).
- `Job` queue + pending map.
- Unit tests: spawn the socket, send a job, simulate a "bootstrap" with a
  `reqwest` client that GETs `/next`, posts back to `/response`, verify the
  oneshot fires with the body.
- Same test for `/error` path → expect `FunctionError = "Unhandled"`.

### Phase 3 — Native backend

- `executor::native::NativeExecutor` with `InstancePool`.
- Bootstrap detection: existence + magic-byte check + arch match.
- `tokio::process::Command::spawn` with full env map.
- Stdout/stderr ring buffer for `LogResult`.
- Idle reaper task + shutdown.
- Wire into `RustackLambda::from_env_with_executor` for `LAMBDA_EXECUTOR=native`
  and the `auto` rule when host matches.
- Unit tests use the workspace fixture binary (Phase 5) — actual end-to-end
  validation lives in integration tests.

### Phase 4 — Docker backend

- Add `bollard = "~0.18"` workspace dep (lambda-core only).
- `executor::docker::DockerExecutor` mirrors `NativeExecutor` but starts
  containers.
- Image resolution map; image pull cache.
- Host-gateway address resolution.
- Container labelling + startup orphan cleanup.
- Tests gated on `LAMBDA_DOCKER_TESTS=1` because they require Docker.

### Phase 5 — Integration tests

- New workspace member `tests/lambda-fixtures/echo-bootstrap` — a tiny
  `[[bin]]` named `bootstrap` that polls the runtime API and echoes the
  payload. Adds two endpoints' worth of behavior:
  - default: echoes the request payload back as `{"echo": <body>}`
  - if env `FAIL_MODE=panic`: posts to `/error` with a known shape
- Integration test `tests/integration/src/test_lambda_invoke.rs`:
  - Spawns the rustack server **in-process** on a random port (reusing the
    pattern from existing tests if one exists; otherwise add a small
    `spawn_server_for_lambda_tests()` helper).
  - For each test: builds the fixture (`cargo build --release -p
    lambda-echo-bootstrap` once via `OnceCell`), zips the binary as
    `bootstrap`, calls `CreateFunction`, calls `Invoke`, asserts roundtrip.
  - Cases:
    - sync echo roundtrip
    - dry-run returns 204
    - async returns 202
    - function error path (FAIL_MODE=panic) → `X-Amz-Function-Error` set
    - timeout → `errorType=Sandbox.Timedout`
    - warm reuse — invoke twice, verify second is faster (loose threshold)
  - All native-only tests are `#[ignore]` and gated on
    `cfg!(target_os = "linux") || cfg!(target_os = "macos")` plus a
    `LAMBDA_NATIVE_TESTS=1` env.
- Optional Docker integration test in same file, gated on
  `LAMBDA_DOCKER_TESTS=1`.

### Phase 6 — Validation

- `cargo build`, `cargo +nightly fmt`, `cargo clippy -- -D warnings`,
  `cargo test`. New Makefile targets:
  - `test-lambda-invoke-native`
  - `test-lambda-invoke-docker`

## 13. Integration Test Plan

### 13.1 Fixture binary

`tests/lambda-fixtures/echo-bootstrap/src/main.rs` — hand-rolled to avoid
depending on `lambda_runtime` (keeps the fixture small, no opaque magic):

```rust
fn main() -> Result<()> {
    let api = std::env::var("AWS_LAMBDA_RUNTIME_API")?;
    let client = reqwest::blocking::Client::new();
    loop {
        let resp = client.get(format!("http://{api}/2018-06-01/runtime/invocation/next")).send()?;
        let request_id = resp.headers().get("Lambda-Runtime-Aws-Request-Id")
            .ok_or_else(|| anyhow!("missing request id"))?
            .to_str()?.to_owned();
        let body = resp.bytes()?;

        if std::env::var("FAIL_MODE").as_deref() == Ok("panic") {
            client.post(format!("http://{api}/2018-06-01/runtime/invocation/{request_id}/error"))
                .json(&serde_json::json!({"errorMessage":"boom","errorType":"TestError"}))
                .send()?;
            continue;
        }

        let echo = serde_json::json!({"echo": serde_json::from_slice::<serde_json::Value>(&body).ok()});
        client.post(format!("http://{api}/2018-06-01/runtime/invocation/{request_id}/response"))
            .json(&echo)
            .send()?;
    }
}
```

### 13.2 Test harness

Tests use the existing aws-sdk-lambda client against an in-process server.
Where today the server is started externally, lambda invoke tests need an
in-process server because the native backend uses host paths under the
`tempfile::tempdir` we own. Helper:

```rust
async fn spawn_lambda_test_server() -> (SocketAddr, ShutdownHandle, TempDir) { /* ... */ }
```

This builds a `RustackLambda` with `LAMBDA_EXECUTOR=native`, the lambda HTTP
service, and serves it on a random port. Returns the URL the SDK should hit.

### 13.3 Coverage matrix

| Test | Backend | Notes |
|------|---------|-------|
| `test_should_invoke_native_echo_sync` | native | golden path |
| `test_should_invoke_dry_run_returns_204` | native | no spawn |
| `test_should_invoke_event_returns_202` | native | async drains in background |
| `test_should_propagate_function_error` | native | FAIL_MODE=panic |
| `test_should_timeout_long_running` | native | sleep handler, timeout=1 |
| `test_should_warm_reuse_instance` | native | second invoke <100ms |
| `test_should_invoke_docker_echo_sync` | docker | gated |

## 14. Risks & Open Questions

| Risk | Mitigation |
|------|------------|
| Runtime API protocol drift | Pinned to v2018-06-01 path which is stable; runtime contract documented. |
| Zombie processes on rustack crash | `kill_on_drop(true)` on `Command`; PID file scan on startup as belt-and-braces. |
| Port exhaustion under heavy warm-pool churn | Bind on `127.0.0.1:0`, drop sockets immediately on instance shutdown. |
| Docker socket permission on macOS | Document `DOCKER_HOST=unix://$HOME/.docker/run/docker.sock`. |
| Native backend running untrusted code | Out of scope — it's a dev tool; document in README. |
| Cross-arch bootstrap | Magic-byte check refuses to spawn; falls back to docker. |
| Test flakiness from process startup time | Init-readiness timeout = 5 s; warm-reuse test uses a `>2x faster` ratio rather than absolute time. |
