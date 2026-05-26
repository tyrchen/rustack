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
- Manifest load rejects unsupported schema versions and paths that escape the snapshot root.
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
5. Assert the snapshot directory contains `manifest.json` plus expected service shards.
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

- Record the elapsed time between process start and health endpoint readiness during `make pulumi-hackathon-snapshot-smoke`.
- The hackathon-app snapshot must load under 2 seconds locally before the health endpoint reports ready.
- S3 object bodies must not appear base64-encoded inside `resources/s3.json`.

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
