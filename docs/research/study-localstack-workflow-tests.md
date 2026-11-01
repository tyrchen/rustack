# Study: Running LocalStack workflow tests against Rustack

Date: 2026-11-01. Vendor: `vendors/localstack` at `45dfee8ce27df00fcac3f91b8071c8156eb5c5c7`. Scope: external workflow compatibility evidence, continuing P5. No live AWS execution is implied.

## Decision

Run original upstream pytest methods with original AWS snapshots, using a separate fixture plugin and explicitly selected node IDs. Do not extract expected values into new Rustack-authored assertions, replace the ASL evaluator, or import the upstream root conftest. Keep the vendor pin unchanged.

```text
┌─ Fixed upstream inputs ───────────────────────────────┐
│ pytest methods │ JSON5 templates │ AWS snapshot files │
└─────────────────────┬────────────────────────────────┘
                      ▼
┌─ Rustack-owned test boundary ─────────────────────────┐
│ exact node inventory + provenance                     │
│ endpoint/client + resource lifecycle fixtures         │
│ private snapshot copies; update/skip-all forbidden    │
└────────────┬──────────────────────────┬───────────────┘
             ▼                          ▼
     Signed boto3 requests      Upstream snapshot verifier
             │                          ▲
             ▼                          │
     Actual Rustack gateway ── observed responses
             │
             ▼
     Owned real Docker Lambda, where selected
```

## Relevant upstream boundaries

- `tests/conftest.py:10` registers plugins that can start LocalStack. Bypass it with pytest's `--noconftest`, disable automatic plugin loading, and load only the compatibility and snapshot plugins.
- `localstack-core/localstack/testing/pytest/stepfunctions/fixtures.py:24` adds the upstream Step Functions transformers to the snapshot session. Preserve those transformers and the method-level transformations.
- `localstack-core/localstack/testing/pytest/stepfunctions/fixtures.py:179` creates IAM roles; Rustack treats execution-role ARNs as validated metadata. The adapter supplies a fixed local role only for tests that do not assert IAM behavior.
- `localstack-core/localstack/testing/pytest/stepfunctions/fixtures.py:273` creates machines and later stops executions/deletes machines. The adapter must retain those resources immediately and propagate cleanup failures rather than logging and continuing.
- `localstack-core/localstack/testing/pytest/stepfunctions/utils.py:576` drives actual Create/Start/Describe/History requests. Reuse it unchanged; this is an external client, not the LocalStack interpreter as Rustack's executor.
- `tests/aws/services/stepfunctions/v2/intrinsic_functions/utils.py:14` executes each original input and snapshots full history. `choice_operators/utils.py:79` constructs definitions and snapshots input/output pairs. Their differing assertion scopes must remain visible.
- `tests/aws/services/lambda_/test_lambda_api.py:1699` uses ECR-created alpine/debian images for metadata CRUD. These are not interchangeable with real-RIC invocation fixtures: unsupported ECR/packaging assumptions require an explicit exclusion rather than silently mapping arbitrary image requests to another image.

The pinned snapshot0.3.4 plugin calls its persistence hooks, but baseline/raw writes occur only in update/raw modes; ordinary verification does not generate a validation file. Override its session fixture to copy the original snapshot into a private evidence directory, retaining the original node ID, and save our own observations/assertions there. Never point update/raw persistence at the vendor tree. Record hashes before and after, including templates and handler sources used by the selected tests.

## Adoption rules

Preserve test bodies, parameterization, expected snapshots, and existing upstream normalization/exclusion markers. Reject snapshot-update, snapshot-raw, skip-all, unexpected collection changes, xfail and runtime skips in the required inventory. Do not add masks after observing a semantic mismatch. Separate intentional profile exclusions from compatibility failures.

For real handler integration, preserve original handler source. Building that source into a preprovisioned Python RIC image is a declared packaging adaptation; it does not establish managed Python ZIP runtime parity. Callback handlers must use explicit local service endpoints and credentials. No request may fall back to a real AWS endpoint.

The image runtime remains Python3.12 to match upstream requests. An explicit read-only ECR manifest lookup on2026-11-01 UTC pins its Linux arm64 manifest to `ab3f7e33c838a04d8750648f4cd1250ef8c20f6137904a3bbccd90da95e4157a`. The [official AWS image documentation](https://docs.aws.amazon.com/lambda/latest/dg/python-image.html) describes the packaged RIC/AL2023 runtime. Each original handler receives its own image at `/var/task/handler.py`, preserving module/traceback names. Buildx metadata binds either the OCI image digest or configuration digest to the observed daemon-addressable ID, without assuming quiet stdout has the same identity. Fixed recipe/source/intent/metadata/result hashes are checked before execution.

## Evidence limits

The existing semantic-oracle memo inspected selected upstream observations but did not execute this suite. A new run reports original pytest node IDs, AWS snapshot recording dates, exact collection and outcomes, source/fixture/package hashes, and physical cleanup. This establishes compatibility for the selected inventory only. Desktop, dedicated reference, broader AWS functionality and final release acceptance remain separate requirements.


## First execution findings

Collection is a separate gate before the server starts. The pinned snapshots contain removed class/method IDs and four removed Lambda parameter IDs (`None`/`json_value7` in direct and optimized JSON-value methods). The actual selected inventory is91 pure and32 Docker nodes. Obsolete IDs remain documented in the manifest; no executed failing case was removed.

Adapter faults were fixed before interpreting product outcomes: use the original AWS snapshot fixture's base transformations and extended transformer utility, and accept the role factory's original keyword argument. Source drift, muted assertions, altered image-build receipts and failed API cleanup remain failing controls.

Original full histories exposed real profile corrections: initial predecessor0, captured RoleArn, serial Map admission from the prior processor terminal, shared Map success/exit predecessor, and suppression of retried Parallel failure markers. The original Parallel transformer itself removes the success marker and normalizes branch event IDs; its passing assertions establish the preserved exit predecessor and remaining assertions, not complete graph parity. Failed ResultSelector handling remains a separately tested local policy.

Three pure methods mix intentional formal-profile restrictions into otherwise overlapping scenarios: context-object Format substitutions, unescaped literal braces in a nested StringToJson call, and empty/whitespace StringToJson input. The initial run also exposed valid integer -0 becoming float -0.0; W444 fixes that independently, including nested values and quoted/exponent preservation. The [formal ASL rules](https://states-language.net/spec.html) prohibit object/array Format Path substitutions and require reserved braces to be escaped; StringToJson applies a JSON parser. Keep those original failures visible while the scope decision is pending; no new ignore path or xfail is introduced.

Real RIC execution additionally exposed optimized Payload double encoding/null defaults, function-error Cause whitespace, optimized service-failure event ordering, and optional timeout fields. Correct these in production rather than altering fixtures. The original optimized-class HTTP metadata exclusions remain the upstream markers. Static redacted service Cause text and unsupported redrive metadata remain distinct from these in-profile corrections.


## Reproduction commands

`make workflows-localstack-python` provisions the exact isolated Python dependencies plus the editable pinned upstream import package. `make check-workflows-localstack` runs integrity controls and strict Clippy on the new Rust owner. `make workflows-localstack-images WORKFLOWS_LOCALSTACK_IMAGES=<new-absolute-directory> WORKFLOWS_LOCALSTACK_BUILDER=<absolute-buildx> WORKFLOWS_DOCKER_SOCKET=<explicit-socket>` builds the six original handlers with immutable receipts. Evidence directories must be new, canonical and private.

Run `make test-workflows-localstack-pure WORKFLOWS_LOCALSTACK_EVIDENCE=<new-absolute-directory>`. For actual Docker, use `make test-workflows-localstack-docker WORKFLOWS_LOCALSTACK_EVIDENCE=<new-absolute-directory> WORKFLOWS_LOCALSTACK_IMAGES=<image-receipt-directory> WORKFLOWS_DOCKER_SOCKET=<explicit-socket> LAMBDA_RUNTIME_IMAGE=<qualified-relay-image>`. Targets retain nonzero failure status for any original assertion mismatch, including a documented profile difference; classification never turns a failed case green. These opt-in targets do not start LocalStack or call live AWS.


The original `TestTimeouts` class excludes `$..redriveCount` and `$..redriveStatus`. Thus its now-passing global-timeout snapshot verifies the remaining DescribeExecution fields, including error/cause omission, but does not establish redrive behavior. These paths originate upstream and are recorded in each assertion receipt; the adapter adds none.


## Final source-cohort verification

Frozen source `c16d68e41d4ee7177adcf895fec201f3d0269d796a17058aae026fa7778aa282` covers1232 execution/tooling files. The actual standalone server binary is SHA256 `88f70bace2a1acb2c0927a3f0b36f53265901f742dc27620d57faae87c387ae8`. The manifest SHA256 is `b4d9750a9fe868cb60c1f6ebb696405b3976ff05214872a56044af36bf3d6fdb`, binding42 selected upstream files plus the unchanged vendor commit.

| Lane | Result | Retained evidence below `/Users/tchen/.target/` | Report SHA256 |
|---|---|---|---|
| Rust | All15 gates pass; all child groups joined | `workflows-localstack-rust-sixth` | `6fc6bb81a4e2c471acc22b94b8446eaa3a45a54d4497f5949c4acb115a1d65fb` |
| Native / Action |3121 Rust passes in41 summaries;19 explicitly gated resources;26 actual Action Python controls pass; both commands joined | `workflows-localstack-native-fifth` | `edab09dd163d608f2b43911d4c26234377db9144c073741e213d28644eb363a7` |
| Original pure pytest |88/91 pass; strict target exits nonzero | `workflows-localstack-pure-tenth` | `e4cb7cddf6620463860e63407b48740d87a557fe5b58d06cd697af47ee1f5f68` |
| Original Docker pytest |30/32 pass; strict target exits nonzero | `workflows-localstack-docker-eighth` | `7fc2f0882143cf15a760386a0ecffea00dc3a80533c4ba8e374f41cdd7f20c93` |

The13 adapter controls and strict Clippy for the new owner pass via `make check-workflows-localstack`; raw output is `/tmp/rustack-localstack-controls-seventh.log`. Strict production-core/library checks and the SDK's existing pedantic gate pass. The final core library has937 passing unit tests; the targeted actual service SDK run passes10/10 before the complete Native rerun. All current-source reports share the same source identity. This is an implemented external probe lane with measured compatibility gaps, not successful strict-compatibility acceptance.

Both external server cleanup receipts confirm successful server exit, joined capture tasks, unchanged binary and absent owned resources. Docker additionally performs the real namespace audit; pure execution does not claim a Docker audit. Every node's setup/teardown passes; there are no xfails or skipped selected nodes. Baseline copies and upstream bytes remain unchanged. Packaging is original Python3.12 handler source in six real RIC images, not managed ZIP/ECR lifecycle equivalence.

The Docker engine is the task-owned native ARM64 VZ profile `rustack-localstack-compat-arm64`, ID `575aa85f-ebb2-4150-a089-201e40f4d035`, with an explicit Unix socket. Existing stopped Rustack VMs were not restarted, the unrelated `reshape-build` profile was untouched, and the default context was not activated. The immutable relay import retains its original archive/ID and import receipt. The six-handler image receipt `workflows-localstack-images-second/images.json` has SHA256 `e8e920ee90dac79b5321115b2efd04574d9113c07e3255ad4d508807317d7edb`; build recipes, original handler hashes, metadata/results and live image IDs remain bound to that engine. Images and private evidence are retained for review; no prune or foreign-resource deletion occurs.

### Remaining original assertions

| Original class/method | Recorded mismatch | Disposition |
|---|---|---|
| `TestGeneric::test_context_json_path` | Object substitution in Format expects a historical map-like string | Preserve formal scalar-only Format profile |
| `TestJsonManipulation::test_json_merge_escaped_argument` | Unescaped reserved literal braces accepted by the historical oracle | Preserve formal intrinsic escaping validation |
| `TestJsonManipulation::test_string_to_json` | Empty and whitespace-only JSON become null in the historical oracle | Preserve strict JSON parsing; all eight other inputs, including integer -0, match |
| `TestTaskServiceLambda::test_no_such_function` | AWS SDK-style Cause text versus the bounded static local service cause | Error class and complete event sequence match; exact diagnostic text remains different |
| `TestTaskServiceLambda::test_no_such_function_catch` | Same Cause text propagated through Catch | Preserve the original failed assertion and local diagnostic policy |

These five selected methods remain in the exact inventory and still fail their original assertions. No new mask, regenerated baseline, xfail or post-failure removal is introduced. They are118 passing nodes out of123 selected nodes, not a96% project-completion or full-AWS-coverage claim. The optional scope question received no answer during execution, so the existing formal profile remains unchanged. Expanding historical behavior requires a separate explicit profile decision; this record does not waive design61's strict successful-inventory exit criterion.

### Failed attempts and interpretation

All earlier directories and raw logs retain their original contents. Pure-first rejects stale snapshot node IDs in preflight; pure-second exposes the adapter's missing AWS transformer fixture; pure-third passes37/91, pure-fourth78/91, and pure-fifth through ninth88/91. Docker-first rejects four removed parameter IDs before server creation; Docker-second passes13/32, and Docker-third through seventh30/32. These outcomes are neither erased nor restamped with the final source.

Rust-first and second deliberately retire after six passing gates to finish independent-review/strict-lint corrections; Rust-third through fifth pass all15 under their original source seals. Native-first fails the expanded start function's strict length lint; Native-second finds the superseded optimized Payload oracle; Native-third finds stale root-entry/timeout SDK oracles; Native-fourth reaches a strict documentation lint in the new helper comment. W438–W446 and the comment are corrected before the final frozen rerun. Direct broad Python discovery reports284 cases with17 missing-Action-fixture setup errors; the actual Action owner subsequently runs and passes all26 controls with its required fixture wiring. An exploratory extra-boundary SDK Clippy invocation exposes existing fixture indexing lints; the repository's required pedantic SDK gate passes without adding suppressions.

Earlier complete83/83 rootful/rootless matrices belong to the previously recorded implementation source and remain historical component qualification. They were not rerun as current-source full matrices here. Docker Desktop, dedicated physical ARM reference and complete same-commit M4 release acceptance remain open; D100's further-x86-execution waiver remains binding. No live AWS execution, remote CI result, release publication or merge is claimed.
