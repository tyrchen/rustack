# Rustack Runtime Snapshot PRD

**Date:** 2026-05-26
**Status:** Draft
**Type:** PRD
**Scope:** Add named runtime snapshots so a Rustack process can persist and reload local AWS-compatible resource state and data-plane contents across normal exits.

## 1. Problem

Rustack currently starts with empty in-memory state every time. A Pulumi stack can create resources successfully, and application traffic can write S3 objects or DynamoDB items, but Ctrl+C discards everything. This breaks the local-development loop: users cannot stop Rustack overnight, restart it with the same stack, and continue using the created resources and data.

The concrete failure mode is visible with `examples/pulumi/hackathon-app`: Pulumi can provision CloudFront, S3, API Gateway V2, Lambda, DynamoDB, SQS, SSM, IAM, and related resources, but restarting Rustack loses both the resource graph and the S3/DynamoDB contents the app depends on.

## 2. Vision

`rustack --snapshot <name>` starts Rustack from a named durable snapshot if it exists. During normal shutdown, including Ctrl+C, Rustack saves the current runtime state back to that same snapshot name. The user-facing workflow is intentionally simple:

```bash
rustack --snapshot hackathon
# run Pulumi/app traffic
# Ctrl+C

rustack --snapshot hackathon
# resources and data are available again
```

The snapshot is a directory on disk with separate resource and data sections. The CLI exposes one snapshot name, while the implementation can store control-plane resources and data-plane payloads as independent shards for performance and future partial restore.

## 3. Goals

| # | Goal | Measure |
|---|------|---------|
| G1 | Named load/save UX | `rustack --snapshot <name>` loads an existing snapshot and saves on graceful shutdown. |
| G2 | Pulumi resource persistence | A Pulumi stack created against Rustack can refresh/read resources after Rustack restarts from the same snapshot. |
| G3 | Data-plane persistence | S3 object bodies and DynamoDB table items written before shutdown are present after restart. |
| G4 | Fast load | The hackathon-app snapshot loads before the health endpoint becomes ready in <= 200 ms on the local reference machine. |
| G5 | Durable save semantics | Save is atomic: an interrupted save never corrupts the previous snapshot. |
| G6 | Extensible service contract | Each service owns its own snapshot encode/decode boundary and can opt into resources, data, or both. |
| G7 | Fast graceful save | The hackathon-app snapshot save path from SIGINT to process exit completes in <= 500 ms on the local reference machine. |

## 4. Non-Goals

- No crash-consistent write-ahead log in this milestone. Snapshots are saved on graceful shutdown.
- No automatic periodic snapshotting in this milestone.
- No distributed or multi-process snapshot coordination.
- No backwards-compatible migration framework beyond a manifest schema version check.
- No encryption-at-rest for local snapshot files. Users must treat snapshots as sensitive because they may contain object data, parameter values, Lambda environment variables, and IAM access keys.
- No promise that every compiled service has full snapshot support in the first implementation. The initial acceptance target is the full hackathon-app resource graph plus S3 and DynamoDB data.

## 5. Users

- **Primary:** Rustack users running IaC-driven local stacks with Pulumi, Terraform, CDK, or SDK bootstrap code.
- **Secondary:** Rustack contributors validating cross-service behavior without re-provisioning expensive local fixtures on every run.
- **Anti-persona:** Production operators seeking a durable database or backup system. Rustack snapshots are local emulator persistence, not production storage.

## 6. Success Metrics

- `make pulumi-hackathon-snapshot-smoke` passes: deploy hackathon-app, save a snapshot by terminating Rustack gracefully, restart from the same snapshot, and verify Pulumi refresh/read-after-write state.
- Targeted Rust tests prove missing snapshot names start empty, invalid names are rejected, and atomic overwrite preserves a previous complete snapshot if a temp write fails.
- S3 snapshot tests prove object metadata and body bytes survive load/save.
- DynamoDB snapshot tests prove table metadata and item contents survive load/save.

## 7. Naming Conventions

- CLI flag: `--snapshot <name>`.
- Snapshot root env override: `RUSTACK_SNAPSHOT_DIR`.
- Default snapshot root: `.rustack/snapshots` relative to the current working directory.
- Snapshot name charset: ASCII alphanumeric, `_`, `-`, and `.`; maximum 64 bytes; no path separators or `..`.
- On-disk schema version: `2`.
- Primary artifact names: `manifest.ss.zst`, `services/<service>/meta.ss.zst`, and optional `services/<service>/data.ss.zst`.
- Service snapshot modules use `snapshot.rs` inside the owning core crate.
