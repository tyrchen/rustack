# Rustack Runtime Snapshot Implementation Plan

**Date:** 2026-05-26
**Status:** Draft
**Type:** Impl Plan
**Depends on:** [ruststack-snapshot-design](./ruststack-snapshot-design.md), [ruststack-snapshot-binary-archive-design](./ruststack-snapshot-binary-archive-design.md)

## Phase 1: Hackathon-App Binary Snapshot Path

Phase 1 ships the complete user-visible snapshot workflow for the services required by `examples/pulumi/hackathon-app`, including S3 and DynamoDB data, using the v2 binary archive layout. Later phases can add service-native row sections, lazy data archive readers, periodic snapshots, migrations, and remaining service-specific data shards.

### Task Table

| # | Task | Spec | Details | Exit Criteria |
|---|------|------|---------|---------------|
| 1 | CLI and snapshot manager | [design § 2](./ruststack-snapshot-design.md#2-user-contract), [archive § 8](./ruststack-snapshot-binary-archive-design.md#8-save-algorithm), [archive § 9](./ruststack-snapshot-binary-archive-design.md#9-load-algorithm) | Keep `--snapshot <name>`, validated snapshot name, default/override root, v2 manifest archive, trait-driven load/save orchestration, and atomic directory replacement. | Unit tests cover CLI parsing, name validation, missing snapshot load, manifest archive load, and atomic save path helpers. |
| 2 | Binary archive envelope | [archive § 5](./ruststack-snapshot-binary-archive-design.md#5-archive-envelope), [archive § 6](./ruststack-snapshot-binary-archive-design.md#6-required-sections) | Implement `*.ss.zst` writer/reader, section directory validation, CRC footer, CBOR payload helpers, and data archive packing/unpacking. | Unit tests cover section round-trip, corrupt magic, corrupt CRC, out-of-bounds sections, and path traversal rejection. |
| 3 | Runtime snapshot registry and parallelism | [archive § 7](./ruststack-snapshot-binary-archive-design.md#7-snapshot-trait), [archive § 8](./ruststack-snapshot-binary-archive-design.md#8-save-algorithm), [archive § 9](./ruststack-snapshot-binary-archive-design.md#9-load-algorithm) | Refactor `SnapshotService` to `save_meta/load_meta`, store services as shareable trait objects, and run service save/load concurrently with a bounded semaphore. | Server starts normally with and without `--snapshot`; health check remains unchanged; save/load loops have no provider-specific branches. |
| 4 | S3 snapshot archive support | [archive § 6.3](./ruststack-snapshot-binary-archive-design.md#63-service-data-archive) | Preserve S3 bucket/object/multipart export/import while packing staged object and part files into `services/s3/data.ss.zst`. | S3 unit test creates bucket/object, exports, imports into fresh provider, reads same body and metadata; final snapshot has no `objects/*.bin` files. |
| 5 | DynamoDB snapshot archive support | [archive § 6.2](./ruststack-snapshot-binary-archive-design.md#62-service-metadata-archive) | Encode table metadata and items as CBOR inside `services/dynamodb/meta.ss.zst`. | DynamoDB unit test creates table/items, exports, imports into fresh provider, gets same item and table description. |
| 6 | Pulumi resource service archive support | [archive § 6.2](./ruststack-snapshot-binary-archive-design.md#62-service-metadata-archive) | Encode SQS, SSM, IAM, Lambda, API Gateway V2, CloudFront, and DynamoDB Streams resource snapshots as CBOR metadata archives. | Targeted provider tests round-trip at least one representative resource per service where practical. |
| 7 | Graceful lifecycle and benchmark instrumentation | [archive § 8](./ruststack-snapshot-binary-archive-design.md#8-save-algorithm), [archive § 10](./ruststack-snapshot-binary-archive-design.md#10-performance-budgets) | Load before bind; save after connection drain; log per-service byte counts/timings; measure SIGINT-to-exit save latency and start-to-health load latency. | Manual `rustack --snapshot local` Ctrl+C creates/overwrites `manifest.ss.zst`; benchmark output reports save/load ms. |
| 8 | Hackathon snapshot smoke and perf target | [verification plan](./ruststack-snapshot-verification-plan.md), [archive § 10](./ruststack-snapshot-binary-archive-design.md#10-performance-budgets) | Extend `make pulumi-hackathon-snapshot-smoke` to assert v2 file shape and enforce hackathon save/load budgets after warm release build. | Target deploys, stops Rustack gracefully, restarts from snapshot, verifies Pulumi refresh/read state, reports save <= 500 ms and load <= 200 ms, and cleans up. |
| 9 | Quality gates, review, PR comment | [verification plan § 5](./ruststack-snapshot-verification-plan.md#5-required-gates), [archive § 11](./ruststack-snapshot-binary-archive-design.md#11-verification) | Run Rust gates, smoke/perf test, independent review against specs, then update PR comments with benchmark results. | All required gates pass or any blocker is explicitly documented; PR has a comment with measured save/load/file-size results. |

### Dependency Order

```text
Task 1 ──► Task 2 ──► Task 3 ──► Task 7 ──► Task 8 ──► Task 9
                         │
                         ├──► Task 4 ─┐
                         ├──► Task 5 ─┼──► Task 7
                         └──► Task 6 ─┘
```

## Phase 2: Broader Service Coverage and Service-Native Sections

Phase 2 adds snapshots for services not required by hackathon-app: SNS, EventBridge, Logs, CloudWatch metrics, KMS, Kinesis, Secrets Manager, SES, and any newly added providers. It also moves high-cardinality services from generic CBOR metadata to service-native row/index sections when benchmarks justify it.

## Phase 3: Operational Hardening

Phase 3 adds optional periodic autosave, explicit export/import commands if needed, schema migration helpers, and performance tests over larger S3/DynamoDB datasets.

## Phase 1 Exit Criteria

- `rustack --snapshot <name>` loads existing snapshots and creates missing snapshots on graceful shutdown.
- Existing non-snapshot `rustack` behavior is unchanged.
- Snapshot directories use v2 shape: `manifest.ss.zst` plus per-service `meta.ss.zst` and optional `data.ss.zst`.
- `make pulumi-hackathon-snapshot-smoke` proves resource save/load with the hackathon-app Pulumi stack.
- S3 object bodies and DynamoDB table items survive restart from snapshot.
- Hackathon snapshot save is <= 500 ms and load-to-health is <= 200 ms on the local reference run after release build warmup.
- No TODOs, no dead-code suppressions, no `unwrap()`/`expect()` in production snapshot code.
- Required Rust verification gates from AGENTS.md pass for Rust-relevant changes.

## Deferred Findings Backlog

If review finds valid out-of-phase issues, append them to `specs/ruststack-snapshot-review.md` with severity, file/line, and fix shape. Do not expand Phase 1 beyond the hackathon-app correctness path unless a finding blocks the exit criteria.
