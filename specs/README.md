# Specs

All specs that for AI to generate code.

## Index

| Spec | Type | Status | Description |
|------|------|--------|-------------|
| [rust-rewrite-feasibility](./rust-rewrite-feasibility.md) | Feasibility | Draft | Full codebase analysis of LocalStack with Rust rewrite strategy |
| [ruststack-s3-implementation](./ruststack-s3-implementation.md) | Design | Superseded | S3 implementation using s3s crate (replaced by smithy redesign) |
| [smithy-s3-redesign-design](./smithy-s3-redesign-design.md) | Design | Draft | Smithy-based S3 redesign replacing s3s with self-owned HTTP/XML/auth stack |
| [ruststack-dynamodb-design](./ruststack-dynamodb-design.md) | Design | Draft | Native Rust DynamoDB implementation with Smithy codegen, in-memory storage engine, and expression parser |
| [ruststack-sqs-design](./ruststack-sqs-design.md) | Design | Draft | Native Rust SQS implementation -- 23 operations, dual protocol (awsJson1.0 + awsQuery), actor-per-queue architecture, FIFO support |
| [ruststack-ssm-design](./ruststack-ssm-design.md) | Design | Draft | Native Rust SSM Parameter Store implementation -- 13 operations, Smithy codegen, in-memory versioned parameter storage |
| [ruststack-sns-design](./ruststack-sns-design.md) | Design | Draft | Native Rust SNS implementation -- 42 operations, awsQuery protocol, SNS-to-SQS fan-out, filter policies, FIFO topics |
| [ruststack-secretsmanager-design](./ruststack-secretsmanager-design.md) | Design | Draft | Native Rust Secrets Manager implementation -- 23 operations, awsJson1.1, version staging labels, rotation lifecycle, scheduled deletion |
| [ruststack-logs-design](./ruststack-logs-design.md) | Design | Draft | Native Rust CloudWatch Logs implementation -- ~40 operations, awsJson1.1, append-only log storage, retention policies, filter patterns |
| [ruststack-lambda-design](./ruststack-lambda-design.md) | Design | Draft | Native Rust Lambda (invoke-only) implementation -- 27 operations, restJson1 protocol, Docker-based execution engine, warm container pooling |
| [ruststack-events-design](./ruststack-events-design.md) | Design | Draft | Native Rust EventBridge implementation -- 57 operations, event pattern matching engine, actor-per-bus architecture, SQS target delivery |
| [ruststack-kms-design](./ruststack-kms-design.md) | Design | Draft | Native Rust KMS implementation -- ~35 operations, AES-256-GCM/RSA/ECC/HMAC crypto engine, envelope encryption, key state machine, alias management |
| [ruststack-kinesis-design](./ruststack-kinesis-design.md) | Design | Draft | Native Rust Kinesis Data Streams implementation -- ~40 operations, actor-per-shard architecture, MD5 partition key routing, shard iterators, CBOR+JSON wire formats |
| [s3-checksum-parity-design](./s3-checksum-parity-design.md) | Design | Draft | S3 checksum parity -- CRC64NVME, ChecksumMode gating, aws-chunked trailing headers, multipart checksum combination, upload validation |
