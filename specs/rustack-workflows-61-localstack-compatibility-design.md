# Workflows 61: External LocalStack compatibility tests

Status: D114 qualifies all123 selected original nodes under one source cohort; broader M4 qualification remains separate. Continues P5 verification within the existing state/integration/API scope. Based on [the pinned upstream study](../docs/research/study-localstack-workflow-tests.md).

## 1. Outcome and authority

Execute an explicit, reproducible subset of LocalStack's original Step Functions pytest tests against Rustack, retaining original test bodies, templates and AWS snapshot assertions. Test counts describe collected pytest nodes; report execution/input counts separately where available. Never report the entire LocalStack suite or cloud parity as passed from a subset.

The first inventory covers deterministic JSONPath intrinsics, Choice and data flow, basic states, Parallel/Inline Map, and the overlapping Lambda task/error/timeout/nested patterns. Inventory and exclusions are chosen before executing the selected tests. Unsupported JSONata, EXPRESS, Activities, distributed Map, AWS SDK integrations, IAM authorization, ECR-specific metadata and non-overlapping service behavior remain explicitly excluded. Callback and image-version cases must be reported according to actual upstream coverage, not inferred from existing Rustack tests.

## 2. Execution boundary

The vendor gitlink and actual checkout must equal `45dfee8ce27df00fcac3f91b8071c8156eb5c5c7`. A checked-in manifest pins selected node IDs and provenance. Collection is exact: no missing, duplicate, unexpected, skipped or xfailed required nodes. Validate files, hashes, bounds and dependencies before starting Rustack or creating resources. Bound manifest/file reads, case count, wall time, diagnostic files and SDK retries/timeouts. Reject symbolic-link or traversal substitutions in evidence paths.

Use original upstream pytest imports with a pinned isolated Python environment. Disable auto-loaded plugins and root conftest; explicitly load the Rustack fixture plugin and upstream snapshot plugin. Run one selected group at a time against an actual owned Rustack process with signed boto3 calls. Runtime startup, readiness, observers, graceful shutdown and forced fallback remain under existing Rust test owners. No background process may outlive its owner unnoticed.

Clients receive an explicit loopback endpoint, region and local test credentials. No AWS profile, ambient credential provider, metadata service or default AWS endpoint is allowed. Do not execute live AWS tests automatically. A future cloud differential invocation requires an explicit isolated account configuration and a separate intentional target.

## 3. Allowed fixture adaptations

- Supply `aws_client` and client factories using actual SDK clients pointed at the owned endpoint, retries disabled and bounded connect/read timeouts.
- Supply a valid local role ARN for tests that treat it as metadata. Do not claim policy validation or IAM lifecycle coverage.
- Create machines/functions through public APIs. Register ownership immediately upon successful creation. Cleanup stops active executions, deletes owned machines/functions, closes clients and propagates all errors.
- Preserve original handler code for real Docker execution. The image packaging, architecture, immutable image ID, runtime version and any supported handler/environment overrides are explicit artifacts. Do not substitute an echo handler or silently map ECR-specific fixtures to local images.
- Keep original snapshot/session transformers. Copy baseline files into private evidence storage before snapshot verification, preserve original node IDs, and persist actual/validation artifacts only there. The baseline bytes must remain unchanged.

Snapshot update/raw/skip-all flags and environment variables are forbidden. Existing upstream skip-path markers may be retained with their provenance, but new masks or regenerated expectations cannot make a failed comparison pass. Full-history and output-only upstream assertions retain their original scope.

## 4. Reports and failure semantics

Write a versioned JSON report containing vendor commit, manifest/source/dependency hashes, original node IDs, expected/collected/executed counts, pass/fail/skip/xfail outcomes, per-node snapshot paths and hashes, actual runtime/packaging mode and cleanup status. Save bounded private raw pytest diagnostics and JUnit/validation output. A successful process exit alone is insufficient: require exact selected outcomes and successful snapshot verification. Any mismatch, missing baseline or cleanup uncertainty fails the target and retains evidence.

Classify findings as product bugs within the supported profile, deliberate profile restrictions, upstream-oracle defects requiring independent evidence, or adapter faults. Preserve original failing evidence before fixes. Never remove a failed selected case to obtain a passing report. Any later inventory change needs a recorded reason and a new manifest identity.

## 5. Delivery and verification

Expose environment preparation, normal adapter controls, pure-ASL execution and real-Docker execution through discoverable Make targets. Use existing process/resource ownership helpers and explicit Docker sockets; preserve unrelated containers, volumes, images, daemons and contexts.

Controls must prove rejection of source drift, snapshot mutation/update attempts, missing/extra/skipped nodes, altered semantic output/error assertions and failed resource cleanup. Relevant Rust gates apply to added Rust owners or product fixes. An independent read-only review checks the original assertions, fixture trust boundaries and actual saved evidence before completion.

Exit evidence is an exact successful required inventory, unchanged upstream baselines, joined owned processes and zero owned Docker remnants, with failures/exclusions and actual coverage documented. A compatibility failure is useful evidence but is not a passing exit. Broader M4 platform/reference and same-commit release qualification remain open.

## 6. Historical pre-D114 baseline

The pre-D114 source c16d68e executes every selected node without skipped/xfail cases: pure88/91 and Docker30/32. All current-source Rust gates, Native/Action checks and physical cleanup pass. See [the evidence ledger and remaining original assertions](../docs/research/study-localstack-workflow-tests.md#final-source-cohort-verification). The three formal-profile and two static-diagnostic differences remain original failures; the targets correctly return nonzero. Implementation of the probe lane is complete, while the exact successful-inventory acceptance criterion above is unmet and is not waived by this baseline.

## 7. Authorized observed-AWS compatibility continuation (D114)

The user explicitly authorizes matching the five remaining recorded AWS behaviors. This supersedes the earlier formal-only/local-static-cause choices for these cases. Keep the same91 pure and32 Docker nodes, vendor commit, original baseline bytes and masks. No live AWS calls, new excluded scope or expectation regeneration are required. Reclassify the existing Format2 exclusion reason as historical selection rather than a current unsupported feature; the unchanged node set receives a new manifest identity and genuinely rebuilt image receipts. Acceptance is91/91 plus32/32 with the existing resource/verification controls.

```text
┌─ Validated ASL / captured Lambda task ─────────────────────────┐
│ literal decoding → bounded Format / StringToJson              │
│ validated target + tenant → owned service-error mapping       │
└────────────────────────┬─────────────────────────────────────┘
                         ▼
┌─ Observed AWS wire semantics ─────────────────────────────────┐
│ collection text │ empty JSON→null │ qualified missing target   │
│ all existing byte/depth/work bounds and redaction preserved    │
└────────────────────────┬─────────────────────────────────────┘
                         ▼
         original snapshot assertions → exact123/123 gate
```

### Intrinsic grammar and value conversion

- Decode intrinsic string escapes exactly once. Ordinary single-quoted string arguments may contain literal `{`/`}`; escaped braces retain the same value. Unknown/dangling escapes and unterminated strings still reject. Format's first argument retains escape provenance; placeholder count and malformed/unescaped format-brace checks remain unchanged.
- Format accepts string, null, boolean and number substitutions as before. Arrays render recursively as `[a, b]`, objects as `{key=value, key=value}`, strings/keys without JSON quotes, null as `null` and booleans/numbers using the existing JSON scalar representation. Delimiters are comma+space and equals. Explicitly sort object keys lexicographically so the selected profile is independent of serde feature unification; the two recorded object observations use this order. This selects deterministic local ordering, without claiming arbitrary JVM map-implementation ordering parity.
- Walk only previously validated Payload trees (depth≤64). Charge value visits and object-entry sorting to the shared work budget before allocation; charge every emitted byte to the scanned-text budget and existing262144-byte intermediate/final encoded payload bounds. Never return partial output. Deep/wide/large outputs still produce typed quota errors; no host formatter or executable interpolation is introduced.
- StringToJson returns null for empty text or text consisting only of ASCII JSON whitespace (space, tab, CR, LF). Charge whitespace classification before this branch and independently charge subsequent parse/normalization scans. Non-ASCII whitespace/control characters, nonempty malformed JSON, duplicate keys and original byte/depth/member limits still reject. Existing integer-negative-zero normalization and conditional reparse charging are unchanged.

### Missing Lambda service causes

FunctionNotFound, VersionNotFound and AliasNotFound keep `Lambda.ResourceNotFoundException`. Build the public target from the already validated caller function reference, optional qualifier and captured tenant scope, never by formatting arbitrary provider/host error text. Plain names become full same-scope ARNs; retain embedded or explicit qualifiers and append `$LATEST` when absent. Revalidate canonical target grammar and its512-byte ceiling before publishing; tenant identity must never be hardcoded.

The exact cause is `Function not found: <qualified ARN> (Service: AWSLambda; Status Code: 404; Error Code: ResourceNotFoundException; Request ID: <UUID>; Proxy: null)`. The UUID identifies this local integration service failure, not an admitted Lambda invocation or a real cloud request. Obtain fallible OS entropy in retained blocking work under the existing accepted-call capacity; validate version4/RFC4122 before publishing. An entropy/worker/conversion failure uses the existing safe service failure path, with a retained redacted typed diagnostic. Successful Lambda response metadata continues using the actual provider invocation ID.

Both prequeue resolution errors and admission-receipt errors use this mapping, while retaining the same start/result/cleanup owner. Direct/optimized/callback event ordering and physical cleanup semantics do not change. Other service/IO/unknown errors retain static bounded causes and internal typed sources. Public causes stay within32768 bytes; Debug/logs redact targets and host error sources.

### Verification order

Finish all code and regression cases, perform independent read-only review and fix findings, then run focused core/bridge/boundary checks before resource execution. Cover nested collections, feature-independent order, Unicode/escaping, literal braces versus format placeholders, empty/ASCII/non-ASCII JSON inputs, quotas, complete missing target qualifiers/scopes, source redaction, entropy failures and request-ID uniqueness. Then execute the exact original91/32 inventories and standard Rust/Native/Action gates under one frozen source cohort. Preserve all previous failed reports as historical evidence; update the English ledger and PR with the actual new results.

## 8. D114 measured acceptance

Source3207a320a329820fa1d0890145c9518b3be6488a3ef402c684667d700e34b226 passes the unchanged91/91 pure and32/32 actual Docker nodes with original baselines/masks, exact collection, complete assertions and successful joined cleanup. All15 Rust gates,3231 Native passes,26 actual Action controls and13 adapter controls pass. The selected strict successful-inventory criterion is met; no current123-node compatibility difference is waived. See [the D114 source/image/report ledger](../docs/research/study-localstack-workflow-tests.md#d114-qualified-source-cohort). This does not qualify the whole upstream suite, live AWS, managed ZIP/ECR lifecycle or the remaining M4 platforms/reference/release bundle.
