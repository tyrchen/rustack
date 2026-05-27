# Rustack Snapshot Binary Archive Design

**Date:** 2026-05-26
**Status:** Draft
**Type:** Design
**Depends on:** [ruststack-snapshot-prd](./ruststack-snapshot-prd.md), [ruststack-snapshot-design](./ruststack-snapshot-design.md), [ruststack-pulumi-hackathon-app](./ruststack-pulumi-hackathon-app.md)

## 1. Purpose

The first runtime snapshot implementation proves the user-visible workflow, but JSON shards plus
many payload files are not the right long-term shape for large local stacks. This design upgrades
the artifact to a small number of service-owned binary archives:

```text
.rustack/snapshots/<name>/
  manifest.ss.zst
  services/
    s3/
      meta.ss.zst
      data.ss.zst
    dynamodb/
      meta.ss.zst
    lambda/
      meta.ss.zst
      data.ss.zst
    sqs/
      meta.ss.zst
      data.ss.zst
    iam/
      meta.ss.zst
```

The key property is that startup reads metadata first. Large payloads live in one service data
archive rather than in thousands of filesystem entries. The archive format is sectioned and
versioned so services can gradually move from generic CBOR sections to service-native columnar
sections without changing the outer layout or CLI contract.

## 2. Goals

| # | Goal | Measure |
|---|------|---------|
| G1 | Keep the public UX unchanged. | `rustack --snapshot <name>` still loads before serving and saves on graceful shutdown. |
| G2 | Replace many payload files with service archive files. | A saved hackathon snapshot contains `manifest.ss.zst` plus at most `meta.ss.zst` and `data.ss.zst` per service. |
| G3 | Make service save/load parallel. | Snapshot manager saves and loads independent services concurrently with a bounded IO/CPU concurrency limit. |
| G4 | Keep the artifact evolvable. | Every archive has magic, archive kind, format version, section directory, bounded lengths, and checksum. |
| G5 | Preserve correctness. | Hackathon Pulumi refresh, S3 object reads, and DynamoDB item reads pass after restart. |
| G6 | Hit real-world performance targets. | Hackathon snapshot graceful save completes in <= 500 ms; cold load to health readiness completes in <= 200 ms on the local reference machine. |

## 3. Non-Goals

- No direct dump of Rust heap structures. `HashMap`, `DashMap`, pointers, allocator state, and
  actor channels are not stable file format contracts.
- No memory mapping in this phase. The format should be mmap-friendly later, but the current code
  remains safe Rust with bounded reads and explicit validation.
- No lazy payload faulting in the first binary archive implementation. `data.ss.zst` is a single
  service archive now; later service-native readers can keep data sections closed until first use.
- No backward compatibility with pre-merge JSON snapshot artifacts. This PR has not shipped a
  public snapshot format yet, so v2 can replace v1 before merge.
- No crash-consistent write-ahead log. Save is still graceful-shutdown based.

## 4. Target Layout

```text
Snapshot root
┌────────────────────────────────────────────────────────────────┐
│ <name>/                                                        │
│   manifest.ss.zst                                              │
│   services/                                                    │
│     <service>/                                                 │
│       meta.ss.zst                                              │
│       data.ss.zst       optional, omitted when service has no   │
│                         external payload archive               │
└────────────────────────────────────────────────────────────────┘
```

`manifest.ss.zst` records only global metadata and file locations. It does not contain service
state. `meta.ss.zst` contains control-plane resource state, indexes, and compact data-plane rows
when they are small enough for fast startup. `data.ss.zst` contains large payload streams: S3 object
bodies, multipart part bodies, Lambda code archives, SQS messages, CloudWatch log events, Kinesis
records, or future service-owned payload segments.

## 5. Archive Envelope

Each `*.ss.zst` file is an outer zstd frame containing an inner Rustack snapshot archive:

```text
Compressed file
  zstd frame
    ArchiveHeader
    SectionDirectory[section_count]
    SectionPayloads
    ArchiveFooter
```

All multi-byte integers are little-endian. The loader treats every archive as untrusted input.
Offsets and lengths are checked before slicing.

Header fields:

| Field | Type | Meaning |
|---|---:|---|
| magic | `[u8; 8]` | `RSSNAP\0\x02` for Rustack snapshot archive v2. |
| archive_kind | `u16` | `1 = manifest`, `2 = service_meta`, `3 = service_data`. |
| flags | `u16` | Reserved; writer sets `0`, loader rejects unknown bits. |
| header_len | `u32` | Fixed header length for this version. |
| section_count | `u32` | Number of directory entries. |
| file_len | `u64` | Inner uncompressed archive byte length including footer. |
| reserved | `u64` | Reserved; writer sets `0`, loader rejects non-zero. |

Directory entry:

| Field | Type | Meaning |
|---|---:|---|
| section_kind | `u16` | Service/global section id. |
| flags | `u16` | Section-local flags. |
| offset | `u64` | Offset from beginning of inner archive. |
| len | `u64` | Payload byte length. |
| row_count | `u64` | Optional logical row count for benchmark/reporting. |

Footer fields:

| Field | Type | Meaning |
|---|---:|---|
| crc32 | `u32` | CRC32 over all inner archive bytes before the footer. |
| payload_len | `u64` | Number of bytes covered by `crc32`. |

CRC32 is an integrity check for local corruption and parser bugs. It is not a security signature.
If Rustack later supports externally signed snapshots, the signature belongs in the manifest and
does not remove bounds validation.

## 6. Required Sections

### 6.1 Manifest Archive

`manifest.ss.zst` requires one section:

| Kind | Name | Payload |
|---:|---|---|
| 1 | `manifest_cbor` | CBOR-encoded `SnapshotManifest`. |

`SnapshotManifest` fields:

```text
schema_version: u32
snapshot_name: string
created_by: "rustack"
rustack_version: string
saved_at_unix_millis: u64
services: map service_name -> ServiceManifest
```

`ServiceManifest` fields:

```text
kind: resource | data
meta_file: "services/<service>/meta.ss.zst"
data_file: optional "services/<service>/data.ss.zst"
meta_compressed_bytes: u64
meta_uncompressed_bytes: u64
data_compressed_bytes: optional u64
data_uncompressed_bytes: optional u64
```

Every path in the manifest must be a relative child path with only normal components. Missing
services load as empty state.

### 6.2 Service Metadata Archive

The first implementation requires one metadata section:

| Kind | Name | Payload |
|---:|---|---|
| 1 | `state_cbor` | CBOR-encoded service snapshot struct. |

This keeps the implementation bounded while replacing JSON parse/format overhead and allowing a
stable binary envelope. Later service-native sections can add, for example, `bucket_rows`,
`object_rows`, `table_rows`, `item_rows`, and `index_rows`. Readers reject unsupported required
format versions rather than guessing.

### 6.3 Service Data Archive

`data.ss.zst` requires two sections:

| Kind | Name | Payload |
|---:|---|---|
| 1 | `data_directory_cbor` | CBOR-encoded list of path/offset/len/crc entries. |
| 2 | `data_bytes` | Concatenated raw payload bytes. |

The directory entry shape:

```text
relative_path: string
offset: u64
len: u64
crc32: u32
```

For the compatibility implementation, services can still produce data into a temporary staging
directory. The snapshot manager packs that directory into `data.ss.zst` and removes the staging
directory before publishing the snapshot. On load, the manager unpacks `data.ss.zst` into a staging
directory before calling the service's import method. This keeps final snapshots compact while
allowing service-native lazy data readers to land later without changing the outer format.

## 7. Snapshot Trait

The app-level trait becomes archive-oriented:

```rust
#[async_trait]
trait SnapshotService: Send + Sync {
    fn service_name(&self) -> &'static str;
    fn snapshot_kind(&self) -> SnapshotKind;
    fn dependencies(&self) -> &'static [&'static str] { &[] }

    async fn save_meta(&self, data_staging_dir: &Path) -> Result<Vec<u8>>;
    async fn load_meta(&self, state_cbor: &[u8], data_staging_dir: &Path) -> Result<()>;
    async fn shutdown(&self) -> Result<()> { Ok(()) }
}
```

`save_meta` returns the service metadata payload already encoded as CBOR. If a service has external
payload files, it writes them into `data_staging_dir` and references those relative paths from its
metadata. The snapshot manager owns archive packing and the outer `*.ss.zst` format, so services do
not duplicate compression and manifest logic.

## 8. Save Algorithm

```text
save(snapshot)
  │
  ├─ create root temp dir
  ├─ prepare services/ and staging/
  ├─ run service saves in parallel with bounded concurrency
  │    ├─ service.save_meta(staging/<service>)
  │    ├─ write services/<service>/meta.ss.zst
  │    └─ pack staging/<service> into services/<service>/data.ss.zst if non-empty
  ├─ write manifest.ss.zst last
  └─ atomically replace the snapshot directory
```

The previous complete snapshot is not removed until the replacement directory exists. If replacement
fails, the manager attempts to restore the previous snapshot name.

## 9. Load Algorithm

```text
load(snapshot)
  │
  ├─ missing snapshot directory -> start empty
  ├─ read manifest.ss.zst
  ├─ validate schema version and service paths
  ├─ run independent service loads in parallel
  │    ├─ unpack data.ss.zst into load-staging/<service> if present
  │    ├─ read services/<service>/meta.ss.zst
  │    └─ service.load_meta(state_cbor, load-staging/<service>)
  └─ post-load providers are ready before the gateway binds
```

The first implementation has no service dependencies, so all registered services may load in
parallel. When future services need dependency ordering, the manager will topologically sort
`dependencies()` and run one dependency layer at a time.

## 10. Performance Budgets

The hackathon-app stack is the real-world reference fixture for this phase:

| Operation | Budget | Measurement |
|---|---:|---|
| Runtime snapshot save | <= 500 ms | Rustack-internal time spent in `SnapshotConfig::save` during `make pulumi-hackathon-snapshot-smoke`. |
| Runtime snapshot load | <= 200 ms | Rustack-internal time spent in `SnapshotConfig::load` during `make pulumi-hackathon-snapshot-smoke`. |

The smoke target also reports SIGINT-to-exit wall time and process-start-to-health wall time as
diagnostic numbers. Those include process startup, HTTP router construction, polling resolution, and
shutdown bookkeeping, so they are not the snapshot format budget.

Expected optimizations for this phase:

- service save/load runs concurrently;
- zstd compression/decompression runs in `spawn_blocking` so it does not block the async runtime;
- metadata uses CBOR instead of pretty JSON;
- S3 data is one compressed archive instead of many final filesystem files;
- manifest contains byte counts so benchmark output can report sizes without scanning trees.

If the target is missed, optimize in this order:

1. Avoid writing empty `data.ss.zst` files.
2. Lower zstd compression level for runtime snapshots.
3. Parallelize S3 data packing by chunk if S3 dominates save.
4. Move Lambda zip bytes from metadata CBOR into service data archive.
5. Replace high-cardinality service CBOR sections with service-native row sections.

## 11. Verification

Required tests:

- archive round-trip preserves section bytes and row counts;
- archive loader rejects corrupt magic, unsupported kind, invalid footer CRC, and out-of-bounds
  sections;
- data archive packing rejects path traversal and restores files byte-for-byte;
- manifest path validation rejects escaping paths;
- service round-trip tests continue to pass with CBOR state payloads;
- `make pulumi-hackathon-snapshot-smoke` verifies Pulumi refresh, S3 object read, DynamoDB item
  read, snapshot file shape, save latency, and load latency.

## 12. AGENTS.md Binding

- Error Handling: app orchestration uses `anyhow::Context`; service crates keep `thiserror` error
  enums.
- Async/concurrency: compression/decompression is `spawn_blocking`; service orchestration uses
  bounded concurrent tasks.
- Safety/security: no `unsafe`; snapshot files are hostile input; all offsets, lengths, counts,
  archive kinds, and manifest paths are validated.
- Serialization: Rustack-owned archive metadata uses `serde` `camelCase`; hot artifact sections use
  stable little-endian binary plus CBOR payloads only where the service schema is still generic.
- Testing: corrupt-input tests are required for the binary loader because it is a trust boundary.
- Observability: log service names, byte counts, and timings; never log payload bodies or secrets.
- Performance: real-world budget evidence must be posted to the PR after benchmark execution.
- Dependencies: `zstd` is used for archive compression; versions must be checked against current
  docs/crates evidence and pass dependency gates.

## 13. Key Decisions

| ID | Decision | Rationale |
|---|---|---|
| D6 | Use `manifest.ss.zst` plus per-service `meta.ss.zst` and optional `data.ss.zst` | Avoids many small final files while preserving service ownership and parallelism. |
| D7 | Use a stable section envelope instead of direct heap dumps | Keeps snapshots portable, versionable, and safe to validate. |
| D8 | Use CBOR as the first service metadata payload | Converts the current JSON snapshot to a compact binary payload without blocking on service-native row codecs. |
| D9 | Pack legacy service data staging into one data archive | Delivers the final file shape now while allowing S3/Lambda lazy data readers later. |
| D10 | Parallelize at the service level first | Service-level concurrency gives most of the hackathon win without adding per-service shard schedulers. |

## 14. Cross-References

- Extends [ruststack-snapshot-design.md](./ruststack-snapshot-design.md)
- Updates [ruststack-snapshot-impl-plan.md](./ruststack-snapshot-impl-plan.md)
- Verified by [ruststack-snapshot-verification-plan.md](./ruststack-snapshot-verification-plan.md)
