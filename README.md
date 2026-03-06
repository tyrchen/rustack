# RustStack

A high-performance, LocalStack-compatible AWS service emulator written in Rust.

Currently implements **S3** (70 operations), **DynamoDB** (12 core operations), and **SQS** (20 operations with FIFO support), providing a unified gateway on a single port.

## Features

- **Full S3 protocol** — 70 operations covering buckets, objects, multipart uploads, versioning, and bucket configuration
- **DynamoDB support** — 12 core operations: `CreateTable`, `DeleteTable`, `DescribeTable`, `ListTables`, `UpdateTable`, `PutItem`, `GetItem`, `DeleteItem`, `UpdateItem`, `Query`, `Scan`, `BatchWriteItem`
- **SQS support** — 20 operations: standard and FIFO queues, message batching, dead-letter queues, long polling, visibility timeouts, tags, and content-based deduplication
- **Unified gateway** — Single port (4566) routes S3, DynamoDB, and SQS via headers; extensible `ServiceRouter` trait for adding new services
- **Selective services** — Enable only the services you need at compile time (Cargo features) or runtime (`SERVICES` env var)
- **AWS SDK compatible** — Drop-in replacement for LocalStack; works with any AWS SDK or CLI
- **SigV4 authentication** — Optional AWS Signature Version 4 request verification
- **Virtual-hosted & path-style** addressing for S3 bucket routing
- **In-memory storage** with automatic disk spillover for large S3 objects (configurable threshold)
- **DynamoDB expressions** — Condition, filter, projection, and update expressions with full parser
- **Smithy-driven codegen** — S3 types auto-generated from the official AWS Smithy model
- **Tiny Docker image** — Static musl binary in a scratch container (~8 MB)
- **Graceful shutdown** and health check endpoints for container orchestration

## Why RustStack?

If you need S3, DynamoDB, and SQS for local development or CI, RustStack gives you a **faster, leaner** alternative to LocalStack:

| | RustStack | LocalStack |
|---|---|---|
| **Language** | Rust (static binary) | Python |
| **Docker image** | ~8 MB (scratch) | ~475 MB compressed / ~1.88 GB on disk |
| **Startup time** | < 1 second | 10-45 seconds (S3 only); up to 2 min (all services) |
| **Memory (idle)** | ~10 MB | ~750 MB minimum |
| **S3 operations** | 70 | ~92 |
| **DynamoDB operations** | 12 | ~30+ |
| **SQS operations** | 20 | ~23 |
| **CI cold start** | Pull + ready in ~3s | Pull + ready in 30-90s |
| **Auth support** | SigV4 + SigV2 + presigned URLs | SigV4 (Pro for IAM enforcement) |
| **License** | MIT, fully open source | Shifting to registration-required; free tier limited |
| **Multi-service** | S3 + DynamoDB + SQS | 80+ AWS services |

**When to use RustStack:** You need fast, reliable S3, DynamoDB, and/or SQS in CI pipelines or local dev, and don't want to wait 30+ seconds for a 2 GB container to boot. Your tests start in seconds, not minutes.

**When to use LocalStack:** You need services beyond S3, DynamoDB, and SQS (Lambda, SNS, etc.) and are willing to trade startup time and resource usage for broader AWS coverage.

## Quick Start

```bash
# Build and run
cargo run -p ruststack-server

# Or use the Makefile
make run
```

The server listens on `0.0.0.0:4566` by default. Point any AWS SDK or CLI at `http://localhost:4566`:

```bash
aws s3 --endpoint-url http://localhost:4566 mb s3://my-bucket
aws s3 --endpoint-url http://localhost:4566 cp file.txt s3://my-bucket/
aws s3 --endpoint-url http://localhost:4566 ls s3://my-bucket/
```

### Docker

```bash
docker build -t ruststack .
docker run -p 4566:4566 ruststack

# Run with only specific services
docker run -p 4566:4566 -e SERVICES=dynamodb ruststack
docker run -p 4566:4566 -e SERVICES=sqs ruststack
```

Multi-arch images (amd64/arm64) are published to `ghcr.io/tyrchen/ruststack` on tagged releases.

## GitHub Action

Use RustStack as a drop-in S3 + DynamoDB + SQS service in your CI pipelines:

```yaml
steps:
  - uses: actions/checkout@v4
  - uses: tyrchen/ruststack@v0
    id: ruststack
```

That's it. The action starts the server, waits for it to be healthy, and exports `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, and `AWS_SECRET_ACCESS_KEY` into the environment. All subsequent `aws` CLI and AWS SDK calls will automatically use RustStack.

### Usage Example

```yaml
name: test
on: [push]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: tyrchen/ruststack@v0

      - name: Run S3 tests
        run: |
          aws s3api create-bucket --bucket my-test-bucket
          aws s3api put-object --bucket my-test-bucket --key hello.txt --body README.md
          aws s3api get-object --bucket my-test-bucket --key hello.txt /tmp/out.txt
```

### Action Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `image-tag` | `latest` | Docker image tag (`latest`, `0.1.0`, etc.) |
| `port` | `4566` | Host port to bind the service to |
| `default-region` | `us-east-1` | Default AWS region |
| `log-level` | `info` | Log level (`error`, `warn`, `info`, `debug`, `trace`) |
| `wait-timeout` | `30` | Seconds to wait for the service to become healthy |
| `services` | *(empty = all)* | Comma-separated list of services to enable (`s3`, `dynamodb`, `sqs`) |

### Action Outputs

| Output | Description |
|--------|-------------|
| `endpoint` | The service endpoint URL (e.g. `http://localhost:4566`) |
| `container-id` | Docker container ID for advanced usage |

### Environment Variables Set by the Action

The action automatically exports these into `$GITHUB_ENV`, so all subsequent steps can use `aws` CLI and AWS SDKs without extra configuration:

| Variable | Value |
|----------|-------|
| `AWS_ENDPOINT_URL` | `http://localhost:<port>` |
| `AWS_ACCESS_KEY_ID` | `test` |
| `AWS_SECRET_ACCESS_KEY` | `test` |
| `AWS_DEFAULT_REGION` | Value of `default-region` input |

### What You Can Test

See the end-to-end test workflows for comprehensive examples:

**S3** ([s3-test.yml](.github/workflows/s3-test.yml)): Bucket CRUD, object CRUD, copy, batch delete, presigned URLs, versioning, tagging, CORS, lifecycle, encryption, object lock, multipart uploads, listing with pagination, POST object, error handling.

**DynamoDB** ([dynamodb-test.yml](.github/workflows/dynamodb-test.yml)): Table CRUD, item CRUD with projections and return values, condition expressions, query with key conditions/filters/pagination, scan, batch operations, update expressions (SET, REMOVE, ADD, DELETE), all data types, error handling.

**SQS** ([sqs-test.yml](.github/workflows/sqs-test.yml)): Queue CRUD, send/receive/delete messages, message attributes, batch operations, FIFO queues with ordering and deduplication, tags, purge, error handling.

## Configuration

All settings are controlled via environment variables, matching LocalStack conventions:

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address and port |
| `SERVICES` | *(empty = all)* | Comma-separated list of services to enable (`s3`, `dynamodb`, `sqs`) |
| `S3_SKIP_SIGNATURE_VALIDATION` | `true` | Skip S3 SigV4 request verification |
| `DYNAMODB_SKIP_SIGNATURE_VALIDATION` | `true` | Skip DynamoDB SigV4 request verification |
| `SQS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SQS SigV4 request verification |
| `S3_VIRTUAL_HOSTING` | `true` | Enable virtual-hosted-style addressing |
| `S3_DOMAIN` | `s3.localhost.localstack.cloud` | Virtual hosting domain |
| `S3_MAX_MEMORY_OBJECT_SIZE` | `524288` | Max S3 object size (bytes) kept in memory before disk spillover |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `LOG_LEVEL` | `info` | Log level (`error`, `warn`, `info`, `debug`, `trace`) |
| `RUST_LOG` | | Fine-grained tracing filter (overrides `LOG_LEVEL`) |

### Selective Service Enablement

RustStack supports running a subset of services via two mechanisms:

**Runtime selection** — Set the `SERVICES` environment variable to a comma-separated list:

```bash
# Only DynamoDB
SERVICES=dynamodb cargo run -p ruststack-server

# Only S3
SERVICES=s3 cargo run -p ruststack-server

# S3 + SQS
SERVICES=s3,sqs cargo run -p ruststack-server

# All services (default when SERVICES is empty or unset)
cargo run -p ruststack-server
```

**Compile-time selection** — Use Cargo feature flags to exclude services from the binary entirely, reducing binary size:

```bash
# S3-only binary
cargo build -p ruststack-server --no-default-features --features s3

# DynamoDB-only binary
cargo build -p ruststack-server --no-default-features --features dynamodb

# SQS-only binary
cargo build -p ruststack-server --no-default-features --features sqs
```

The health check endpoint dynamically reports only the services that are running:

```json
{"services":{"s3":"running","dynamodb":"running","sqs":"running"}}
```

## Supported Operations

<details>
<summary><b>Bucket operations (43)</b></summary>

| Category | Operations |
|----------|-----------|
| CRUD | CreateBucket, DeleteBucket, HeadBucket, ListBuckets, GetBucketLocation |
| Versioning | GetBucketVersioning, PutBucketVersioning |
| Encryption | GetBucketEncryption, PutBucketEncryption, DeleteBucketEncryption |
| CORS | GetBucketCors, PutBucketCors, DeleteBucketCors |
| Lifecycle | GetBucketLifecycleConfiguration, PutBucketLifecycleConfiguration, DeleteBucketLifecycle |
| Policy | GetBucketPolicy, PutBucketPolicy, DeleteBucketPolicy, GetBucketPolicyStatus |
| Tagging | GetBucketTagging, PutBucketTagging, DeleteBucketTagging |
| Notifications | GetBucketNotificationConfiguration, PutBucketNotificationConfiguration |
| Logging | GetBucketLogging, PutBucketLogging |
| Public Access | GetPublicAccessBlock, PutPublicAccessBlock, DeletePublicAccessBlock |
| Ownership | GetBucketOwnershipControls, PutBucketOwnershipControls, DeleteBucketOwnershipControls |
| Object Lock | GetObjectLockConfiguration, PutObjectLockConfiguration |
| Accelerate | GetBucketAccelerateConfiguration, PutBucketAccelerateConfiguration |
| Payment | GetBucketRequestPayment, PutBucketRequestPayment |
| Website | GetBucketWebsite, PutBucketWebsite, DeleteBucketWebsite |
| ACL | GetBucketAcl, PutBucketAcl |

</details>

<details>
<summary><b>Object operations (18)</b></summary>

| Category | Operations |
|----------|-----------|
| CRUD | PutObject, GetObject, HeadObject, DeleteObject, DeleteObjects, CopyObject |
| Tagging | GetObjectTagging, PutObjectTagging, DeleteObjectTagging |
| ACL | GetObjectAcl, PutObjectAcl |
| Retention | GetObjectRetention, PutObjectRetention |
| Legal Hold | GetObjectLegalHold, PutObjectLegalHold |
| Attributes | GetObjectAttributes |

</details>

<details>
<summary><b>Multipart upload operations (7)</b></summary>

CreateMultipartUpload, UploadPart, UploadPartCopy, CompleteMultipartUpload, AbortMultipartUpload, ListParts, ListMultipartUploads

</details>

<details>
<summary><b>List operations (3)</b></summary>

ListObjects, ListObjectsV2, ListObjectVersions

</details>

## Supported DynamoDB Operations

| Category | Operations |
|----------|-----------|
| Table management | CreateTable, DeleteTable, DescribeTable, ListTables, UpdateTable |
| Item CRUD | PutItem, GetItem, DeleteItem, UpdateItem |
| Query & scan | Query, Scan |
| Batch | BatchWriteItem |

DynamoDB features include: condition expressions, filter expressions, projection expressions, update expressions (SET, REMOVE, ADD, DELETE), key conditions with sort key operators (=, <, <=, >, >=, BETWEEN, begins_with), and consistent/eventually-consistent reads.

## Supported SQS Operations

| Category | Operations |
|----------|-----------|
| Queue management | CreateQueue, DeleteQueue, GetQueueUrl, ListQueues, GetQueueAttributes, SetQueueAttributes |
| Messages | SendMessage, ReceiveMessage, DeleteMessage, ChangeMessageVisibility, PurgeQueue |
| Batch | SendMessageBatch, DeleteMessageBatch, ChangeMessageVisibilityBatch |
| Tags | TagQueue, UntagQueue, ListQueueTags |
| Permissions | AddPermission, RemovePermission |
| Dead-letter queues | ListDeadLetterSourceQueues |

SQS features include: standard and FIFO queues, content-based and explicit message deduplication, message groups with ordering guarantees, dead-letter queue redrive policies, long polling, per-message and per-queue visibility timeouts, message delay, message attributes, and queue tagging.

## Architecture

```
ruststack-core              — Shared types, config, multi-account/region state
ruststack-auth              — AWS SigV4/SigV2 authentication

ruststack-s3-model          — S3 types auto-generated from AWS Smithy model (codegen/)
ruststack-s3-xml            — XML serialization/deserialization (quick-xml)
ruststack-s3-http           — S3 HTTP routing, request/response conversion, hyper service
ruststack-s3-core           — S3 business logic, in-memory state, storage engine

ruststack-dynamodb-model    — DynamoDB types (operations, errors, I/O structs)
ruststack-dynamodb-http     — DynamoDB HTTP dispatch, awsJson1.0 protocol
ruststack-dynamodb-core     — DynamoDB business logic, B-tree storage, expression engine

ruststack-sqs-model         — SQS types (operations, errors, I/O structs)
ruststack-sqs-http          — SQS HTTP dispatch, awsJson1.0 protocol
ruststack-sqs-core          — SQS business logic, actor-per-queue model, FIFO engine

ruststack-server            — Unified server binary with gateway routing
```

### Request Pipeline

```
HTTP Request
  → Gateway health check interception
  → ServiceRouter dispatch (first match wins)
  ├─ DynamoDBServiceRouter (X-Amz-Target: DynamoDB_*)
  │   → Body collection → JSON deserialization
  │   → SigV4 authentication (optional)
  │   → Operation dispatch (DynamoDBHandler trait)
  │   → Business logic (RustStackDynamoDB provider)
  │   → JSON response serialization
  ├─ SQSServiceRouter (X-Amz-Target: AmazonSQS.*)
  │   → Body collection → JSON deserialization
  │   → SigV4 authentication (optional)
  │   → Operation dispatch (SqsHandler trait)
  │   → Business logic (RustStackSqs provider, actor-per-queue)
  │   → JSON response serialization
  └─ S3ServiceRouter (catch-all)
      → S3Router (path-style / virtual-hosted-style)
      → Body collection
      → SigV4 authentication (optional)
      → Operation dispatch (S3Handler trait)
      → Business logic (RustStackS3 provider)
      → XML/JSON response serialization
  → HTTP Response
```

Adding a new AWS service requires implementing the `ServiceRouter` trait and registering it in `main.rs` — no changes to `gateway.rs`.

## Development

**Prerequisites:** Rust 1.93+ (pinned in `rust-toolchain.toml`)

```bash
make build      # Compile all crates
make check      # cargo check --all-targets --all-features
make test       # Run unit tests (cargo nextest)
make fmt        # Format with cargo +nightly fmt
make clippy     # Lint with -D warnings
make audit      # Security vulnerability check
make deny       # License policy enforcement
make codegen    # Regenerate S3 model types from Smithy
```

### Integration Tests

Integration tests use the official AWS SDK for Rust against a running server:

```bash
# Terminal 1: start the server
make run

# Terminal 2: run integration tests
cargo test -p ruststack-integration -- --ignored
```

Tests cover S3 (buckets, objects, multipart uploads, versioning, CORS), DynamoDB (tables, items, queries, expressions), and SQS (queues, messages, FIFO, batching, DLQ).

### CI/CD

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `build.yml` | Push / PR | Format, lint, test, coverage |
| `integration.yml` | Push / PR | AWS SDK integration tests, MinIO Mint (S3), Alternator (DynamoDB), boto3 (SQS) compatibility |
| `s3-test.yml` | Push / PR | End-to-end S3 tests via the GitHub Action + AWS CLI |
| `dynamodb-test.yml` | Push / PR | End-to-end DynamoDB tests via the GitHub Action + AWS CLI |
| `sqs-test.yml` | Push / PR | End-to-end SQS tests via the GitHub Action + AWS CLI |
| `nightly.yml` | Daily 06:00 UTC | Ceph s3-tests compatibility suite |
| `release-docker.yml` | Version tags / manual | Multi-arch Docker image to GHCR |

## License

This project is distributed under the terms of MIT.

See [LICENSE](LICENSE.md) for details.

Copyright 2025 Tyr Chen
