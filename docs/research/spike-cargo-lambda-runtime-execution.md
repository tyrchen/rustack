# Spike: How Cargo Lambda build artifacts are executed by the Lambda runtime

Status: Done | Owner: Rustack | Date: 2026-05-28 | Outcome: **PASS-with-caveat**

## Question

When a user runs `cargo lambda build`, what exact artifact is produced, how does AWS Lambda execute it through the Lambda runtime API, and what does Rustack need to support for that artifact to run locally?

## Method

I validated the local artifact shape with Cargo Lambda `1.9.0` on Apple Silicon:

```bash
cargo lambda build --release -p rustack-lambda-echo-bootstrap --arm64 --output-format zip --lambda-dir target/lambda
unzip -l target/lambda/bootstrap/bootstrap.zip
```

I cross-checked the result against the current Cargo Lambda build docs, AWS Lambda Rust zip deployment docs, AWS custom runtime docs, and AWS Runtime API docs on 2026-05-28.

## Findings

1. `cargo lambda build --arm64 --output-format zip` produces a zip file whose root contains an executable named `bootstrap`. With the fixture binary named `bootstrap`, the explicit output was `target/lambda/bootstrap/bootstrap.zip`, and `unzip -l` showed exactly one root-level `bootstrap` entry. The extracted file was executable and identified as `ELF 64-bit LSB pie executable, ARM aarch64, ... for GNU/Linux`.

2. Cargo Lambda's documented default output root is `target/lambda`; each binary gets a subdirectory and the runtime entry artifact is named `bootstrap`. The `--arm64` shortcut compiles for `aarch64-unknown-linux-gnu`, and `--output-format zip` emits an upload-ready zip. Source: Cargo Lambda build docs, 2026-05-28, <https://www.cargo-lambda.info/commands/build.html>.

3. AWS treats Rust Lambda functions as custom/runtime-client functions on an OS-only runtime. The deploy path is a zip package for `provided.al2023` or `provided.al2`, and the package must be compiled for Linux and for the configured architecture, `x86_64` or `arm64`. AWS's Rust packaging docs show uploading `target/lambda/<function>/bootstrap.zip` with `--runtime provided.al2023`. Sources: AWS Rust zip package docs, 2026-05-28, <https://docs.aws.amazon.com/lambda/latest/dg/rust-package.html>; AWS OS-only runtime docs, 2026-05-28, <https://docs.aws.amazon.com/lambda/latest/dg/runtimes-provided.html>.

4. AWS Lambda starts the root-level `bootstrap` as the custom runtime entrypoint. If the package root does not contain an executable `bootstrap`, Lambda reports `Runtime.InvalidEntrypoint`. Source: AWS custom runtime docs, 2026-05-28, <https://docs.aws.amazon.com/lambda/latest/dg/runtimes-custom.html>.

5. The `bootstrap` does not receive the invoke payload on stdin. It is a long-lived runtime process. Lambda sets `AWS_LAMBDA_RUNTIME_API`, then the bootstrap calls `GET /2018-06-01/runtime/invocation/next`, receives the event body plus headers such as `Lambda-Runtime-Aws-Request-Id`, runs the handler, and posts either `POST /2018-06-01/runtime/invocation/{request_id}/response` or `POST /2018-06-01/runtime/invocation/{request_id}/error`. Source: AWS Runtime API docs, 2026-05-28, <https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html>.

6. Rustack already models the same runtime API contract. `runtime_api::start` binds one socket per warm instance on `127.0.0.1:0`; the code documents and routes `/runtime/invocation/next`, `/response`, `/error`, and `/init/error` in [runtime_api.rs](../../crates/rustack-lambda-core/src/executor/runtime_api.rs:1) and [runtime_api.rs](../../crates/rustack-lambda-core/src/executor/runtime_api.rs:124). The `/next` handler queues the payload and response headers, including `Lambda-Runtime-Aws-Request-Id`, `Lambda-Runtime-Deadline-Ms`, `Lambda-Runtime-Invoked-Function-Arn`, and `Lambda-Runtime-Trace-Id`, in [runtime_api.rs](../../crates/rustack-lambda-core/src/executor/runtime_api.rs:214). Completion posts remove the pending request and return `202 Accepted` in [runtime_api.rs](../../crates/rustack-lambda-core/src/executor/runtime_api.rs:256).

7. Rustack's zip ingestion path is compatible with Cargo Lambda zips. `CreateFunction` and `UpdateFunctionCode` decode `ZipFile`, store the raw zip, and extract to `{code_dir}/{function}/{version}/extracted/`; the store preserves Unix file modes so the `bootstrap` executable bit survives extraction. See [provider.rs](../../crates/rustack-lambda-core/src/provider.rs:381), [provider.rs](../../crates/rustack-lambda-core/src/provider.rs:2528), [storage.rs](../../crates/rustack-lambda-core/src/storage.rs:935), and [storage.rs](../../crates/rustack-lambda-core/src/storage.rs:1031).

8. Rustack's native executor starts `code_root/bootstrap` directly, sets `AWS_LAMBDA_RUNTIME_API`, `_HANDLER`, `LAMBDA_TASK_ROOT`, Lambda metadata env vars, and user env vars, then sends invoke jobs through the runtime API server. See [native.rs](../../crates/rustack-lambda-core/src/executor/native.rs:101), [native.rs](../../crates/rustack-lambda-core/src/executor/native.rs:228), and [instance.rs](../../crates/rustack-lambda-core/src/executor/instance.rs:116).

9. Caveat: the validated `cargo lambda build --arm64` artifact is a Linux ARM ELF. Rustack's native executor intentionally rejects ELF on macOS; it only runs ELF on Linux and Mach-O on macOS, after checking declared architecture. See [native.rs](../../crates/rustack-lambda-core/src/executor/native.rs:163). Therefore, a Cargo Lambda zip built for AWS `arm64` will run natively only when Rustack itself is running on compatible Linux ARM64. On a macOS Apple Silicon developer machine, Rustack needs either a container backend, a Squib microVM guest agent that can run Linux arm64 artifacts, or a separate host-built Mach-O test bootstrap for native-only local testing.

## Execution flow

```text
Build time
  cargo lambda build --arm64 --output-format zip
        |
        v
  target/lambda/<bin>/bootstrap.zip
        |
        v
  zip root: ./bootstrap  (Linux arm64 executable)

AWS/Rustack invoke time
  CreateFunction ZipFile
        |
        v
  unzip to LAMBDA_TASK_ROOT / code_root
        |
        v
  start ./bootstrap with AWS_LAMBDA_RUNTIME_API=<host:port>
        |
        v
  bootstrap GET /2018-06-01/runtime/invocation/next
        |
        v
  runtime API returns event body + request-id/deadline/ARN headers
        |
        v
  bootstrap runs handler
        |
        +--> POST /runtime/invocation/{id}/response  -> success payload
        |
        +--> POST /runtime/invocation/{id}/error     -> FunctionError=Unhandled
```

## Decision

**GO-with-amendments**: Rustack's Runtime API and zip extraction model are the right execution contract for Cargo Lambda artifacts. For AWS-shaped artifacts, the supported local path should be:

- Upload `target/lambda/<bin>/bootstrap.zip` as Lambda `ZipFile`.
- Configure runtime `provided.al2023`, handler `bootstrap`, and architecture `arm64` for `cargo lambda build --arm64`.
- Execute with a container backend or Squib microVM on macOS, because the artifact is Linux ELF.
- Execute with native backend only when the extracted `bootstrap` matches the Rustack host OS and architecture.

## Risks identified

1. Docker backend is the missing piece for Apple Silicon developers who want to run the actual Cargo Lambda AWS artifact locally. The current native backend is useful for host-native test bootstraps, not Linux ELF artifacts on macOS.

2. Cargo Lambda defaulted to a target directory that did not appear under this workspace until `--lambda-dir target/lambda` was passed explicitly. Rustack docs/tests should use an explicit `--lambda-dir` in examples to make the artifact path deterministic.

3. The Runtime API implementation currently sets `_X_AMZN_TRACE_ID` only as a response header to the bootstrap. AWS custom runtime docs say runtimes should propagate that header into `_X_AMZN_TRACE_ID` locally before invoking the handler. If a hand-rolled bootstrap skips that step, X-Ray behavior will differ from production, but this is runtime-code responsibility, not Rustack's server responsibility.
