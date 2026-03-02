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
