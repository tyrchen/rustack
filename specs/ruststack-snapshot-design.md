# Rustack Runtime Snapshot Design

**Date:** 2026-05-26
**Status:** Draft
**Type:** Design
**Depends on:** [ruststack-snapshot-prd](./ruststack-snapshot-prd.md), [ruststack-pulumi-hackathon-app](./ruststack-pulumi-hackathon-app.md)

## 1. Purpose

Runtime snapshots provide local persistence for Rustack's in-memory service providers. The design keeps the CLI simple while making resource and data snapshots separate implementation concepts. This avoids a single huge JSON blob, allows S3 object bodies to live as files, and lets each service implement the persistence boundary closest to its internal store.

## 2. User Contract

`rustack --snapshot <name>` means:

1. Validate `<name>` before starting the gateway.
2. Resolve the snapshot path under `RUSTACK_SNAPSHOT_DIR` or `.rustack/snapshots`.
3. If the snapshot exists, load all supported service shards before binding the gateway port.
4. Serve requests normally.
5. On Ctrl+C or another graceful shutdown path handled by Rustack, drain HTTP connections, stop runtime-owned background workers that need quiescing, then save the current state into a temporary directory and atomically replace the named snapshot.

`--snapshot` is ignored by `--help`, `--version`, and `--health-check` because those modes do not start a runtime.

## 3. Snapshot Layout

```text
.rustack/snapshots/<name>/
  manifest.json
  resources/
    s3.json
    dynamodb.json
    sqs.json
    ssm.json
    iam.json
    lambda.json
    apigatewayv2.json
    cloudfront.json
  data/
    s3/
      objects/<stable-id>.bin
      parts/<stable-id>.bin
    <service>/
```

The first implementation may omit a file for a service that has no state or no snapshot support. Services missing from the manifest load as empty state.

`manifest.json` is the only root file the loader reads first:

```json
{
  "schemaVersion": 1,
  "createdBy": "rustack",
  "rustackVersion": "0.8.0",
  "snapshotName": "hackathon",
  "savedAtUnixMillis": 1779753600000,
  "services": {
    "s3": { "file": "resources/s3.json", "kind": "data" },
    "dynamodb": { "file": "resources/dynamodb.json", "kind": "data" },
    "iam": { "file": "resources/iam.json", "kind": "resource" }
  }
}
```

## 4. Resource and Data Boundaries

Resources are control-plane objects created by IaC or management APIs: buckets, tables, queues, IAM roles, Lambda functions, API Gateway routes, CloudFront distributions, and SSM parameters. Data is user/application payload: S3 object bodies, DynamoDB items, SQS messages, stream records, and future service-specific payloads.

The CLI always loads and saves both for a named snapshot. Internally, service shard types are split so later commands can support partial export/import without changing the format:

```text
                 ┌───────────────────────────────┐
                 │ rustack --snapshot <name>     │
                 │ - validate name               │
                 │ - resolve root                │
                 └───────────────┬───────────────┘
                                 │
              load before bind   │   save after drain
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ SnapshotManager                                                     │
│                                                                     │
│  manifest.json ──► resources/*.json ──► provider stores             │
│          │              │                                           │
│          │              └────────────► control-plane resources      │
│          │                                                          │
│          └────────────► data/<service>/* ───────────────► payloads  │
└─────────────────────────────────────────────────────────────────────┘
        ▲                    ▲                    ▲
        │                    │                    │
   S3 provider         DynamoDB provider     Pulumi resource providers
   resource+data       resource+data         resource shards
```

## 5. Runtime Lifecycle

The runtime needs explicit access to providers, not only boxed HTTP routers. `build_services` therefore returns a runtime state holder alongside routers:

```text
main
 │
 │ 1. parse CLI, load SnapshotConfig
 ▼
build_runtime(snapshot)
 │
 │ 2. construct providers from env config
 │ 3. apply service shards to providers
 │ 4. construct HTTP routers from same providers
 ▼
serve(listener, gateway)
 │
 │ 5. Ctrl+C
 ▼
drain connections
 │
 │ 6. stop provider-owned workers that need quiescing
 │ 7. collect provider snapshots
 │ 8. atomic save
 ▼
exit
```

This avoids trying to downcast `Box<dyn ServiceRouter>` or reconstruct state from HTTP traffic. Services remain the owners of their persistence contracts.

Runtime orchestration uses an app-layer trait registry, so the manager does not need a new provider-specific branch for each service:

```rust
#[async_trait]
trait SnapshotService: Send + Sync {
    fn service_name(&self) -> &'static str;
    fn snapshot_kind(&self) -> SnapshotKind;

    async fn save_state(&self, state_file: &Path, data_dir: &Path) -> Result<()>;
    async fn save_data(&self, data_dir: &Path) -> Result<()> { Ok(()) }
    async fn load_data(&self, data_dir: &Path) -> Result<()> { Ok(()) }
    async fn load_state(&self, state_file: &Path, data_dir: &Path) -> Result<()>;
    async fn shutdown(&self) -> Result<()> { Ok(()) }
}
```

`save_state` receives the service data directory because some services, notably S3, need state metadata that references files under `data/<service>/`. Most resource-only services use only the JSON state file and inherit the no-op data methods.

## 6. Service Contracts

Each supported service exposes small snapshot methods from its core crate:

```rust
pub fn export_snapshot(&self) -> Result<ServiceSnapshot, ServiceSnapshotError>;
pub fn import_snapshot(&self, snapshot: ServiceSnapshot) -> Result<(), ServiceSnapshotError>;
```

Async is used only when body files or code archives must be read or written. The app-layer `SnapshotService` wrapper serializes the service snapshot to `resources/<service>.json`, passes `data/<service>` to services that need payload files, and records the shard in the manifest. Errors follow AGENTS.md § Error Handling: `thiserror` for library errors and `anyhow` with context in the CLI.

Initial support matrix:

| Service | Resources | Data | Notes |
|---------|-----------|------|-------|
| S3 | buckets, configs, object metadata | object bodies, multipart parts | Object bodies are files under `data/s3` to avoid base64 expansion and improve load time. |
| DynamoDB | table metadata, TTL, tags, PITR | items | Table items are JSON because DynamoDB `AttributeValue` already has a compact serde shape. |
| SQS | queue metadata, attributes, tags | messages if cheap to expose | Actor snapshot command quiesces one queue at a time. |
| SSM | parameter metadata | parameter versions and values | SecureString values are local plaintext, matching current in-memory behavior. |
| IAM | users, roles, policies, instance profiles, access keys, OIDC providers | N/A | Sensitive values stay local but are not redacted in snapshot files. |
| Lambda | functions, versions, aliases, policies, URLs, event source mappings | zip bytes | Rebuild extracted code directories from zip bytes during import. |
| API Gateway V2 | APIs and nested routes/integrations/stages/deployments | N/A | Store maps are already cloneable records. |
| CloudFront | distributions, OAC/OAI, functions, policies, tags | N/A | Managed policies are still seeded by provider construction and may be skipped from saved customer state. |
| STS | N/A | N/A | Stateless. |

## 7. Atomic Save

Save writes to `<snapshot>.tmp-<pid>-<nonce>`, fsyncs best-effort by completing all file writes, writes `manifest.json` last, then renames into place. If a previous snapshot exists:

1. Rename previous snapshot to `<snapshot>.old-<pid>-<nonce>`.
2. Rename temp snapshot to `<snapshot>`.
3. Remove old snapshot.

If step 2 fails, restore the old snapshot name before returning the error. The implementation must never delete the previous complete snapshot before the replacement exists.

## 8. Load Validation

Loader validation rejects:

- snapshot names outside the allowed charset/length;
- manifest schema versions other than `1`;
- service shard paths that escape the snapshot root;
- malformed JSON shards.

Missing snapshot directories are not errors. They start an empty runtime and save a new snapshot on graceful shutdown.

## 9. Performance

The design avoids API replay. Loading is direct deserialization plus file references/copies. The hackathon-app snapshot must load before the health endpoint is exposed. S3 object bodies should not be base64-encoded in the resource JSON; large object bodies should remain files and be read only if the current storage backend needs to materialize them.

Minimum implementation budget:

- hackathon-app snapshot load completes in less than 2 seconds on a local developer machine;
- S3 object body files are copied or referenced without JSON base64 expansion;
- per-service JSON shards are independent so later parallel loading can be added without format changes.

## 10. Observability

Rustack logs:

- snapshot name and resolved root at startup;
- whether a snapshot was loaded or not found;
- per-service load/save counts where cheap;
- atomic save success or error.

Logs must not dump object bodies, parameter values, IAM secret access keys, Lambda environment variables, or request payloads.

## 11. AGENTS.md Binding

- Error handling: per AGENTS.md § Error Handling. Library errors use `thiserror`; CLI orchestration uses `anyhow::Context`.
- Async/concurrency: per AGENTS.md § Async & Concurrency. Snapshot save runs after connection drain; SQS actor state is requested by channel rather than shared locks.
- Type design: snapshot name is a validated newtype, not a raw user-supplied path.
- Safety/security: no `unsafe`; validate all snapshot file paths and schema versions at load boundary.
- Serialization: use serde with stable field names and `#[serde(rename_all = "camelCase")]` for new Rustack-owned snapshot structs.
- Testing: add unit tests for name validation and service round-trips; add an integration smoke for hackathon-app save/load.
- Observability: use `tracing`, no `println!` in production Rust paths.
- Performance: avoid base64 for S3 data files; pre-size vectors when counts are known.
- Documentation: update `--help`, specs index, and user-facing docs if the CLI surface changes.

## 12. Key Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | One CLI snapshot name, split resources/data internally | Preserves simple UX while allowing efficient storage and future partial import/export. |
| D2 | Direct provider/store serialization instead of HTTP replay | API replay is slower, loses current state if mutations happen outside HTTP, and requires buffering every request body. |
| D3 | Per-service shards | Keeps ownership local to each service and avoids a monolithic schema that changes on every service edit. |
| D4 | Atomic directory replace | Protects the previous snapshot from partial writes or interrupted shutdown saves. |
| D5 | Initial acceptance targets hackathon-app services | Gives an end-to-end correctness proof across real Pulumi resources without blocking on every stateless or unexercised service. |

## 13. Cross-References

- [ruststack-snapshot-prd.md § 3. Goals](./ruststack-snapshot-prd.md#3-goals)
- [ruststack-snapshot-impl-plan.md](./ruststack-snapshot-impl-plan.md)
- [ruststack-snapshot-verification-plan.md](./ruststack-snapshot-verification-plan.md)
- [ruststack-pulumi-hackathon-app.md](./ruststack-pulumi-hackathon-app.md)
