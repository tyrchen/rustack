# Spike: Independent Step Functions Wire and Semantic Evidence

Status: PASS with a confirmed generator remediation · Date: 2026-10-25 · Consumer: [API](../../specs/rustack-workflows-10-api-design.md), [ASL](../../specs/rustack-workflows-30-asl-design.md), [integrations](../../specs/rustack-workflows-40-integrations-design.md)

## Question and method

Can the selected STANDARD/JSONPath contracts be pinned to independent sources and ordinary SDK behavior before writing the engine?

The [fixture catalog](../../tests/workflows/semantic-cases.json) contains 48 document-derived expectations, not interpreter test results. [Provenance](../../tests/workflows/provenance.json) pins SDK/dependency versions. Four [offline botocore tests](../../tests/workflows/test_botocore_wire.py) exercise its serializer/parser without creating a client, resolving credentials or contacting AWS. Run `make workflows-fixture-check workflows-wire-probe`.

The [SFN Smithy model](../../codegen/smithy-model/stepfunctions.json) is pinned to aws/api-models-aws commit `18e485ce1c8dbf29d47e5ce00d9b8513da95912c`, SHA-256 `38dd64c074f5c05320978970dafa6e0f199ab2e33b6065232002e8a603c5f6ed`, 370,185 bytes. `make workflows-model-check` verifies its hash, protocol, signing name and 37-operation service. `make workflows-codegen-spike` independently generates six Rust files into `/tmp/rustack-spikes/stepfunctions-generated/src` from 312 shapes.

The existing LocalStack submodule was initialized at its already-pinned commit `45dfee8ce27df00fcac3f91b8071c8156eb5c5c7`; its gitlink was not changed. Only relevant test/snapshot evidence was inspected, not the full interpreter architecture. No implementation code was copied.

## Findings

| Evidence | Result and binding consequence |
|---|---|
| botocore 1.43.118 / boto3 1.43.118 | StartExecution emits AWSStepFunctions.StartExecution and preserves the raw input JSON string |
| SDK numeric timestamp parsing |0.125→1970-01-01T00:00:00.125Z;−0.125→1969-12-31T23:59:59.875Z. HTTP timestamps are numeric fractional epoch seconds |
| Current generated CreateStateMachineOutput | A standalone Rust test confirms numeric creationDate fails ordinary chrono deserialization. Generation succeeds, but its current wire codec is insufficient |
| Callback error parser and current API docs | Known closed/expired tasks use TaskTimedOut; unknown/malformed tokens use InvalidToken. TaskDoesNotExist describes a nonexistent activity |
| Current AWS error guide | States.ALL excludes Runtime/DataLimitExceeded; explicit DataLimitExceeded matching is supported. Older upstream expectations cannot override this current documented contract |
| AWS-validated nested snapshots | `.sync` has string Input/Output; `.sync:2` has JSON Input/Output. Both include a TaskSubmitted event between TaskStarted and terminal completion; optimized null Input normalizes to an empty object |

Nested evidence is directly inspectable in the [validated tests](../../vendors/localstack/tests/aws/services/stepfunctions/v2/callback/test_callback.py) at lines 285/333; paired output shapes are in [snapshots](../../vendors/localstack/tests/aws/services/stepfunctions/v2/callback/test_callback.snapshot.json) at lines 713/1251, with validation dates 2023-06-30 and2023-08-12 in the [validation record](../../vendors/localstack/tests/aws/services/stepfunctions/v2/callback/test_callback.validation.json). Snapshot timestamps are normalized and do not establish a date unit. A [historic direct AWS execution report](https://github.com/aws/aws-cdk/issues/5474) records optimized nested dates as epoch milliseconds; this is corroborating historical evidence, not a current cloud run. The local contract explicitly selects milliseconds for optimized envelopes while HTTP remains epoch seconds.

Primary references: [Smithy JSON timestamp serialization](https://smithy.io/2.0/aws/protocols/aws-json-1_0-protocol.html#shape-serialization), [AWS callback errors](https://docs.aws.amazon.com/step-functions/latest/apireference/API_SendTaskSuccess.html), [error handling](https://docs.aws.amazon.com/step-functions/latest/dg/concepts-error-handling.html), [nested integrations](https://docs.aws.amazon.com/step-functions/latest/dg/connect-stepfunctions.html). Individual cases identify their sources.

## Decision

**GO with concrete remediation:** keep the API/profile and correct the reviewed contracts before consumers. P2 task2.1 must add protocol-aware numeric timestamp generation and required/optional/fractional/negative round-trip tests before the model becomes a production crate. Do not hand-edit generated output or copy the existing KMS timestamp adapter's unchecked negative-fraction conversion. The transformation and intrinsic expectations remain independent fixtures for the future interpreter; passing catalog/SDK checks does not mark A04–A11 complete.

## Evidence limits

No live AWS differential suite was run, and no complete emulator was used as a semantic oracle. Historical AWS-validated snapshots supplement current official documentation only where they do not conflict. Current guide-compatible States.Runtime behavior is selected for unevaluable paths; internal Path/Intrinsic/ResultPath diagnostics remain separate. Native x86 hardware, Docker Desktop/rootless qualification, full execution-history parity and the final performance gates remain unverified.
