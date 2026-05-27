# Rustack Runtime Snapshot Verification Plan

**Date:** 2026-05-26
**Status:** Draft
**Type:** Verification Plan
**Depends on:** [ruststack-snapshot-design](./ruststack-snapshot-design.md), [ruststack-snapshot-impl-plan](./ruststack-snapshot-impl-plan.md)

## 1. Unit Tests

Required unit coverage:

- Snapshot name validation accepts `hackathon`, `dev.1`, `team_stack-01` and rejects empty names, names longer than 64 bytes, path separators, `..`, and non-ASCII characters.
- CLI classification recognizes `--snapshot <name>` as run mode with snapshot config and rejects missing values or unknown flags.
- Snapshot path resolution honors `RUSTACK_SNAPSHOT_DIR` and defaults to `.rustack/snapshots`.
- `*.ss.zst` archive round-trip preserves section bytes, flags, and row counts.
- Archive load rejects unsupported magic, unsupported archive kind, corrupt CRC footer, duplicate
  sections, and out-of-bounds section offsets/lengths.
- Manifest load rejects unsupported schema versions and paths that escape the snapshot root.
- Service data archive packing/unpacking preserves file bytes and rejects path traversal entries.
- S3 provider round-trip preserves bucket state, bucket policy/tags where present, object metadata, and object bytes.
- DynamoDB provider round-trip preserves table metadata, tags, TTL/PITR where present, and all items.
- Representative resource provider round-trips cover SSM parameter, IAM role or policy, Lambda function/event source mapping, API Gateway API/route/stage, CloudFront distribution/OAC/function, and SQS queue metadata.

## 2. Integration Smoke

Add a repository target:

```bash
make pulumi-hackathon-snapshot-smoke
```

The target must:

1. Start Rustack with `--snapshot <unique-name>` on a local endpoint.
2. Run `examples/pulumi/hackathon-app` with a persistent temporary Pulumi file backend.
3. Run `pulumi up --yes --skip-preview`.
4. Terminate Rustack with SIGINT so the normal Ctrl+C path saves the snapshot.
5. Assert the snapshot directory contains `manifest.ss.zst` plus expected service `meta.ss.zst`
   files and no unpacked S3 `objects/*.bin` final files.
6. Restart Rustack with the same `--snapshot <unique-name>`.
7. Run `pulumi refresh --yes --skip-preview` or an equivalent read-after-write validation against the same Pulumi stack.
8. Verify at least one S3 object from the stack is readable after restart.
9. Verify the DynamoDB table exists after restart; if the smoke writes an item, verify that item.
10. Destroy the stack and remove temporary Pulumi/snapshot state.

## 3. Manual Reproduction

For local debugging:

```bash
RUSTACK_SNAPSHOT_DIR=/tmp/rustack-snapshots \
GATEWAY_LISTEN=127.0.0.1:4567 \
cargo run -p rustack-cli -- --snapshot hackathon
```

In a second shell:

```bash
RUSTACK_ENDPOINT=http://127.0.0.1:4567 \
PULUMI_STACK=rustack-hackathon-snapshot \
make pulumi-hackathon-smoke
```

When using the smoke script as-is, disable destroy for snapshot debugging; the dedicated snapshot smoke target automates that flow.

## 4. Performance Checks

- Record Rustack-internal snapshot save latency during `SnapshotConfig::save`.
- Record Rustack-internal snapshot load latency during `SnapshotConfig::load`.
- Also report SIGINT-to-exit wall time and process-start-to-health wall time as diagnostics.
- The hackathon-app snapshot save path must complete in <= 500 ms and the snapshot load path must
  complete in <= 200 ms on the local reference machine after the release binary is already built.
- S3 object bodies must be packed into `services/s3/data.ss.zst`, not stored as unpacked final
  files or base64-encoded metadata.
- The PR comment must include save ms, load ms, manifest size, total service metadata bytes, and
  total service data bytes for the benchmark run.

## 5. Required Gates

Because this feature changes Rust source, Cargo manifests, Makefile targets, CLI behavior, specs, and test automation, run:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
git diff --check
```

If dependency versions change, also run:

```bash
cargo deny check
cargo audit
```

The stricter clippy command from AGENTS.md is required for snapshot boundary modules because they parse file-system input:

```bash
cargo clippy --workspace --all-targets -- \
  -D warnings -W clippy::pedantic \
  -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::indexing_slicing -W clippy::panic
```
