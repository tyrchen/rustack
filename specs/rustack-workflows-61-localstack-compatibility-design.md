# Workflows 61: External LocalStack compatibility tests

Status: probes implemented and independently reviewed; strict compatibility exit remains open with five original assertion differences. Continues P5 verification; does not expand the product's advertised AWS profile. Based on [the pinned upstream study](../docs/research/study-localstack-workflow-tests.md).

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

## 6. Measured baseline

The final recorded source executes every selected node without skipped/xfail cases: pure88/91 and Docker30/32. All current-source Rust gates, Native/Action checks and physical cleanup pass. See [the evidence ledger and remaining original assertions](../docs/research/study-localstack-workflow-tests.md#final-source-cohort-verification). The three formal-profile and two static-diagnostic differences remain original failures; the targets correctly return nonzero. Implementation of the probe lane is complete, while the exact successful-inventory acceptance criterion above is unmet and is not waived by this baseline.
