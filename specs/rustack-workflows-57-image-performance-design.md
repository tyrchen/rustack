# Image Lambda Performance and Repeated Cleanup

Status: implemented and independently reviewed; final seventh-source current-profile host15/full ARM79/direct Linux3/postcondition qualification passes. Dedicated-reference/Desktop/rootless/W373 and full M4 remain open.

Phase:P5/5.3. Decision:D102. Apply [verification§5](./rustack-workflows-70-verification-plan.md#5-performance-and-resource-acceptance), [D100](./rustack-workflows-99-decisions.md#d100--linux-x86_64-validation-is-waived) and [D101](./rustack-workflows-70-verification-plan.md#52-direct-linux-arm64-pure-workflow-measurements-d101). This design does not waive dedicated-reference or Desktop/rootless requirements.

## 1. Scope and ownership

Qualify three real signed ordinary-SDK workloads:100 warm calls,20 preloaded cold environments and100 successful-invoke/handler-timeout cycles under active log flood. Reuse the existing production startup, Docker provider/pool/environment, independent Rust RIC image, Python SDK Matrix/metadata oracle, registered image SDK owner and unchanged Rust Runner. Add no benchmark crate, AWS API field, initialization hook or alternate executor. The operational workers diagnostic adds optional runtimeTasks, read from the actual Tokio handle metrics; omit it outside a runtime. It carries only a bounded count, no task identities or request data. The controller and actual CLI must additionally run directly inside the native arm64 Linux guest; a macOS controller/server capture alone cannot establish Linux execution.

The direct Linux lane uses the retained D1014-vCPU/8GiB VZ profile with read-only source/target mounts and independently bounded guest/host owners. Four virtual CPUs do not establish physical dedication: every report retains referenceQualification=pending. Do not run x86_64 validation. Preserve historical D74/D99/D101 evidence.

```text
┌─ explicit Make: same-source ELF build + preloaded local images ───┐
│ exact test inventory → original host/guest process owners         │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
┌─ native arm64 Linux guest ───────────────────────────────────────┐
│ Python SDK → actual CLI → provider/pool → relay + function/RIC    │
│ warm100 / fresh20 /100×(success → log flood + timeout → absence)  │
│ independent daemon/owner metadata + live health/admission probes │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
 successful child join → clean server shutdown → namespace audit
 → bounded raw report + artifact/source/resource postconditions
```

## 2. Clock and initialization diagnostics

Use monotonic integer nanoseconds in the SDK and handler. Round reported durations up to whole microseconds. For warm overhead, subtract handler nanoseconds from the complete signed Invoke/response-body nanoseconds before rounding; reject handler time greater than the enclosing observation. Do not subtract separately rounded samples, interpolate percentiles or exclude slow observations. Record all ordered raw end-to-end/handler/overhead samples. Nearest-rank p50/p95/max use ranks50/95/100 for100 samples,10/19/20 for20 samples. Enforce warm overhead p95≤50000µs and cold end-to-end p95≤5000000µs.

At successful Image environment initialization emit one structured INFO event on the dedicated rustack_lambda_core::docker::initialization target: fixed message, validated generated environment UUID, relay_micros and ric_micros as bounded numeric fields. The relay phase starts on entry to environment initialization and ends at observed RelayReady; the function/RIC phase starts there and ends at observed BootstrapReady. They include their respective container create/attach/start IO, scheduling and handshake observation; they do not claim isolated RIC CPU time. The two contiguous intervals partition observed initialization. Do not emit a successful-ready event on initialization error, cancellation or timeout. No artifact path, function name, request/event/environment value, credential or token appears in the event. Existing default log-level behavior and runtime wire/ASL/history/archive schemas are unchanged.

The performance-only server profile enables this target with all other logs at error. After startup readiness and before spawning the SDK child, the retained Rust owner captures a bounded newline-complete immutable server-log prefix. Verify that prefix is unchanged after joined shutdown and bind its byte count/hash; the prefix contains startup kernel/endpoint self-test environment events, which are not workload samples. Parse every successful initialization event in the remaining suffix, rejecting unknown identities. Cold evidence requires exactly20 distinct environment IDs, exactly one event per independently observed environment, both durations and their sum bounded by that environment's enclosing Invoke duration with explicit rounding tolerance. Never match by line order alone. Ordinary profiles retain their existing error-only filter. A deterministic controlled tracing subscriber must verify stage intervals and absence of a ready event on failed initialization.

## 3. Independent handler fixture

Extend the existing independent Rust RIC with two explicit modes, timed and timed-log-flood. They retain the existing computed value, request ID, process-local counter, revision and other response fields. Only these modes add handlerNanos, measured after next returns through computation/delay, immediately before response JSON construction. Existing compute/error/framing/forgery/hold behavior and output contracts remain unchanged.

The timed mode sleeps for the existing bounded delayMillis (warm workload uses10ms). The flood mode starts one owned writer thread, writes fixed non-secret1KiB chunks paced1ms apart with a hard1MiB ceiling, and enters an authenticated bounded collector barrier only after a positive write. The controller probes readiness while that barrier holds the handler and the writer remains active. After release the handler stops/joins its writer and reports a second matching request-ID barrier with before/after byte counts; require0<before<after<1MiB, proving the flood phase spans the probe and did not exhaust before release. It then sleeps5000ms and is retired by the real1s handler timeout. All collector threads/connections and writer exits are owned/bounded; a missed barrier, exhausted writer, early timeout, failed join or mismatched ID fails the workload. The collector reuses the existing bounded custom-RIC HTTP collector and token barrier; the established explicit bridge/gateway startup profile supplies reachability. Do not emit host data or random unbounded text. Fixture response serialization/posting is intentionally part of adapter overhead. Verify checked timing conversion and unchanged ordinary response fields without unsafe code or panic on input.

## 4. Workloads

The registered server owner creates its evidence root with explicit0700 permissions, independent of host/guest umask; production ancestor/namespace permission checks remain unchanged. Each workload has a fresh owned server with one environment slot, an explicitly configured2s idle timeout and actual production resource limits. DeleteFunction removes metadata but does not promise synchronous warm-environment retirement. Outside Invoke timing, deletion starts one15s observation covering deletion completion, actual idle-driven container absence and zero allocation/memory/quarantine charges; fail on expiry rather than assuming API completion proves cleanup. Warm calls stay sequential within the configured idle window. Other fixtures retain their existing idle setting. Its selected images are provisioned/inspected before timing; ordinary service startup and one warm priming call are excluded only where explicitly stated below. All SDK retries are disabled, credentials are fixed local dummy credentials, per-operation IO is bounded and complete child lifetime stays within the existing480s image SDK owner cutoff. A Python original400s alarm covers workload/cleanup/report IO and interruption handlers; timeout worker futures/threads must join within their bounded SDK operation. Errors retain evidence and fail qualification.

### 4.1 Warm100

Create/settle one function with reserved concurrency1. Prime once and prove a single relay/function pair. Execute100 sequential timed10ms calls; require computation, revision, unique request IDs, counters2..101 and the same exact pair/environment throughout. Keep all100 raw nanosecond observations, rank summaries and measured overhead. Delete the function and independently prove its pair absent before returning the report. Function creation, priming, metadata inspections and deletion are outside individual Invoke timing.

### 4.2 Cold20

Create/settle a fresh function for each observation. Require no live environment before its first Invoke. Execute one timed call with counter1; keep its exact pair/environment identity and complete Invoke duration. Delete and prove all containers absent before the next iteration; require20 distinct environments and40 distinct container IDs. Do not pull/build during measurements. After clean server shutdown bind the20 actual initialization events to these identities and durations, reporting relay and function/RIC phase samples separately. Require actual server/SDK OS/architecture provenance; distinguish debug assertions and VM allocations from reference qualification.

### 4.3 Success/timeout100 with live health

Use a function with Timeout=1 and reserved concurrency1. Each of100 cycles first succeeds on a fresh pair with counter1, then invokes timed-log-flood asynchronously on that same pair. Observe the matching flood-begin barrier before probing; release only after the live readiness bracket, then require flood-end with increased but unexhausted byte counts before accepting the timeout. Bracket an actual GET /_health/ready observation with two capabilities observations showing active Lambda work while the future remains incomplete; record per-cycle health latency and require it below2s. Future submission or container existence alone cannot prove accepted handler work. Require the actual ServiceException timeout classification, then zero owned containers and zero accepted/active invocation/environment charges after the returned error. Do not retry admission or hide an eventually growing pool.

Record all100 health/timeout durations, exact completed-cycle count, per-cycle container high-water≤2, absent pairs after every timeout and successful reuse of the single slot. Keep stage-independent daemon identity and owner-label fences. Use one persistent health connection. First perform and explicitly report one successful priming invocation followed by function deletion and independently confirmed empty namespace; this initializes the provider's legitimate lazy persistent WorkManager supervisor. Create the actual test function, then record runtimeTasks before its first invocation. This priming is outside the100 measured cycles and cannot replace any cycle. Start the original2s quiescence clock immediately when the timeout result returns, before container/accounting/task observations. Check its cutoff before and after every task response, including an immediate baseline match; measure through that successful response. After each timeout require zero Docker allocation/memory/quarantine and provider accepted/active charges, then within2s observe the actual Tokio live-task count return to that exact original baseline before the next cycle; report all100 quiescence durations/counts. No admission retry is permitted. Record actual server RSS/OS thread overhead separately from configured resource quotas where available; container/task accounting cannot be inferred from elapsed times or a final empty list. A missed budget is retained and analyzed, never filtered away. Producer coordination uses a capacity-one channel with nonblocking coalesced notifications after positive writes. Atomic byte counters establish progress. Both first-positive and post-release observations share one absolute1s cutoff beginning at flood entry; stale notifications cannot reset it or prove progress. Keep the writer active across the held begin barrier and stop/join it on every returned path. There is no independent100ms scheduler-speed guard. Controls must prove delayed first progress, stale notifications under the original cutoff, disconnect and nonblocking coalescing. This does not change the actual1s invocation timeout or any health/quiescence/workload requirement. An unobserved flood barrier must retain a bounded fixed diagnostic distinguishing begin/end, expected/observed begin/release/end counts and the already-completed invocation outcome (timeout, function exit, other service/SDK failure or success). Never print exception messages, credentials, collector tokens or payloads, retry a cycle, or substitute a later successful run for the failure.

## 5. Linux artifacts and reproducibility

Add discoverable Make targets for controls, focused resources, Linux cross-build and direct guest execution. Reuse the D101 bounded artifact selector with an explicit allowlisted test-target parameter for workflows_sdk or docker_startup. Keep the old default behavior and its pure-lane regressions. Require one actual GNU arm64 ELF harness and its Cargo-embedded sibling server, matching hashes before/after on both sides, original deadlines and fixed diagnostics. Each Cargo stream stays≤4MiB/4096 records and each record≤262144 bytes with the existing depth/member/string caps. Measurement documents explicitly allow up to4096 members under their32KiB byte limit; all existing Action/D101 decoder callers retain their512-member default.

Before the canonical lambda-docker-fixtures entrypoint performs any image build, retain existing IDs for the seven rebuilt source aliases (relay, ordinary handler, custom A/B, no-runtime, server and release image) with immutable content-addressed cache tags on the same fenced source engine. The allowlist has exactly seven names for arm64 or amd64; unknown architectures fail before Docker IO. Preserve every alias observed at entry; absent aliases are skipped to support initial or partial provisioning. The architecture-bound seven-name allowlist minus the report's complete preserved-reference list identifies those absences. This guard cannot certify that an absent alias never existed: the independent original-ID baseline audit remains mandatory for detecting prior loss. Reject multiple/invalid source IDs and cache alias conflicts before overwriting any reference; tag only captured IDs, verify each cache mapping and recheck original aliases before returning. Every operation uses the existing local-Unix/engine fences and original120s preparation guard; interrupted/uncertain effects fail without retries or deletion. Preserve the snapshot as one bounded WORKFLOWS_SOURCE_CACHE record in full qualification logs. This protects addressability when fixture builds replace mutable aliases; platform config equivalence alone cannot replace old image IDs.

Provision the relay, ordinary handler and updated custom-RIC images on the selected Linux daemon before timing. Resolve each selected source alias once to its full immutable content ID, save only that ID, load that ID and anchor it on the target with the content-addressed operator cache alias rustack-workflows-cache:d102-<64 lowercase hex digits>. Reject an existing alias mapped to any other ID; a matching alias is reusable. Fence alias inventory, creation and inspection like every other Docker operation and validate its exact mapping before and after the guest workload. Preserve these target cache aliases: untagged image imports can otherwise be garbage-collected when the runtime removes its final private pin. Configure/inspect the guest fixtures by the captured IDs; mutable alias movement cannot substitute a later artifact. Validate both Docker endpoints with the existing local-Unix grammar and fence every save/load/image inspect/inventory operation with the captured daemon ID immediately before and after dispatch. A post-effect mismatch retains uncertainty and fails without retry or deletion. Import only explicitly selected immutable local image sources from the retained ARM source engine if needed; bind source/target daemon IDs, content IDs and archive hashes/size limits, retain image caches, and preserve all preexisting sources. Transfer exactly three selected images, at most1GiB per saved archive and2GiB total. Each source save and target load has120s, all preparation has600s; readonly identity/metadata calls have10s and32KiB outputs. Save only into exclusive regular files in the task-owned evidence directory, enforce an inherited min(1GiB, remaining aggregate allowance, inherited soft/hard limits) file-size limit before dispatch and verify size/link/type/hash afterward. Retain CLI process/group ownership through completion or bounded retirement; source/target identity mismatch or output overflow fails without deleting images. No registry publication, prune, unrelated deletion, existing VM/context restart or global activation is permitted. The new guest remains retained after tests. Final image-cohort acceptance additionally binds each selected ID to the complete ARM fixture builder's exact named manifest-list export. A fixture rebuild can refresh attestation/index IDs despite identical platform configs. After the complete gate, capture/import its exact exported IDs and perform the final Linux run without another image build; retain earlier focused captures separately rather than equating config digests with addressable image IDs.

The guest uses its actual Python3.12 interpreter and the existing pinned pure-Python boto3/botocore1.43.118 package tree mounted read-only from the host SDK cache. Pass that explicit SDK root as a validated argument, verify actual distribution versions and record interpreter/package provenance. Do not execute the host Python binary or use ambient AWS credentials/proxies. Existing host executions keep the established pinned Python3.13 environment. This introduces no package/version change or network install into the guest.

Both actual-resource Make entrypoints require the current same-feature Docker manifest preflight (83 resources after D107). The direct guest owner additionally requires exact equality between its ignored ordinary test names derived from the canonical manifest (50 after D107) and the frozen manifest, then runs exactly the three named ignored performance resources once. Verify guest Python3.12 before dispatch and in each resulting report. Each normal Rust control and each required Python control must run explicitly; no exit0/zero-test success is accepted. The guest original cutoff is1500s, each resource group≤480s and its existing retirement allowance; the host SSH cutoff1600s stays within an original1650s host observation. Metadata/hash/report IO is covered, with the D101 interruption/retirement-safe guard. No user request/token/env dump is allowed. Preserve bounded raw child reports only after successful physical cleanup.

```text
┌─ one owned paced writer ──────────────┐
│ stderr write → atomic byte counter    │
│              → try_send notification │
└────────────────────┬─────────────────┘
                     ▼ capacity1, coalesced; writer never waits here
┌─ handler thread: one absolute1s cutoff ─────────────────────┐
│ first positive count → held begin/health/release barrier    │
│ newer count → stop + join writer → end proof → timed sleep  │
│ stale event: reread count; never renew the original cutoff  │
└────────────────────────────────────────────────────────────┘
```

## 6. Inventory, review and acceptance

Freeze the following three ordinary-startup resources before first execution:

- test_should_measure_one_hundred_warm_image_invocations
- test_should_measure_twenty_preloaded_cold_image_environments
- test_should_reuse_one_image_slot_after_one_hundred_log_flood_timeouts

They extend43 ordinary/76 total Docker resources to46/79 across the existing eight suites; Native26 remains unchanged. Focused and complete functional gates must use the same final sources. Only ARM qualification is scheduled under D100. The second ARM engine may satisfy existing two-engine correctness preconditions; both architectures are not required. Retain original Runner/image SDK lifecycle semantics and failure evidence.

Normal controls cover exact sample count/order/ranks, subtract-before-rounding, budget boundaries, invalid handler time, cold identity/event duplication/mismatch/missing rows, non-live health evidence, timeout classification, accounting growth and malformed/oversized reports. Run focused Linux resources and complete ARM functional inventory, required build/test/fmt/Clippy/rustdoc/strict/Native/security/text gates, independent source/evidence review and exact namespace/source-preservation postconditions. A successful local VM capture closes this workload dependency only: dedicated-reference, Desktop/rootless, W373 and complete M4 remain required.

## 7. Pinned metadata peer during the live flood bracket

Before each timed invocation, connect, negotiate the API and verify the engine on one independent metadata socket. Retain that exact socket with implicit reconnect disabled until the live pair comparison and authenticated end observation finish. Reject socket closure/replacement and nested/concurrent use; close on every exit. Outside this scope, each ordinary observation still verifies its own connection.

```text
┌─ before timed Invoke ───────────────────────────────────────┐
│ connect → negotiate API → verify engine → pin exact socket   │
└────────────────────────────┬───────────────────────────────┘
                             ▼ no reconnect or new identity setup
┌─ existing live bracket, original deadlines ─────────────────┐
│ begin → readiness/health → same pair → release → end         │
│ socket loss/replacement → fail → finally close owned socket  │
└────────────────────────────┬───────────────────────────────┘
                             ▼ normal per-operation fencing resumes
```

Keep the one-second handler/handshake budget, 100 cycles, health budget, physical retirement proof and resource limits unchanged. The previous rootless run's cycle-54 end-handshake timeout remains failed evidence with an unproven root cause. Removing repeated connection/identity setup from the live bracket does not retrospectively explain that failure.

## 8. Live observation ordering and failure localization (D109)

The second D108 source cohort fails cycle89 at end observation after begin89/release89/end88; handler outcome is the actual one-second timeout. The earlier complete83/83 run remains valid historical evidence, but cannot establish that the intermittent issue is fixed. Source review identifies a concrete oracle defect: the final capabilities/completion samples precede the potentially slow live Docker GET. Move that GET before the final capabilities sample, then sample completion last. Under the collector condition, check completion, insert the release flag, and check completion again before notifying: a completed future requires withdrawing the flag and rejecting the cycle. Future completion is monotonic, so the last negative sample brackets the inserted flag without treating the collector lock as a lock on Future state. Require running rows for both observed roles. This strengthens the existing live proof without changing deadlines.

```text
┌─ before Invoke ─────────────────────────────────────────────────┐
│ verified pinned metadata peer → actual running warm pair          │
└──────────────────────────────┬───────────────────────────────────┘
                               ▼ original submission clock
┌─ one-second handler live bracket ────────────────────────────────┐
│ begin → before capabilities → health → live pair GET               │
│       → after capabilities → completion sample                     │
│       → collector lock: completion recheck → release → end         │
└──────────────────────────────┬───────────────────────────────────┘
                               ▼ existing timeout/retirement/accounting
```

Retain bounded monotonic phase diagnostics on any failed cycle: submit, collector acceptance, begin receipt, before-capability/health/pair/after-capability boundaries, release insertion/withdrawal/commit, begin-response-written, end acceptance and completion-callback observation. Use fixed field names, nonnegative elapsed microseconds and current-cycle counters only; never emit tokens, event bodies, function names or credentials. Each timestamp is a local observation; the callback observation can lag the actual Future transition, and a successful server write does not prove client receipt. Bind the accepted/written transport hooks and completion callback to their original cycle object so delayed callbacks cannot contaminate a later cycle. A missing phase remains missing rather than being inferred. The existing healthy report schema and all100-cycle/one-second/health/retirement budgets remain unchanged. Normal controls must expose a delayed pair lookup and completion at the release boundary.

The previous logs cannot distinguish a delayed controller observation, begin-response transport, writer join or end connection. The production log ring overwrites capped data while continuing to drain; no cap-stop fault has been established. Diagnose the focused100-cycle resource first; only after concrete code corrections stabilize should the final complete source cohort be frozen and requalified. A retry alone must not relabel the failed cohort as passed.

## 9. Initialization-owned flood control channel (D110)

Focused D109 diagnostics retain a failure at cycle84 before any authenticated begin was accepted: only submitted=0 and completionCallbackObserved=1176836µs, with83 complete prior cycles. This proves late controller release is not the sole failure mode. It does not identify whether first writer progress, scheduling or request transport caused that historical timeout.

Remove connection establishment from the one-second measured handler window. The opt-in IMAGE_FIXTURE_INIT_MODE=flood branch prepares the collector channel through the existing RIC Agent before its first Runtime next request. This uses the existing20s initialization budget, adds no invocation and leaves warm-counter semantics, handlerTimeout=1s,100 cycles,2s health, physical retirement, process limits and report-size bounds unchanged. All other initialization modes retain their behavior. The existing ureq2.12.1 Agent pools fully consumed, length-delimited responses by scheme/host/port/proxy; Runtime API traffic therefore uses a separate pool key. Pool reuse alone is insufficient because it may reconnect: the collector enforces socket identity independently. See the [versioned upstream source](https://docs.rs/crate/ureq/2.12.1/source/README.md) and local registry response.rs/pool.rs/unit.rs.

```text
┌─ existing initialization owner (before first next) ───────────────┐
│ explicit flood mode → authenticated POST /flood-ready              │
│ full response read → same Agent retains this connection           │
└────────────────────────────────┬─────────────────────────────────┘
                                 ▼ one actual socket object
┌─ collector channel: exactly three authenticated exchanges ────────┐
│ new → ready reply → ready                                         │
│ ready → begin reply held for live health proof → begin            │
│ begin → end reply with positive writer-span proof → complete       │
│ mismatch/replay/new socket/invalid fields → reject + close         │
│ end response → close; next environment requires a fresh channel    │
└────────────────────────────────┬─────────────────────────────────┘
                                 ▼ existing handler timeout + pair retirement
```

The pure channel state owns a strong reference to the accepted socket object; never authorize by reused descriptor number or peer port. Bind function name and the canonical invocation UUID, require exactly ready→begin→end, and commit each phase only after its successful response write. Ready accepts exactly functionName; existing begin/end body validation remains mandatory. A new ready channel requires the previous one complete and physically closed. Reject begin/end on a replacement connection, including a client library's implicit reconnect. The generic collector keeps its existing close-per-response behavior unless an explicit bounded flood transport policy opts in; keep4 connection slots,4KiB cumulative headers,2KiB request bodies, existing socket/collector deadlines and joined shutdown. Close on rejection and after end. Delayed transport hooks retain the original channel and timeline objects.

Before timed submission, wait for authenticated ready plus its response-write commit under the collector condition. Charge this wait to the original20s preparation cutoff captured before the warm invocation; never reset it after receiving a response. Choose the timed submission clock only after that observation and under the same lock as submit, so a delayed callback cannot acquire a later timestamp than an already-chosen start. After timeout and physical environment retirement, wait for the end-write commit and actual socket close, then prove the request UUID and same-socket phases. This wait and runtime-task quiescence share the original timeout-return+2s cutoff and elapsed accounting; never create a new close budget. Notify observers after actual collector socket closure. SDK response completion/Docker retirement alone is not a synchronization event for collector threads. The successful cycles measurement adds controlChannels with exactly four integer counters: prepared, readyBeforeInvoke, sameConnection and closed, each exactly100. The existing unique100 request UUIDs and finite state transitions prevent replay; do not enlarge the32KiB report or weaken existing validators. Record current-cycle channel stage in bounded failure diagnostics without addresses, tokens or function names.

Normal controls cover strict phase order, mismatched/reused socket objects, closed sockets, wrong function/request identity, premature replacement, response-write ordering, authentication before transport hooks, exactly three persistent exchanges and close after end, and missing/wrong channel counters in retained reports. Focused actual rootless cycles must prove the protocol before final qualification. Keep every prior failure unchanged; describe this as removing a setup hazard, not proving the historical cause.

The controlled execution-image refresh now covers five sources: runtime, custom RIC A/B, server and release, through four fixed Make targets (runtime/custom-RIC/server/release). Retain all old IDs before any build. Both explicit functional controllers and rootless import require the exact five-image source-sealed producer; only those five canonical aliases may advance with preceding receipt/cache-anchor proof. The full catalog remains15 images within the existing archive bounds. No x86 build/validation, daemon restart, image deletion or publication is introduced.

### D110 executable artifact correction

The first real D110 run fails before the flood protocol because the rebuilt RIC layer preserves a host0700 bootstrap owned by root. The build owner inherited umask077 to protect private diagnostics; relying on COPY's source mode made both entrypoints unexecutable by UID1000/65532. The retained actual image-2.tar independently proves both root-owned0700 entries; this is a confirmed artifact defect, separate from earlier intermittent handshakes. Set explicit COPY --chmod=0555 for /image-bootstrap and /alternate-bootstrap. Keep the non-root user/read-only policy, retain the bad image IDs and raw failure records, rebuild through the existing guarded five-image producer, inspect the new immutable layer permissions and execute the real non-root resource. No budget or protocol waiver is permitted.
