# Lambda hardening design

Status: implementation contract for review R01/R06/R11/R12 and Lambda R16. Supersedes unsafe defaults and mutable layout in [executor design](ruststack-lambda-executor-design.md); retains [Cargo Lambda Runtime API contract](../docs/research/spike-cargo-lambda-runtime-execution.md).

## Boundaries and API

FunctionName is a private validated newtype: 1–64 ASCII alphanumeric, underscore or hyphen bytes. Qualifier is `$LATEST`, positive decimal version or 1–128 ASCII alphanumeric/underscore/hyphen alias. Full ARN must have exactly seven/eight components, supported AWS partition, Lambda service, nonempty bounded ASCII region, twelve-digit account and `function` resource. Partial account ARN and simple qualified names remain supported. No percent-decoding occurs in core: encoded separators are invalid. Store insertion/update and snapshot import validate records independently of HTTP. Explicit qualifiers are checked even when a reference contains a qualifier.

`LambdaConfig::from_env() -> Result<LambdaConfig, LambdaServiceError>` fails on malformed booleans, enums, numbers and conflicts; no environment setting and Default both choose Disabled. Disabled returns explicit unavailable error, never echo. Docker is unsupported and fails explicitly, never native/noop fallback. Native and Auto require explicit operator selection and are trusted-host execution, not a sandbox.

## Immutable artifacts and failures

```text
external name/ARN + ZIP
       │ validate identifiers and size
       ▼
store-owned random artifact ID (never function name)
       │ private staging / code.zip / extracted
       │ validate ZIP, paths, entry modes, CRC, actual expanded bytes
       ├── failure ──► remove only new staging, return ZIP/IO error
       ▼
complete immutable artifact ──► atomic metadata reference replacement
       │                              │
published version keeps reference    latest switches to another artifact
```

Each upload receives an unguessable internal UUID directory. Paths do not join logical function names or qualifiers. Reject symlinks in root ancestry and artifact traversal; ZIP symlinks, absolute/parent/backslash entries and duplicate paths are rejected. Root is operator-owned; native execution is explicitly trusted and cannot be treated as protection against a malicious same-user process racing filesystem operations. Extraction runs on bounded blocking workers, maximum ZIP 50 MiB, expansion 250 MiB, maximum 10,000 entries. No validation or IO failure is swallowed. Staging lifetime owns cleanup on error/cancellation. Completed artifacts are retained while versions/in-flight requests can reference them; cleanup never recursively deletes a name-derived path. Snapshot restore stages all records first, validates names/ARNs/versions/aliases and package metadata, and only then replaces records. It never clears the code root before validating input.

## Revisions, admission and lifecycle

Warm identity includes unique artifact path plus execution configuration fingerprint and resolved version. New uploads/recreation receive distinct artifact IDs; configuration changes alter the fingerprint. Published versions share only immutable references. Existing invocations finish on their captured revision; no new invocation reuses old revision processes. Warm pool is bounded globally to 32 and per function to 1, independent of execution permits.

Provider admits at most 32 execution requests globally and default 8 per logical function across versions; reserved concurrency overrides default, 0 rejects. Event work is held in a tracked bounded task set (128 slots): capacity is acquired before 202; queueing/execution owns its capacity until terminal outcome. Execution permits are acquired immediately before execution, retained through timeout/cancellation and released by RAII. Synchronous overload is an explicit throttling error. Event workers may wait for execution capacity within the 128 accepted-work budget. Supervisor records completion/error/panic/cancellation without logging payloads.

`RustackLambda::quiesce(timeout: Duration) -> Result<(), LambdaServiceError>` stops admission, waits for accepted synchronous/background work, and cancels remaining work on deadline with an explicit failure. It does not delete resource metadata. `shutdown()` cancels/joins accepted work before stopping executor resources. Parent runtime calls quiesce before snapshot and shutdown after snapshot; timeout means do not publish a successful consistency snapshot.

## Verification

Real minimal ZIPs replace historical fake PK blobs. Test illegal name/reference/qualifier at resolver, store, provider and snapshot boundaries; temporary outside sentinels and symlink roots stay unchanged. Bad ZIP/CRC/path/size/IO update preserves prior hash/revision/files. A/B artifacts preserve published A and latest B; warm tests include configuration changes and delete/recreate. Admission tests cover 32/8, reserved=0, 128 Event saturation, cancellation and quiesce deadline. Native bootstrap fixture tests use host-built binaries; Linux Cargo Lambda ELF is not executable natively on macOS. All targeted verification explicitly uses workspace CARGO_TARGET_DIR and managed jobs. Parent owns root manifest/lock, HTTP and application runtime integration.
