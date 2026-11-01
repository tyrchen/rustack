# Survey: Step Functions and Container Lambda Compatibility

Status: complete documentation survey; no Docker runtime experiment performed · Date: 2026-10-25 · Consumer: [Workflows PRD](../../specs/rustack-workflows-00-prd.md)

## 1. Question and outcome

Can Rustack interpret ASL itself and run the Lambda images deployed by other projects without depending on another AWS emulator? Yes, provided it implements three distinct contracts: workflow semantics, Lambda execution environments, and the invocation bridge. RIE is not a workflow engine; a successfully started container is not a successful invocation.

The existing [Cargo Lambda spike](./spike-cargo-lambda-runtime-execution.md) validates ZIP/bootstrap/Runtime API behavior. This survey adds official protocol and Docker API documentation. No new upstream implementation source was inspected, so no vendor submodule was added. The current `vendors/localstack` directory is uninitialized and is not evidence of an implementation review.

## 2. Inspected repository facts

| Fact | Evidence | Consequence |
|---|---|---|
| Executor already supports Native/Auto/Squib; Docker is unavailable | [executor/mod.rs](../../crates/rustack-lambda-core/src/executor/mod.rs), [config.rs](../../crates/rustack-lambda-core/src/config.rs) | Add a backend incrementally; retain disabled default |
| Runtime API binds host 127.0.0.1:0 per instance | [runtime_api.rs](../../crates/rustack-lambda-core/src/executor/runtime_api.rs) | Changing the container hostname cannot make that host loopback socket reachable |
| Provider separately rejects Docker | [provider.rs](../../crates/rustack-lambda-core/src/provider.rs:927) | Adding an executor file alone cannot enable invocation |
| Invocation request omits ImageConfig/ephemeral storage, although versions store them | [types.rs](../../crates/rustack-lambda-core/src/executor/types.rs), [storage.rs](../../crates/rustack-lambda-core/src/storage.rs:115) | Capture a complete versioned execution revision |
| Cross-service adapters live in the binary | [lambda_s3_bridge.rs](../../apps/rustack/src/lambda_s3_bridge.rs) | Workflow core should not depend directly on Lambda core |
| Worker lifecycle is independent of snapshot participation | [runtime.rs](../../apps/rustack/src/runtime.rs), [snapshot.rs](../../apps/rustack/src/snapshot.rs) | Register workflow workers, not just an HTTP router |

## 3. Protocol findings

ASL now includes JSONPath, JSONata and variable syntax. The initial release therefore needs an explicit STANDARD + JSONPath profile and precise unsupported-feature diagnostics, rather than an unrestricted ASL claim. [ASL specification](https://states-language.net/spec.html), [workflow types](https://docs.aws.amazon.com/step-functions/latest/dg/choosing-workflow-type.html).

AWS limits definitions to 1 MiB, state payloads to 256 KiB and Standard history to 25,000 events. Inline Map allows up to 40 concurrent iterations. Local memory/concurrency budgets must be separately documented. [Quotas](https://docs.aws.amazon.com/step-functions/latest/dg/service-quotas.html), [Inline Map](https://docs.aws.amazon.com/step-functions/latest/dg/state-map-inline.html).

Ordinary optimized Lambda invocation waits for its synchronous result. Lambda does not support `.sync`, but supports callbacks. Nested Step Functions supports startExecution.sync and .sync:2, with string versus JSON Output respectively. [Integration matrix](https://docs.aws.amazon.com/step-functions/latest/dg/integrate-optimized.html), [Lambda integration](https://docs.aws.amazon.com/step-functions/latest/dg/connect-lambda.html), [nested workflows](https://docs.aws.amazon.com/step-functions/latest/dg/connect-stepfunctions.html).

HTTP success does not establish Task success: Lambda can report FunctionError with HTTP200. Task deadlines differ from Lambda execution deadlines. Current AWS documentation allows explicitly matching States.DataLimitExceeded while excluding it from States.ALL; older statements that it is always uncatchable are insufficient. States.Runtime remains non-retriable. [Error handling](https://docs.aws.amazon.com/step-functions/latest/dg/concepts-error-handling.html).

The Smithy service is com.amazonaws.sfn#AWSStepFunctions, version 2016-11-23, awsJson1_0, SigV4 states. Standard StartExecution is idempotent for a still-running execution with the same name/input; closed or differently supplied input returns ExecutionAlreadyExists. Names remain reserved 90 days after closure. Input is a JSON-bearing string on the wire. [Official model](https://raw.githubusercontent.com/aws/api-models-aws/main/models/sfn/service/2016-11-23/sfn-2016-11-23.json), [StartExecution](https://docs.aws.amazon.com/step-functions/latest/apireference/API_StartExecution.html).

## 4. Image and Runtime API findings

Images must target Linux, implement the Lambda Runtime API and tolerate a read-only root filesystem. ImageConfig overrides ENTRYPOINT/CMD/WORKDIR at the argv level. Resolve image tags to digests on deployment; moving a tag afterward does not update the function. [Image requirements](https://docs.aws.amazon.com/lambda/latest/dg/images-create.html), [ImageConfig](https://docs.aws.amazon.com/lambda/latest/api/API_ImageConfig.html), [UpdateFunctionCode](https://docs.aws.amazon.com/lambda/latest/api/API_UpdateFunctionCode.html).

Runtime API2018-06-01 defines next/response/error/init-error. Current documentation adds an optional Lambda-Runtime-Invocation-Id: supplied echoes must match, while older RICs can omit it. The proposed release covers single-concurrency Event Functions, not Managed Instances. [Runtime API](https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html).

## 5. Docker design evidence and remaining experiment

Docker documents `network=container:<id>` namespace sharing. A small relay can own an environment-local loopback Runtime API and exchange tasks over Docker attach, avoiding host Runtime API publishing. **This full composition is a design inference, not a completed runtime experiment.** P0 must test attach framing/EOF, official RIC behavior, rootless/Desktop limits and cleanup. [Networking](https://docs.docker.com/engine/network/), [rootless](https://docs.docker.com/engine/security/rootless/).

Control transport and handler SDK access are different paths. Docker attach can eliminate host reachability requirements for control traffic; handlers still need an operator-configured reachable gateway endpoint. No published Runtime API port does not mean arbitrary localhost SDK endpoints work inside a container.

## 6. Dependency check

Official documentation returned these versions on 2026-10-25; implementation must recheck them.

| Dependency | Observed version/API | Recommendation |
|---|---|---|
| [bollard](https://docs.rs/bollard/latest/bollard/) | 0.21.1; generated query parameters, API negotiation, standalone pipe feature | Replace the old ~0.18 guidance; initial transport uses pipe only, no remote Docker |
| [winnow](https://docs.rs/winnow/latest/winnow/) | 1.0.4 parser combinators | Parse bounded Path/intrinsic grammar rather than embed a general expression engine |
| Tokio/bytes/ciborium/thiserror/serde/config/uuid | Already present in workspace | Reuse existing dependencies before adding a database or another async runtime |

This survey is not a completed vulnerability audit, MSRV check or release build. New dependencies still require AGENTS.md audit/deny and feature review.

## 7. Risk closure artifacts

P0 must produce `spike-lambda-container-relay.md` for netns/attach/official RIC/native architectures, and `spike-stepfunctions-semantic-oracle.md` for source hashes, SDK versions and semantic fixtures. These are planned artifacts, not existing passing evidence. The implementation plan assigns their exit gates.

## P0 follow-up

The implementation run initialized the existing LocalStack gitlink without moving it, executed the retained relay experiments, and corrected the earlier protocol assumptions. See the [actual relay spike](./spike-lambda-container-relay.md) and [semantic oracle](./spike-stepfunctions-semantic-oracle.md). The original survey's lack-of-runtime-evidence statement is historical; these later memos carry the measured evidence and its limits.
