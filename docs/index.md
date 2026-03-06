# Documentation

## Research

- [LocalStack S3 Implementation Research](./research/localstack-s3-research.md) - Comprehensive analysis of what a Rust-based LocalStack S3 service needs to support for feature parity.
- [s3s Crate Ecosystem Research](./research/s3s-crate-research.md) - Comprehensive analysis of the s3s Rust crate for building S3-compatible services.
- [LocalStack Container & CI Research](./research/localstack-container-ci-research.md) - How LocalStack's Docker container works, gateway routing through port 4566, health endpoints, and the official GitHub Action for CI.
- [S3 Integration Test Suites Research](./research/s3-integration-test-suites-research.md) - Survey of available S3 compliance/integration test suites (Ceph s3-tests, MinIO Mint, MSST-S3, s3s-e2e) and recommendations for adoption.
- [smithy-rs Server Codegen Research](./research/smithy-rs-server-codegen-research.md) - Evaluation of using AWS smithy-rs directly to build an S3-compatible server via Smithy code generation, compared with s3s.
- [DynamoDB API Research](./research/dynamodb-api-research.md) - Comprehensive analysis of the AWS DynamoDB API surface, protocol, data model, Smithy model, and what a Rust-based local DynamoDB implementation would need.
- [DynamoDB Integration Test Suites Research](./research/dynamodb-integration-test-suites-research.md) - Survey of available DynamoDB compatibility/conformance test suites and recommendations for adoption, analogous to MinIO Mint for S3.
- [SSM Parameter Store Research](./research/ssm-parameter-store-research.md) - Comprehensive analysis of the AWS SSM Parameter Store API surface, protocol, data model, Smithy model, test suites, and implementation scope for RustStack.
- [SQS API Research](./research/sqs-api-research.md) - Comprehensive analysis of the AWS SQS API surface, protocol (awsJson1_0 with awsQuery compatibility), Smithy model, features (FIFO, DLQ, long polling, visibility timeout), test suites, and implementation challenges for RustStack.
- [SQS Integration Test Suites Research](./research/sqs-test-suites.md) - Survey of available SQS compatibility/conformance test suites (ElasticMQ, LocalStack, Moto, GoAWS, AWS SDK tests) and recommendations for adoption, analogous to MinIO Mint for S3.
