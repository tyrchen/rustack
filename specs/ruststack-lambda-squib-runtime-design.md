# Rustack Lambda Squib Runtime Design

## Status

Draft implementation spec. The first implementation uses the local Squib `0.2.0` path crate from `../squib/crates/squib`; after Squib `0.2.0` is published, Rustack should switch this dependency to the registry version.

## Problem

Rustack Lambda can already run `provided.*` bootstraps directly on the host through the native executor. That path is useful for fast local feedback, but it is not an isolation boundary and cannot execute Linux `arm64` Lambda artifacts on macOS without a compatibility layer.

Squib provides a Firecracker-shaped microVM runtime for Apple Silicon. Its embeddable API can start a microVM from a static configuration file, and its vsock muxer exposes host Unix sockets for guest services. Rustack should add a Squib executor backend that starts the microVM once, stages a Lambda invocation over vsock, and returns a normal `InvokeResponse`.

## Goals

- Add `LAMBDA_EXECUTOR=squib` as an explicit Lambda executor backend.
- Use the local Squib path crate while the `0.2.0` release is unpublished.
- Require `arm64` Lambda functions for this backend.
- Keep Rustack's existing Lambda CRUD, storage, and API response behavior unchanged.
- Define a small, versionable host-to-guest invocation protocol over Squib's vsock muxer.
- Fail with actionable errors when the Squib config, vsock socket, function package, or guest agent is missing.
- Keep Docker behavior unchanged. Docker remains a separate future backend.

## Non-Goals

- Building the Lambda guest image or guest agent in this change.
- Emulating AWS's complete Lambda sandbox lifecycle.
- Running image-package Lambdas inside Squib.
- Auto-selecting Squib for `LAMBDA_EXECUTOR=auto`.
- Replacing the native executor for host-runnable `provided.*` bootstraps.

## Squib Findings

The local Squib workspace is versioned as `0.2.0` and exposes an embeddable facade through `Squib::builder()` in `../squib/crates/squib/src/lib.rs`. The builder accepts a static config file, instance id, optional API socket, network mode, and run budget, then starts the runtime through `spawn().await`.

Squib's production VMM loop currently exists for macOS/HVF. Non-macOS builds compile a stub that reports `InstanceStart` as unsupported. This is acceptable for Rustack because the backend can be compiled everywhere while only being operational on supported hosts.

The VMM loop starts a `UdsVsockMuxer` when the Squib config includes vsock. The muxer binds host-initiated ports:

- `5000`: warm-path execute
- `5001`: health
- `5003`: stage-and-invoke

The muxer derives host socket paths as `{uds_path}_{port}` and expects host clients to send a bounded text preamble:

```text
CONNECT <port>\n
```

It replies with:

```text
OK <host_port>\n
```

After that handshake, the Unix stream is bridged to the guest vsock stream.

## Architecture

Rustack adds a `SquibExecutor` beside `NativeExecutor` and `NoopExecutor`.

```text
AWS Lambda Invoke API
        |
        v
RustackLambda::invoke
        |
        v
ExecutorBackend::Squib
        |
        v
SquibExecutor
  - validate Zip + arm64 + code root
  - lazily start Squib microVM
  - connect to {vsock_path}_{stage_port}
  - send staged invocation JSON
  - decode guest response
        |
        v
Squib UDS/vsock muxer
        |
        v
guest Lambda agent
        |
        v
function bootstrap
```

The executor owns one Squib runtime handle. It starts the microVM lazily on the first invocation and keeps it alive until provider shutdown. This keeps the initial change narrow while preserving a path to warm pools later.

## Configuration

`LambdaConfig` gains Squib-specific executor settings parsed from environment variables:

| Environment Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `LAMBDA_EXECUTOR` | yes | `native` | Set to `squib` to enable the backend. |
| `LAMBDA_SQUIB_CONFIG_FILE` | for Squib | none | Static Squib VM configuration file. |
| `LAMBDA_SQUIB_VSOCK_PATH` | for Squib | none | Base path from the Squib config `vsock.uds_path`. |
| `LAMBDA_SQUIB_INSTANCE_ID` | no | `rustack-lambda` | Squib instance id. |
| `LAMBDA_SQUIB_STAGE_PORT` | no | `5003` | Guest agent stage-and-invoke port. |
| `LAMBDA_SQUIB_CONNECT_TIMEOUT_MS` | no | `2000` | Timeout for Squib startup socket connection and handshake. |
| `LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES` | no | `8388608` | Maximum single-line response size from the guest agent. |
| `LAMBDA_SQUIB_RUN_BUDGET_SECS` | no | `86400` | Maximum wall-clock budget for the long-lived Squib microVM. |

The config file and vsock path stay explicit instead of being inferred. Squib's static config owns kernel/initrd/rootfs/device setup; Rustack only needs the values required to start Squib and dial the guest agent.

## Invocation Protocol

The host sends one UTF-8 JSON object followed by `\n` over the bridged stream:

```json
{
  "protocolVersion": "rustack.squib.lambda.v1",
  "requestId": "uuid",
  "functionArn": "arn:aws:lambda:us-east-1:000000000000:function:echo",
  "functionName": "echo",
  "qualifier": "$LATEST",
  "runtime": "provided.al2023",
  "handler": "bootstrap",
  "architectures": ["arm64"],
  "codeRoot": "/tmp/rustack-lambda-code/echo/current",
  "environment": {
    "KEY": "VALUE"
  },
  "timeoutMs": 3000,
  "memoryMb": 128,
  "captureLogs": true,
  "payloadBase64": "e30="
}
```

The guest replies with one JSON object followed by `\n`:

```json
{
  "protocolVersion": "rustack.squib.lambda.v1",
  "status": 200,
  "payloadBase64": "eyJvayI6dHJ1ZX0=",
  "functionError": null,
  "logTailBase64": null,
  "executedVersion": "$LATEST"
}
```

The guest agent is responsible for making the `codeRoot` visible inside the guest, setting Lambda runtime environment variables, running the function bootstrap, applying function timeout, and mapping runtime errors to `functionError`.

## Error Handling

The Squib executor maps failures into existing `ExecutorError` variants:

- Missing Squib config, missing vsock path, unsupported OS, non-Zip package, non-`arm64` function: `Unsupported`.
- Missing code root: `InvalidCode`.
- Squib startup failure or bad muxer handshake: `InitFailed`.
- Guest EOF, malformed response, invalid base64 payload: `RuntimeExited`.
- Invocation deadline exceeded: `Timeout`.
- Unix socket read/write errors: `Io`.

All socket reads are bounded. The response limit defaults to the synchronous Lambda payload budget plus base64 overhead.

## Security And Isolation

This backend improves process isolation relative to the native executor, but it is only as strong as Squib's VMM, the guest image, and the host file sharing model. The initial protocol deliberately passes a host `codeRoot` path; that assumes the guest agent has a controlled staging mechanism. A production guest should avoid broad host mounts and instead copy function code through a bounded staging channel or a narrow shared directory.

The host rejects image packages and non-`arm64` functions before contacting the guest. User environment variables are sent as structured JSON, not shell-expanded strings.

## Testing

The initial implementation should include:

- Backend parser tests for `squib` and `microvm` aliases.
- Config parsing tests for defaults and required Squib fields.
- Unit tests for Squib port socket path derivation.
- Unit tests for request/response JSON and base64 decoding.
- Error tests for missing config and unsupported architecture.

Full end-to-end invocation requires a Squib guest image with a matching `rustack.squib.lambda.v1` agent and should be added once that image is part of the repository or CI environment.

## Implementation Plan

1. Add a workspace dependency on local Squib `0.2.0`.
2. Add `ExecutorBackend::Squib` parsing.
3. Add `SquibExecutorConfig` to Lambda configuration.
4. Add `SquibExecutor` and select it from `build_executor`.
5. Implement lazy Squib startup and bounded vsock stage-and-invoke.
6. Add focused unit tests.
7. Run Rust verification and open a PR.

## Future Work

- Switch dependency to `squib = "0.2.0"` when published.
- Add guest agent and image build automation.
- Add health checks on port `5001` before invoking.
- Add warm execution on port `5000`.
- Add per-function microVM pools and teardown policies.
- Add Docker parity or fallback for non-macOS hosts.
