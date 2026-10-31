# Workflows 80: User-Facing Delivery Roadmap

Status: M0 functionally qualified on retained P0 lanes; full M0–M4 release incomplete · Date: 2026-10-31

Current checkpoint: P1/M0 passes L01–L09 on the retained macOS arm64 host/native-arm64 VZ and emulated-Linux-amd64 QEMU Docker lanes. D97's current full gate passes72 exact resources/521 Rust cases per lane, including real Node24, S3/DynamoDB SDKs, image/ZIP archives, all nine image callback modes and managed Pulumi lifecycle/active-plugin Stop. The workflow/API/structured/nested/callback/persistence features and current-host Native qualification are implemented. Exact postconditions prove zero remnants and thirteen source IDs preserved per engine. See [final D97 qualification](./rustack-workflows-93-review.md#d97-final-qualification-and-exact-postconditions). Server-in-container, native-x86/Linux reference performance, Desktop/rootless and the complete M4 platform manifest remain required; this is not complete release qualification.

## 1. Delivery principles

Each milestone is usable end to end and preserves bounded execution, honest errors and explicit capability reporting. Unfinished capabilities remain rejected. M0 is useful independently, but the requested workflow/runtime feature is complete only after M4. The [implementation plan](./rustack-workflows-90-impl-plan.md) orders engineering work by dependency; this roadmap groups outcomes a consuming project can use.

```text
┌─ P0: evidence, contracts and risk retirement ───────────────────────┐
│ official RIC + Docker relay proof │ semantic fixtures / model pin │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M0: run real image Lambdas ────────────────────────────────────────┐
│ deploy → immutable digest/version → actual Invoke → cleanup        │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M1: test sequential serverless workflows ─────────────────────────┐
│ SDK → ASL/data flow → Docker Lambda → Retry/Catch → history         │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M2: coordinate fan-out and nested jobs ────────────────────────────┐
│ Parallel/Inline Map │ child execution │ .sync/.sync:2 cancellation  │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M3: test callback workflows ──────────────────────────────────────┐
│ Task.Token → real Lambda → SendTask* → heartbeat/deadline           │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M4: use in consuming-project CI ──────────────────────────────────┐
│ provided ZIP │ snapshots │ platform matrix │ fault/load/IaC gates  │
└───────────────────────────────────────────────────────────────────┘
```

## 2. Milestones

| Milestone | What another project can do | Exit evidence | Engineering phase |
|---|---|---|---|
| M0 — Actual image execution | Create/update an Image Lambda, publish/alias it and receive the handler's actual result using a normal SDK | L01–L09; official RIC and custom Rust fixture; digest/version isolation and owned cleanup | P1 |
| M1 — Sequential workflows | Create STANDARD definitions and test six state types, transformations, real Lambda tasks, retries, catching, waiting and timeouts | A01–A13, I01; Rust SDK+boto3; PRD scenario succeeds and deliberate failures fail | P2 |
| M2 — Fan-out and nested completion | Execute all eight state types, bound Inline Map concurrency, wait for real child workflows and stop owned subwork | I02–I05, P01–P03; correct order/types and no missed completion/deadlock | P3 |
| M3 — Callback workflows | Pass Task.Token to a real function and complete/fail/heartbeat its task safely | C01–C03; early callbacks, retries and revoked tokens covered without token disclosure | P4 |
| M4 — Reusable local/CI dependency | Use supported image/ZIP artifacts, snapshots and a basic IaC lifecycle across the platform matrix | L02 completed, L10/L11, O01–O05; complete verification manifest passes without required skips | P5 |

All milestone evidence refers to the [verification plan](./rustack-workflows-70-verification-plan.md). Each exit also includes the applicable repository quality gates and fresh capability output. A demo without negative cases does not close a milestone.

## 3. Calendar and confidence

Assume one experienced Rust engineer, a working Docker environment, preexisting gateway/Lambda hardening, no distributed persistence requirement and access to both Linux architectures plus Docker Desktop. Estimate 56–83 engineering days including P0, then add 25% for review, integration and platform troubleshooting: approximately **14–21 calendar weeks**. These are planning estimates based on identified work, not measured delivery velocity.

| User-visible checkpoint | Approximate cumulative calendar window |
|---|---|
| M0 | Weeks 4–6 |
| M1 | Weeks 8–12 |
| M2 | Weeks 10–15 |
| M3 | Weeks 12–18 |
| M4 | Weeks 14–21 |

P0 is the first re-estimation point. If relay portability fails, or semantic evidence forces a broader expression engine, revise this roadmap before implementation commitments. Image execution, ASL semantics and shutdown correctness are substantive work, not a small API addition. Optional future scope—EXPRESS, JSONata, ECS/Batch, WAL recovery—has no implicit delivery date in this estimate.

## 4. Cross-references

- ← [PRD](./rustack-workflows-00-prd.md)
- ↔ [implementation plan](./rustack-workflows-90-impl-plan.md), [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)
