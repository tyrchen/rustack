# Rustack Lambda Squib Runtime Design

## Status

Implemented behind PR #27. Rustack depends on the published `squib = "0.2.0"` crate from crates.io.

## Problem

Rustack Lambda can already run `provided.*` bootstraps directly on the host through the native executor. That path is useful for fast local feedback, but it is not an isolation boundary and cannot execute Linux `arm64` Lambda artifacts on macOS without a compatibility layer.

Squib provides a Firecracker-shaped microVM runtime for Apple Silicon. Its embeddable API can start a microVM from a static configuration file, and its vsock muxer exposes host Unix sockets for guest services. Rustack should add a Squib executor backend that starts the microVM once, stages a Lambda invocation over vsock, and returns a normal `InvokeResponse`.

## Goals

- Add `LAMBDA_EXECUTOR=squib` as an explicit Lambda executor backend.
- Use the published Squib `0.2.0` crate.
- Default `LAMBDA_EXECUTOR=auto` to Squib for macOS Zip Lambdas, while keeping image packages off Squib.
- Require `arm64` Lambda functions for this backend.
- Keep Rustack's existing Lambda CRUD, storage, and API response behavior unchanged.
- Define a small, versionable host-to-guest invocation protocol over Squib's vsock muxer.
- Fail with actionable errors when the Squib config, vsock socket, function package, or guest agent is missing.
- Keep Docker behavior unchanged. Docker remains a separate future backend.

## Non-Goals

- Emulating AWS's complete Lambda sandbox lifecycle.
- Running image-package Lambdas inside Squib.
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
  - validate Zip + arm64 + code zip
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
| `LAMBDA_EXECUTOR` | no | `auto` | `auto` uses Squib for macOS Zip functions and native execution otherwise. Set to `squib` to force Squib. |
| `LAMBDA_SQUIB_CONFIG_FILE` | no | `target/rustack-lambda-squib/config.json` | Static Squib VM configuration file. Build the default image with `make lambda-squib-image`. |
| `LAMBDA_SQUIB_VSOCK_PATH` | no | `target/rustack-lambda-squib/vsock.sock` | Base path from the Squib config `vsock.uds_path`. |
| `LAMBDA_SQUIB_INSTANCE_ID` | no | `rustack_lambda` | Squib instance id. |
| `LAMBDA_SQUIB_STAGE_PORT` | no | `5003` | Guest agent stage-and-invoke port. |
| `LAMBDA_SQUIB_CONNECT_TIMEOUT_MS` | no | `15000` | Timeout for Squib startup socket connection and handshake. |
| `LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES` | no | `8388608` | Maximum single-line response size from the guest agent. |
| `LAMBDA_SQUIB_RUN_BUDGET_SECS` | no | `86400` | Maximum wall-clock budget for the long-lived Squib microVM. |
| `LAMBDA_SQUIB_SHUTDOWN_TIMEOUT_MS` | no | `10000` | Maximum time to wait for guest poweroff plus Squib shutdown. |

The default VM image is generated under `target/rustack-lambda-squib`. The image target downloads the Firecracker arm64 kernel, resolves the latest Amazon Linux 2023 minimal arm64 container rootfs, verifies its SHA256SUMS, injects a static Rustack guest agent, and packages an initramfs. A small Alpine busybox helper is also injected only to bring loopback up and power the guest off from `/init`; Lambda user code runs against the AL2023 userland so normal `cargo lambda build --arm64 --output-format zip` artifacts have the expected glibc loader and libraries.

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
  "codeZipBase64": "UEsDBAoAAAA...",
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

The host sends the original stored Zip bytes over vsock. `codeRoot` is optional metadata only. The guest agent validates the protocol, decodes the Zip into `/tmp/rustack-lambda/{request_id}/code`, rejects unsafe archive paths through `zip::read::ZipFile::enclosed_name`, preserves Unix permissions, and runs the archive-root `bootstrap`.

The same stage port also accepts a bounded control message used during Rustack shutdown:

```json
{
  "protocolVersion": "rustack.squib.lambda.v1",
  "kind": "shutdown"
}
```

The guest responds and then powers off. This is required because Squib 0.2.0 can request VMM shutdown, but the HVF vCPU driver does not observe the shutdown flag directly; a guest poweroff gives the VMM thread a real exit path.

The guest agent sets Lambda runtime environment variables, serves the Lambda Runtime API on guest loopback, applies the function timeout, captures bounded bootstrap stdout/stderr for agent-side errors, and maps runtime errors to `functionError`.

## Error Handling

The Squib executor maps failures into existing `ExecutorError` variants:

- Missing Squib config, missing vsock path, unsupported OS, non-Zip package, non-`arm64` function: `Unsupported`.
- Missing code zip: `InvalidCode`.
- Squib startup failure or bad muxer handshake: `InitFailed`.
- Guest EOF, malformed response, invalid base64 payload: `RuntimeExited`.
- Invocation deadline exceeded: `Timeout`.
- Unix socket read/write errors: `Io`.

All socket reads are bounded. The response limit defaults to the synchronous Lambda payload budget plus base64 overhead.

## Security And Isolation

This backend improves process isolation relative to the native executor, but it is only as strong as Squib's VMM and the guest image. The protocol copies function code bytes through a bounded vsock staging channel rather than mounting arbitrary host directories into the guest.

The host rejects image packages and non-`arm64` functions before contacting the guest. User environment variables are sent as structured JSON, not shell-expanded strings.

## Testing

The initial implementation should include:

- Backend parser tests for `squib` and `microvm` aliases.
- Config parsing tests for defaults and required Squib fields.
- Unit tests for Squib port socket path derivation.
- Unit tests for request/response JSON and base64 decoding.
- Error tests for missing config and unsupported architecture.

End-to-end coverage is provided by `make test-lambda-invoke-squib`. The target builds the default guest image, codesigns the integration test binary with the macOS hypervisor entitlement, builds the demo `rustack-lambda-echo-bootstrap` app with `cargo lambda --arm64 --output-format zip`, uploads the resulting Zip through Rustack's normal `CreateFunction` path, invokes it through `ExecutorBackend::Auto`, and asserts the Squib/AL2023 guest returns the echoed payload.

## Implementation Plan

1. Add a workspace dependency on published Squib `0.2.0`.
2. Add `ExecutorBackend::Squib` parsing.
3. Add `SquibExecutorConfig` to Lambda configuration.
4. Add `SquibExecutor` and select it explicitly or through the macOS Zip auto rule.
5. Implement lazy Squib startup and bounded vsock stage-and-invoke.
6. Add focused unit tests.
7. Add `tools/lambda-squib-agent`, default AL2023 initramfs assembly, gated cargo-lambda/Squib e2e coverage, Rust verification, and PR updates.

## Future Work

- Add health checks on port `5001` before invoking.
- Add warm execution on port `5000`.
- Add per-function microVM pools and teardown policies.
- Add Docker parity or fallback for non-macOS hosts.
