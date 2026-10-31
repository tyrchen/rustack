# Rustack Workflows: Step Functions and Real Lambda Execution

Status: implementation in progress; full M0–M4 incomplete · Date: 2026-10-31 · Delivery: M0–M4

Real image/SDK/callback/IaC and server-in-container consumers are functionally qualified through D98 on native-arm64 VZ/emulated-amd64 QEMU lanes; see [final D98 qualification](./rustack-workflows-93-review.md#d98-final-qualification-and-exact-postconditions). Workflow/API/structured/nested/callback/persistence and current-host Native qualification are implemented. Default release packaging, native-x86/reference/Desktop/rootless, remaining performance/load and complete M0–M4 acceptance remain open.

## 1. Problem and vision

Other projects need to test real serverless workflows locally and in CI: deploy the same Lambda container images used on AWS, create state machines through ordinary AWS SDKs, exercise branching, retries, timeouts and child workflows, and inspect actual outputs and execution histories. At feature inception, Rustack had a Lambda control plane and Runtime API, explicitly rejected Docker execution and had no Step Functions engine. Returning plausible API responses or echoing inputs cannot validate function code or workflow behavior.

Rustack will own an ASL interpreter and Docker Lambda execution environments, reusing its gateway, Lambda provider, immutable revisions and snapshots. Clients change their AWS endpoint rather than rewrite production workflows. The complete release profile is **STANDARD workflows, an explicit JSONPath subset, all eight state types, real image Lambdas, Retry/Catch/timeouts, Parallel/Inline Map, nested `.sync`, and Lambda callbacks**. No Step Functions Local, LocalStack, Temporal or other replacement workflow engine is required.

## 2. Existing implementation and specification authority

The [research survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md) records inspected code and official sources. These documents describe future work; historical Implemented labels are not new verification evidence.

This specification set:

- Preserves [Lambda hardening](./rustack-lambda-hardening-design.md): execution defaults to disabled, artifacts are immutable, and admission and cleanup remain bounded.
- Supersedes the RIE-forwarding architecture in the [original Lambda design](./ruststack-lambda-design.md), and the old Docker networking, dependency versions and broad cleanup rules in the [executor design](./ruststack-lambda-executor-design.md).
- Preserves Native and Squib as independent backends. Image packages require Docker; they never silently fall back to Native or Squib.
- Extends [runtime hardening](./rustack-runtime-hardening-design.md) worker ordering and [snapshot coverage](./ruststack-snapshot-binary-archive-design.md). It does not add a crash-consistent execution log.

## 3. Users and measurable goals

Primary users maintain Rust, Python or Node Lambda projects with CDK/Terraform/Pulumi-generated ASL. CI maintainers need repeatable Linux execution without cloud credentials. Apple Silicon developers need to execute Linux arm64 images without rebuilding handlers for macOS.

| ID | Goal | Acceptance measure |
|---|---|---|
| G1 | Execute real images | Results originate inside the deployed handler; broken images, missing RICs and initialization failures fail explicitly, without echo fallback |
| G2 | Work with ordinary SDKs | Rust SDK and boto3 create/start/describe/history/stop scenarios pass without Rustack-only required wire fields |
| G3 | Make core ASL trustworthy | Every required case in the [verification matrix](./rustack-workflows-70-verification-plan.md) passes |
| G4 | Implement actual `.sync` | Parents await child termination; `.sync` and `.sync:2` preserve different Output types; invalid `lambda:invoke.sync` is rejected during creation |
| G5 | Reuse deployment artifacts | Native-architecture Linux x86_64, Linux arm64 and macOS arm64 + Docker Desktop lanes pass; the prepared example completes within five minutes excluding builds and pulls |
| G6 | Bound and clean up work | Overload is explicit; Stop, timeout and shutdown leave no unowned tasks or containers; cleanup failures remain tracked and diagnosable |
| G7 | Expose compatibility honestly | Unsupported syntax and integrations fail validation at a precise location; capabilities distinguish compiled, enabled, probed and limited behavior |

Performance numbers are acceptance budgets, not existing measurements.

## 4. Release scope

| Capability | Milestone | Authoritative design |
|---|---|---|
| Image Lambda deploy/update/get, digest pinning, ImageConfig, versions, actual Invoke | M0 | [Lambda runtime](./rustack-workflows-20-lambda-runtime-design.md) |
| STANDARD APIs, Pass/Task/Choice/Wait/Succeed/Fail, data transformations, Retry/Catch | M1 | [API](./rustack-workflows-10-api-design.md), [ASL](./rustack-workflows-30-asl-design.md) |
| Parallel, Inline Map, nested startExecution/.sync/.sync:2 | M2 | [ASL](./rustack-workflows-30-asl-design.md), [integrations](./rustack-workflows-40-integrations-design.md) |
| Lambda `.waitForTaskToken`, SendTask* and heartbeats | M3 | [integrations](./rustack-workflows-40-integrations-design.md) |
| Container execution of provided.al2023 ZIPs, snapshots, platform and fault validation | M4 | [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md) |

M0 is the smallest useful end-to-end slice: SDK → Lambda API → Docker handler → SDK result. M1 first closes SDK → ASL → Docker Lambda → history. **The complete requested outcome is M4**, not an M0/M1 scaffold.

Excluded from this release: EXPRESS/StartSyncExecution, JSONata/Assign/variables, Distributed Map, Activities, Redrive/TestState, Step Functions version publication/aliases (the unpublished empty-version inventory read is included for IaC), cross-account AssumeRole, arbitrary AWS SDK integrations, ECS/Batch/Glue `.sync`, an ECR emulator, user-image builds, VPC emulation, extensions, SnapStart, streaming responses, Managed Instances, an AWS IAM policy evaluator, and cloud-grade durable workflow recovery.

Each exclusion requires explicit rejection or an advertised limitation, rather than storing ignored fields and reporting success. An eventual ECS `.sync` adapter requires a real ECS execution service; starting an arbitrary Docker container does not establish ECS compatibility.

## 5. User experience

Use the existing configuration entry point: `RUSTACK_CONFIG=./rustack.yaml rustack`. Enable `SERVICES=lambda,stepfunctions` and explicitly choose `LAMBDA_EXECUTOR=docker`. The consuming project builds its images beforehand. Local mode accepts an image such as `rustack-echo:integration`; an unchanged ECR reference must already exist in the daemon or match an operator-enabled registry pull policy.

```json
{
  "StartAt": "Call",
  "States": {
    "Call": {
      "Type": "Task",
      "Resource": "arn:aws:states:::lambda:invoke",
      "Parameters": {
        "FunctionName": "arn:aws:lambda:us-east-1:000000000000:function:echo",
        "Payload.$": "$"
      },
      "TimeoutSeconds": 15,
      "Retry": [{"ErrorEquals": ["Lambda.TooManyRequestsException"], "MaxAttempts": 2}],
      "OutputPath": "$.Payload",
      "End": true
    }
  }
}
```

The SDK sequence is CreateFunction(PackageType=Image, Code.ImageUri=...) → wait for Active → CreateStateMachine(type=STANDARD) → StartExecution(input is a JSON string) → poll DescribeExecution → GetExecutionHistory. Production ASL needs no special testing fields. Ordinary `lambda:invoke` already waits for function completion; adding `.sync` is invalid.

## 6. Compatibility declaration

Unmodified production ASL works within the documented profile. Modern CDK output using JSONata or other integrations receives an explicit rejection. Resource names initially accept ASCII `[A-Za-z0-9_-]`, an advertised local restriction. Authentication follows Rustack's gateway trust modes; execution role ARNs are validated metadata, not evidence of AWS IAM authorization parity.

Standard execution guarantees apply within a live Rustack runtime. Snapshots do not recover requests made after the last completed save or ensure exactly-once external side effects.

## 7. Reading order and cross-references

Read 00 → 10 → 20 → 30 → 40 for contracts. Read 60/70 alongside them. Document 80 describes user-visible delivery, 90 specifies engineering dependencies, and 99 records decisions.

- → [API and data model](./rustack-workflows-10-api-design.md)
- → [roadmap](./rustack-workflows-80-roadmap.md), [implementation plan](./rustack-workflows-90-impl-plan.md)
- ↔ [key decisions](./rustack-workflows-99-decisions.md)
