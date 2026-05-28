# Specs

All specs that for AI to generate code.

## Index

| Spec | Type | Status | Description |
|------|------|--------|-------------|
| [rust-rewrite-feasibility](./rust-rewrite-feasibility.md) | Feasibility | Draft | Full codebase analysis of LocalStack with Rust rewrite strategy |
| [rustack-s3-implementation](./rustack-s3-implementation.md) | Design | Superseded | S3 implementation using s3s crate (replaced by smithy redesign) |
| [smithy-s3-redesign-design](./smithy-s3-redesign-design.md) | Design | Draft | Smithy-based S3 redesign replacing s3s with self-owned HTTP/XML/auth stack |
| [rustack-dynamodb-design](./rustack-dynamodb-design.md) | Design | Draft | Native Rust DynamoDB implementation with Smithy codegen, in-memory storage engine, and expression parser |
| [rustack-sqs-design](./rustack-sqs-design.md) | Design | Draft | Native Rust SQS implementation -- 23 operations, dual protocol (awsJson1.0 + awsQuery), actor-per-queue architecture, FIFO support |
| [rustack-ssm-design](./rustack-ssm-design.md) | Design | Draft | Native Rust SSM Parameter Store implementation -- 13 operations, Smithy codegen, in-memory versioned parameter storage |
| [rustack-sns-design](./rustack-sns-design.md) | Design | Draft | Native Rust SNS implementation -- 42 operations, awsQuery protocol, SNS-to-SQS fan-out, filter policies, FIFO topics |
| [rustack-secretsmanager-design](./rustack-secretsmanager-design.md) | Design | Draft | Native Rust Secrets Manager implementation -- 23 operations, awsJson1.1, version staging labels, rotation lifecycle, scheduled deletion |
| [rustack-logs-design](./rustack-logs-design.md) | Design | Draft | Native Rust CloudWatch Logs implementation -- ~40 operations, awsJson1.1, append-only log storage, retention policies, filter patterns |
| [rustack-lambda-design](./rustack-lambda-design.md) | Design | Draft | Native Rust Lambda (invoke-only) implementation -- 27 operations, restJson1 protocol, Docker-based execution engine, warm container pooling |
| [ruststack-lambda-executor-design](./ruststack-lambda-executor-design.md) | Impl Plan | Draft | Real Lambda Invoke -- in-process Lambda Runtime API server, native (process) + Docker backends with warm pool, idle reaper, async invoke; integration tests with workspace-built Rust echo bootstrap |
| [ruststack-lambda-squib-runtime-design](./ruststack-lambda-squib-runtime-design.md) | Design | Implemented | Squib microVM Lambda executor -- published Squib 0.2.0, default macOS Zip auto backend, AL2023 arm64 guest image builder, bounded vsock stage-and-invoke protocol, cargo-lambda e2e |
| [rustack-events-design](./rustack-events-design.md) | Design | Draft | Native Rust EventBridge implementation -- 57 operations, event pattern matching engine, actor-per-bus architecture, SQS target delivery |
| [rustack-kms-design](./rustack-kms-design.md) | Design | Draft | Native Rust KMS implementation -- ~35 operations, AES-256-GCM/RSA/ECC/HMAC crypto engine, envelope encryption, key state machine, alias management |
| [rustack-kinesis-design](./rustack-kinesis-design.md) | Design | Draft | Native Rust Kinesis Data Streams implementation -- ~40 operations, actor-per-shard architecture, MD5 partition key routing, shard iterators, CBOR+JSON wire formats |
| [rustack-iam-design](./rustack-iam-design.md) | Design | Draft | Native Rust IAM implementation -- ~60 operations, awsQuery protocol, users/roles/groups/policies/instance-profiles/access-keys, global service model, SigV4-based gateway routing |
| [s3-checksum-parity-design](./s3-checksum-parity-design.md) | Design | Draft | S3 checksum parity -- CRC64NVME, ChecksumMode gating, aws-chunked trailing headers, multipart checksum combination, upload validation |
| [rustack-ses-design](./rustack-ses-design.md) | Design | Draft | Native Rust SES implementation -- ~30 v1 operations (awsQuery) + ~10 v2 operations (restJson1), email retrospection endpoint, template rendering, identity management |
| [smithy-codegen-all-services-design](./smithy-codegen-all-services-design.md) | Design | Draft | Universal Smithy codegen -- extend S3-only codegen to all 7+ services with TOML configs, protocol-aware serde, error extraction, overlay system |
| [service-operations-gap-impl-plan](./service-operations-gap-impl-plan.md) | Design | Draft | Operations gap analysis and implementation plan -- 44 Tier 1 ops (DynamoDB transactions/TTL/tagging, Lambda layers/ESM, IAM OIDC), 56 Tier 2 ops, phased delivery across 6 phases |
| [rustack-cloudfront-design](./rustack-cloudfront-design.md) | Design | Draft | Native Rust CloudFront management-plane implementation -- ~90 ops, restXml protocol, ETag/IfMatch concurrency, Distribution/Invalidation state machines, OAC/OAI, Cache/OriginRequest/ResponseHeaders policies, KeyGroups/PublicKeys, CloudFront Functions (stored, not executed) |
| [rustack-cloudfront-dataplane-design](./rustack-cloudfront-dataplane-design.md) | Design | Draft | axum-based CloudFront data plane -- pass-through reverse proxy for end-to-end IaC testing, in-process S3 origin dispatch, cache-behavior path matching, DefaultRootObject/OriginPath/custom headers, no caching, no Lambda@Edge/Functions execution, phased D0 (S3) / D1 (HTTP + policies + host routing) / D2 (APIGW + Lambda URL) |
| [ruststack-pulumi-target-design](./ruststack-pulumi-target-design.md) | Design | Implemented | Pulumi AWS provider endpoint override target for Rustack, with TypeScript smoke stack and `make pulumi-smoke` validation |
| [ruststack-pulumi-hackathon-app](./ruststack-pulumi-hackathon-app.md) | Validation Spec | Implemented | Real-world serverless hackathon app Pulumi stack covering CloudFront, S3, API Gateway, Lambda, DynamoDB, SQS, SSM, IAM policies, OAC, bucket policies, and worker event source mapping |
| [ruststack-snapshot-prd](./ruststack-snapshot-prd.md) | PRD | Draft | Named runtime snapshots for loading/saving Rustack resource and data-plane state across graceful restarts |
| [ruststack-snapshot-design](./ruststack-snapshot-design.md) | Design | Draft | Snapshot layout, resource/data shard boundaries, runtime lifecycle, atomic save/load behavior, and service contracts |
| [ruststack-snapshot-binary-archive-design](./ruststack-snapshot-binary-archive-design.md) | Design | Draft | Binary `*.ss.zst` snapshot archive format with per-service `meta.ss.zst`/`data.ss.zst`, parallel save/load, and hackathon perf budgets |
| [ruststack-snapshot-impl-plan](./ruststack-snapshot-impl-plan.md) | Impl Plan | Draft | Phase plan for hackathon-app snapshot support, service shard implementation, smoke target, and quality gates |
| [ruststack-snapshot-verification-plan](./ruststack-snapshot-verification-plan.md) | Verification Plan | Draft | Unit, integration, Pulumi, performance, and Rust quality gates for snapshot support |
