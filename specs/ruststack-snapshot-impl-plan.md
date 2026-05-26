# Rustack Runtime Snapshot Implementation Plan

**Date:** 2026-05-26
**Status:** Draft
**Type:** Impl Plan
**Depends on:** [ruststack-snapshot-design](./ruststack-snapshot-design.md)

## Phase 1: Hackathon-App Snapshot Path

Phase 1 ships the complete user-visible snapshot workflow for the services required by `examples/pulumi/hackathon-app`, including S3 and DynamoDB data. Later phases can add periodic snapshots, migrations, and remaining service-specific data shards.

### Task Table

| # | Task | Spec | Details | Exit Criteria |
|---|------|------|---------|---------------|
| 1 | CLI and snapshot manager | [design § 2](./ruststack-snapshot-design.md#2-user-contract), [design § 7](./ruststack-snapshot-design.md#7-atomic-save) | Add `--snapshot <name>`, validated snapshot name, default/override root, manifest structs, trait-driven load/save orchestration, and atomic directory replacement. | Unit tests cover CLI parsing, name validation, missing snapshot load, and atomic save path helpers. |
| 2 | Runtime snapshot registry | [design § 5](./ruststack-snapshot-design.md#5-runtime-lifecycle) | Refactor `build_services` into runtime construction that registers `SnapshotService` wrappers for provider export/import while still returning routers. | Server starts normally with and without `--snapshot`; health check remains unchanged; adding a service does not require editing the main save/load loops. |
| 3 | S3 snapshot support | [design § 6](./ruststack-snapshot-design.md#6-service-contracts) | Export/import bucket metadata, bucket config fields, object metadata, object bodies, and multipart part bodies. | S3 unit test creates bucket/object, exports, imports into fresh provider, reads same body and metadata. |
| 4 | DynamoDB snapshot support | [design § 6](./ruststack-snapshot-design.md#6-service-contracts) | Export/import table metadata, tags, TTL, PITR, and all table items. | DynamoDB unit test creates table/items, exports, imports into fresh provider, gets same item and table description. |
| 5 | Pulumi resource service support | [design § 6](./ruststack-snapshot-design.md#6-service-contracts) | Add resource snapshot support for SQS, SSM, IAM, Lambda, API Gateway V2, and CloudFront sufficient for hackathon-app refresh/read paths. | Targeted provider tests round-trip at least one representative resource per service where practical. |
| 6 | Graceful save/load lifecycle | [design § 5](./ruststack-snapshot-design.md#5-runtime-lifecycle), [design § 10](./ruststack-snapshot-design.md#10-observability) | Load before bind; save after connection drain and Lambda executor shutdown; log service counts without sensitive payloads. | Manual `rustack --snapshot local` Ctrl+C creates/overwrites snapshot. |
| 7 | Hackathon snapshot smoke target | [verification plan](./ruststack-snapshot-verification-plan.md) | Add `make pulumi-hackathon-snapshot-smoke` using the existing Pulumi project. | Target deploys, stops Rustack gracefully, restarts from snapshot, verifies Pulumi refresh/read state, and cleans up. |
| 8 | Quality gates and review | [verification plan](./ruststack-snapshot-verification-plan.md#5-required-gates) | Run Rust gates, smoke test, and independent review against specs. | All required gates pass or any blocker is explicitly documented. |

### Dependency Order

```text
Task 1 ──► Task 2 ──► Task 6 ──► Task 7 ──► Task 8
             │
             ├──► Task 3 ─┐
             ├──► Task 4 ─┼──► Task 6
             └──► Task 5 ─┘
```

## Phase 2: Broader Service Coverage

Phase 2 adds snapshots for services not required by hackathon-app: SNS, EventBridge, Logs, CloudWatch metrics, KMS, Kinesis, Secrets Manager, SES, and any newly added providers. Each service follows the same `SnapshotService` wrapper contract.

## Phase 3: Operational Hardening

Phase 3 adds optional periodic autosave, explicit export/import commands if needed, schema migration helpers, and performance tests over larger S3/DynamoDB datasets.

## Phase 1 Exit Criteria

- `rustack --snapshot <name>` loads existing snapshots and creates missing snapshots on graceful shutdown.
- Existing non-snapshot `rustack` behavior is unchanged.
- `make pulumi-hackathon-snapshot-smoke` proves resource save/load with the hackathon-app Pulumi stack.
- S3 object bodies and DynamoDB table items survive restart from snapshot.
- No TODOs, no dead-code suppressions, no `unwrap()`/`expect()` in production snapshot code.
- Required Rust verification gates from AGENTS.md pass for Rust-relevant changes.

## Deferred Findings Backlog

If review finds valid out-of-phase issues, append them to `specs/ruststack-snapshot-review.md` with severity, file/line, and fix shape. Do not expand Phase 1 beyond the hackathon-app correctness path unless a finding blocks the exit criteria.
