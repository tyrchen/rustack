# Local smoke tests

Start a fresh server from the current checkout with all default services enabled:

```bash
make run
```

In a second terminal, run:

```bash
make smoke-services     # SDK/HTTP functional assertions, no Pulumi dependency
make smoke-regressions  # Workspace regressions plus real native Lambda execution
make smoke             # Both checks above, followed by Pulumi provisioning/cleanup
```

`smoke` requires Cargo, curl, Node.js/npm, and the Pulumi CLI. The server must
already be running. Start it from the current checkout to test the current
code rather than a stale binary. For a separate local instance:

```bash
# Terminal 1
GATEWAY_LISTEN=127.0.0.1:4588 make run
# Terminal 2
make smoke SMOKE_ENDPOINT=http://127.0.0.1:4588
```

Use a disposable local instance: these tests create and modify resources.
Individual existing integration tests do not all enforce cleanup, and KMS key
deletion is scheduled rather than immediate. Restart the in-memory server to
discard remaining fixtures. The runner does not stop a server it did not start.

## Functional coverage

The Makefile contains an explicit manifest of 30 exact test names. The runner
checks every name against the compiled test list before executing anything, so
a renamed or removed test fails instead of producing a zero-test success.
Tests run sequentially; any failed assertion stops the target.

| Service | Asserted behavior |
| --- | --- |
| S3 | Write/read identical object bytes, list objects, delete object |
| SSM | Set/get parameter value, overwrite value and increment version |
| DynamoDB | Put/get item and query partition/sort keys |
| DynamoDB Streams | Table write produces a readable INSERT record |
| SQS | Send/receive message and delete message |
| SNS | Publish a message and receive it through an SQS subscription |
| EventBridge | Matching event is delivered to an SQS target |
| Kinesis | Put record and read its data through a shard iterator |
| KMS | Symmetric encrypt/decrypt, secp256k1 sign/verify, reject invalid DIGEST lengths |
| Secrets Manager | Create/read secret value and write a new version |
| CloudWatch Logs | Put/get log event content |
| CloudWatch Metrics | Put metric and retrieve statistics |
| SES | Verify sender and submit email; assert returned message ID |
| IAM | Create/read role metadata |
| STS | Get and assert caller identity |
| Lambda | Create/read function, DryRun, S3 code create/update with hash/size readback, missing S3 bucket error |
| API Gateway V2 | Execute a configured MOCK integration through HTTP |
| CloudFront | Retrieve S3 content through a distribution; config read/update/disable/delete with correct geo restriction XML |

SES checks local API acceptance, not external mail delivery. IAM checks metadata,
not production policy enforcement. Lambda's live-server cases do not execute
function code; `smoke-regressions` also runs the existing native executor tests,
which execute a real bootstrap through the provider/runtime API and bypass the
public SDK/HTTP gateway. API Gateway's selected case is MOCK, not Lambda proxy.
This suite checks representative workflows, not every supported operation.

## Recent change coverage

`smoke-regressions` runs `cargo test --workspace`, including existing unit,
fixture integration, and documentation tests, then
`make test-lambda-invoke-native`. Server-dependent ignored cases are selected
separately by `smoke-services`.

| Recent change | Check included in the smoke workflow |
| --- | --- |
| KMS secp256k1 (#55) and wrong-length DIGEST errors (#58) | New live SDK sign/verify test, invalid DIGEST rejection for Sign/Verify with key ID and ARN; existing crypto/dispatch regressions |
| SigV4 normalized queries (#54) | Auth canonicalization, signed/presigned verification, duplicate/UTF-8 validation and plus-sign tampering regressions |
| CloudFront query/XML decoding (#56) | HTTP dispatcher and XML parser regressions for decode-once, entities, malformed UTF-8/XML and non-destructive rejection |
| Lambda S3 code packages (#51) | Live create/update with changed hash/size readback and missing-bucket cases; provider regressions and real native invocation with S3 code |
| System hardening (#52) | Workspace regressions for configuration, resource limits, data correctness, worker lifecycle, snapshot publication/recovery and immutable/warm Lambda code |
| Release/dependency updates | Build/test the current workspace; run Pulumi with the example's locked npm dependencies |

The manifest should gain a representative SDK/HTTP case when a new service
operation or workflow is introduced. Keep lower-level rejection and lifecycle
regressions in the owning crate so the regression stage includes them automatically.

## Pulumi and persistence

The final stage runs [the Pulumi smoke](./pulumi.md) against the same endpoint.
Both provisioning and cleanup errors fail the target; cleanup failures retain the
local Pulumi state directory and print its path for inspection. The examples use
bounded provider retries (`maxRetries: 3`) for transient connection errors. Pulumi SQS deletion may
wait around two minutes; the workspace regression stage also makes `smoke`
slower than the 30-case `smoke-services` subset.

Live snapshot restart checks remain separate:

```bash
make pulumi-hackathon-snapshot-smoke
make pulumi-commerce-snapshot-smoke
```

Those targets start/stop their own instances and verify persisted data; the
commerce case also checks restored CloudFront cache hits. They are not run by
`make smoke`.

The expanded cleanup checks exposed an invalid CloudFront geo restriction XML
shape. `Quantity` now appears directly under `GeoRestriction`, and `Items`
contains only `Location` elements, as required by the
[AWS API schema](https://docs.aws.amazon.com/cloudfront/latest/APIReference/API_GeoRestriction.html).
The serializer regression checks populated locations; the live SDK smoke checks
empty locations and the disable-before-delete lifecycle.

The `pulumi-test` CI workflow runs the regression, Pulumi, and service stages
against its own freshly built server and stops that server in an always-run step.
