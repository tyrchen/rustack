# Study: Bollard's Docker Response and Attach Boundaries

Status: pinned source review complete; bounded wire implementation and qualification recorded below · Date:2026-10-25 · Vendor pins: `vendors/bollard` @ `f9ec79e7546edfb674b1066c44a68815cb52251c` (`v0.21.1`); `vendors/moby` @ `af898abe44662d9554fb15ee4d4a7307f1b8e315` (`v28.0.0`, historical compatibility/merge source)

## Question

Can the production Lambda environment owner use Bollard's ordinary response/attach APIs while enforcing the repository's byte bounds, strict framing and diagnostic secrecy before materialization?

The [existing relay experiment](./spike-lambda-container-relay.md) proves binary Docker attach and unchanged official RIC execution. It does not establish bounds inside the selected client's decoders. This study examines that smaller boundary before P1 task1.3 consumes it. The crate's [official version documentation](https://docs.rs/crate/bollard/0.21.1) and the published package's `.cargo_vcs_info.json` agree with this vendored release pin. No performance or cloud-parity claim follows from source inspection.

## Architecture map

```text
┌─ Bollard typed endpoint ───────────────────────────────────────────────┐
│ models/query parameters → request builder → selected transport        │
│                                │ synchronous request TRACE            │
│               ┌────────────────▼──────────────────────────────────┐   │
│               │ Hyper response headers / Incoming                │   │
│               │ request timeout ends after response acquisition   │   │
│               └───────┬───────────────────────┬───────────────────┘   │
│                       │                       │                       │
│  ordinary JSON        ▼                       ▼ HTTP upgrade         │
│  collect entire body → Bytes       split read/write → FramedRead      │
│  → response DEBUG → Serde          8-byte Docker header                │
│                                    → await declared payload length   │
│                                    → LogOutput / Console on EOF       │
└──────────────────────────────────────────────────────────────────────┘
          ▲                                      ▲
          │ body limits must precede collect      │ framing limits must
          │                                      │ precede buffering
┌─────────┴──────────────────────────────────────┴──────────────────────┐
│ Rustack host owner: independent deadlines, bounded parsing, retained  │
│ connection tasks, no uncertain replay, mandatory dependency filter    │
└───────────────────────────────────────────────────────────────────────┘
```

## Hot path and allocations

Endpoint methods construct generated Docker requests. For example, attach creates an upgrade POST and calls the private upgrade helper, then wraps its reader in the log decoder: `vendors/bollard/src/container.rs:540` and `:557`. The independent relay protocol lives *inside* these Docker stdout records; validating a CBOR prefix after receiving LogOutput cannot bound earlier Docker decoder buffering.

Ordinary model decoding collects the complete Incoming body before Serde: `vendors/bollard/src/docker.rs:1736`. String/error decoding does the same at `:1727`. Neither call site adds a body-length limiter. A bounded timeout is useful but cannot by itself bound bytes accumulated before expiry. Image inspection exposes image-supplied configuration, so these responses are not only small daemon-version records.

Request execution applies a timeout around obtaining the Hyper response at `vendors/bollard/src/docker.rs:1703`. Response body consumption occurs afterward. Rustack therefore needs an absolute deadline around header acquisition **and** body/frame processing, with a clock recheck before publishing success. This finding does not mean Bollard has no timeout at all; its existing header/request timeout is a different boundary.

The custom transport interface returns exactly `Response<Incoming>`: `vendors/bollard/src/docker.rs:127`. A straightforward `Response<Limited<Incoming>>` wrapper cannot satisfy that type. Lower-level counted IO or a body-forwarding transport can enforce limits, but would add a second transport layer to retain the otherwise opaque decoder. There is no ordinary response-limit parameter on the examined endpoint path.

## Attach decoder and EOF behavior

`NewlineLogOutputDecoder` owns a two-stage state: waiting for an eight-byte header, or waiting for a stream-type/declared-length payload. Its header processing at `vendors/bollard/src/read.rs:75` converts the u32 length and records it without a local maximum. At `:80`, it waits until BytesMut contains that length, then splits a complete payload. Incomplete records therefore remain buffered across polls. This is linear work/storage in received record size, rather than a Rustack-specific upper bound.

The decoder supports both multiplexed output and console/TTY fallback. A leading byte outside its multiplexed range can yield Console, and EOF handling at `vendors/bollard/src/read.rs:103` flushes trailing bytes as Console instead of treating every incomplete non-TTY record as an error. That compatibility policy is reasonable for a general Docker client. The relay protocol instead requires strict non-TTY framing: malformed/truncated records must invalidate the environment and must never become control payload.

Streaming a validated record in small chunks avoids allocating its whole declared payload. Docker record boundaries and CBOR frame boundaries are independent: a control frame can span records, and a record can carry bytes belonging to successive control frames. Rustack's existing ControlClient already preserves CBOR partial reads through retained IO; the outer decoder must preserve those bytes without relaxing its own validation.

## Diagnostic dispatch

Request-body TRACE runs while `process_request` constructs its future, before that future is polled: `vendors/bollard/src/docker.rs:1580`. Model response DEBUG occurs after body collection at `:1738`. Truncated log EOF DEBUG includes the trailing buffer at `vendors/bollard/src/read.rs:110`.

Wrapping only an already-constructed future is insufficient for synchronous stream builders. A dependency filter must cover construction and every poll, including spawned HTTP/stream workers. Errors must also avoid formatting arbitrary daemon messages or source payloads into infrastructure output. These observations refine W01; they do not establish that W01's production consumer is already implemented.

## Reusable contracts

The generated `bollard-stubs` package contains Serde models and query shapes independently of the network client. The release pins version `1.53.1-rc.29.3.1` with default features disabled in `vendors/bollard/Cargo.toml:65`. Its model/query sources are `vendors/bollard/codegen/swagger/src/models.rs` and `query_parameters.rs`. Those types are useful without adopting the response collector, attach decoder, socket discovery, SSH or TLS features.

The upstream API negotiation behavior at `vendors/bollard/src/docker.rs:1537` selects the lower supported version after reading the daemon version. Rustack should preserve the observable version compatibility while validating the bounded version response and rejecting an incompatible minimum. Do not infer the runtime API version only from a README example; the release's actual constant and generated model pin are more precise.

## Outcome for the specification

Use the generated model/query contracts, with a small bounded Unix HTTP/upgrade adapter for the exact Docker operations in this feature. Keep container/deployment ownership above that adapter. The component design, including concrete quotas, connection supervision and error behavior, belongs in [runtime §3.6](../../specs/rustack-workflows-20-lambda-runtime-design.md#36-bounded-docker-api-boundary), not in this research memo.

The earlier production suggestion to use Bollard's ordinary pipe client is superseded by D19; the retained P0 driver remains valid as a feasibility experiment. This is an in-phase transport refinement, not a request to rewrite the Docker API or support another container platform.

Required implementation evidence remains bounded fake-daemon tests, actual attach/large-payload/truncation checks, TRACE capture and the established arm64/amd64 RIC lanes. No unresolved source assumption authorizes bypassing those tests. A source review is not proof that the forthcoming bounded adapter works.

## Implementation follow-through

P1 now reuses only generated bollard-stubs models in production. DockerWire owns bounded version/info/image inspection, typed container create/start/inspect/kill/wait/remove/list and strict upgraded attach. Its actor retains request jobs after caller cancellation; ordinary HTTP drivers are aborted/joined or recover/drop their one-shot IO, and upgraded drivers join before socket transfer. A separate32-job wait lane prevents long-running lifecycle waits from starving the two destructive-cleanup slots. Deployment actors, log/pull/network/probe consumers, pool policy and provider integration remain required; this is not executor acceptance.

The current workspace and standalone probe align on [Tokio1.54.1](https://docs.rs/crate/tokio/1.54.1) and [Hyper1.12.0](https://docs.rs/crate/hyper/1.12.0), rechecked against official package documentation on2026-10-25. The original Hyper1.11.1 audit remains historical evidence; mandatory task-bound filtering and fragmented trailer tests remain in effect on the updated stack.

A retained compatibility regression calls the actual pinned Bollard0.21.1 create and logs APIs against a fake local Unix daemon under TRACE with LogTracer enabled. It covers synchronous stream construction, asynchronous request/response polls and the decoder's truncated-EOF Console fallback. Test-only markers in request environment, response warnings, container target and trailing bytes are absent from infrastructure capture while an application event remains present. The production boundary independently captures create requests, response bytes/headers and fragmented trailers. These are executed call-site tests, not only synthetic logging-target tests.

The explicit `make test-lambda-docker-wire` target builds the real relay and unchanged official Python RIC image and uses DockerWire for daemon IO without the Docker CLI. It sends and receives6MiB-minus4096-byte-class data through actual multiplexed attach, checks computed warm results and a handler error, closes/joins the relay stream and verifies the run-label scope is empty after removal. Stress qualification uses60s IO/120s attempt budgets; it does not establish default5s latency or throughput. The existing five-test `test-lambda-runtime-docker` suite remains a separate relay-contract regression. Exact platform results and current review state are recorded in [verification](../../specs/rustack-workflows-70-verification-plan.md) and [implementation review](../../specs/rustack-workflows-93-review.md).

## Platform-specific image identity follow-up

The2026-10-25 Engine29.5.2/containerd-store inspection of the local official-RIC fixture disproved an overly broad reading of the generated Id field's historical config-ID description. Unqualified inspect returned an OCI index descriptor and the same digest as Id. Platform-qualified inspect returned a different Id and a single image-manifest descriptor for Linux arm64, while RepoDigests still named the parent index. The manifests view also included a separate attestation descriptor. Neither the parent Id nor the first RepoDigests string alone identifies the selected executable child.

The [official version history](https://docs.docker.com/reference/api/engine/version-history/#v149-api-changes) and [CLI inspect documentation](https://docs.docker.com/reference/cli/docker/image/inspect/) pin platform-qualified inspection to API1.49+. The platform argument is JSON-encoded OCI platform data and cannot be combined with manifests=true. Generated contracts distinguish descriptor/manifest/platform data at `vendors/bollard/codegen/swagger/src/models.rs:4006`, `:4129` (manifest summary), and the typed image inspect query in `vendors/bollard/codegen/swagger/src/query_parameters.rs`. Descriptor fields and RepoDigests are daemon metadata, not evidence of a successful registry push: BuildKit load can populate RepoDigests even for a short, unpushed local name.

Downstream resolution must request the declared platform when available, validate Linux architecture and an executable effective entry configuration, and reject an unresolved index or attestation as executable metadata. Older compatible API lanes may inspect legacy single-platform images but cannot silently accept an unresolved index. A registry URI must distinguish its repository/index evidence from the selected child descriptor; short local references retain the explicit local URI extension. The implementation contract and tests belong to runtime §2.8/D21. This is identity correctness evidence, not qualification of arbitrary cross-architecture emulation or registry pulls.

Independent preflight also identified the Docker create merge boundary: an empty request entrypoint can cause an empty command to be filled from the image. The future create adapter must consume fully materialized validated argv/workdir, with the empty-entrypoint reset sentinel, rather than pass only optional function overrides. Capturing only a metadata hash would lose the defaults needed to do this without per-invoke inspection. Repository evidence comparison also needs familiar Docker Hub name normalization while retaining the original short-local provenance decision. These refinements are recorded as W33/W34 in the review backlog and pinned in §2.8/D21.

The first consumer qualification deliberately tried create by the platform-inspect child Id and failed404 on both retained daemons. A read-only lookup of that still-current child digest also returns404, while the current unqualified parent Id is addressable. Resolution must therefore capture the addressable parent before selecting a platform and create by that immutable parent plus an explicit platform parameter. The selected manifest remains independent fingerprint/registry metadata; it must not be discarded. Rebuilding a mutable tag can also replace its index descriptor, so a later deployment owner must retain artifact availability under its own lifetime policy; missing externally removed content must fail rather than resolve a newer tag.

Independent implementation review additionally requires recomputing saved fingerprints after valid-string mutations, and rejects descriptor-free fallback without positive classic-store /info evidence. A downlevel containerd reply can omit Descriptor while still exposing a nonempty GraphDriver. Classic names must match between inspect and info, manifest summaries must not contradict the fallback, and containerd's driver-type status marker must reject it. These qualifications are explicit compatibility checks, not a claim that an old containerd/GraphDriver spelling proves classic behavior.

### Pinned engine source for merge and legacy-store guards

The newly vendored Moby v28.0.0 is a historical API1.48 source pin, not the source version of the live Engine29.5.2 daemons. Its config merge fills an empty command when the request entrypoint has length zero at `vendors/moby/daemon/commit.go:72`; `vendors/moby/daemon/create.go:302` resets the single-empty-string entrypoint only after merge. This explains why materialized nonempty/reset entrypoints preserve an intentionally empty command. Actual modern-daemon create/inspect regressions verify both clear-command and reset-entrypoint cases.

The same historical source strips Descriptor below API1.48 at `vendors/moby/api/server/router/image/image_routes.go:374`. Containerd nevertheless supplies a GraphDriver name from its snapshotter at `vendors/moby/daemon/containerd/image_inspect.go:117`; a nonempty GraphDriver cannot alone prove classic storage. The info status marker is emitted at `vendors/moby/daemon/containerd/service.go:112`, with the exact io.containerd.snapshotter.v1 value pinned at `vendors/moby/vendor/github.com/containerd/containerd/v2/plugins/types.go:39`. Classic VFS returns nil status at `vendors/moby/daemon/graphdriver/vfs/driver.go:87`. Normalize absent/null DriverStatus after matching classic driver evidence, while rejecting the explicit containerd marker and malformed rows. Fake-daemon/pure metadata tests cover these paths; no execution qualification of an old daemon is claimed.

The final local resolver retains validated execution defaults in Arc storage, recomputes a canonical fingerprint on restore, binds platform inspection to the captured addressable parent Id and passes that Id with explicit platform to actual create. Its61 additional tests cover metadata/defaults/legacy API behavior and valid-value stale-hash mutation. Both real two-case Docker wire suites pass on arm64 VZ and amd64/QEMU. Full workspace1964 tests and standard build/fmt/Clippy/rustdoc pass. Deployment actors, availability leases, allowed pulls, unpacked-image quota proof and pool/provider integration still belong to the remaining P1 work.

## Image pin and unpacked-size consumer preflight

Pinned Moby's containerd tag path targets a resolved descriptor at `vendors/moby/daemon/containerd/image_tag.go:18`; the create/replace helper at:37 establishes the alias before later event/dangling-name work. This permits visible matching alias evidence to establish a tag commit after a lost response, while absence cannot establish that an unobserved tag will never commit. Cleanup must never replay Tag as a barrier.

Image delete by repository reference removes that reference, and force/prune flags govern additional deletion at `vendors/moby/daemon/containerd/image_delete.go:24`. Rustack's adapter therefore accepts only allocated private-pin identities, never a caller source tag or content ID, and disables force/ancestor pruning. Published metadata still uses the original requested URI.

The generated manifest summary explicitly separates compressed/content, unpacked image data and total storage at `vendors/bollard/codegen/swagger/src/models.rs:4236`. The real fixture's selected content size and unpacked size differ substantially. Modern quota proof must match the selected image summary by digest/platform and read ImageData.Size.Unpacked; a compressed Size or Total is not the same measurement. Legacy fallback remains restricted to verified classic storage. No image handler needs to run to obtain this metadata for already unpacked images; unavailable proof is an explicit preparation failure.

Fresh-alias Tag preflight returns400/404 before mutation in the pinned Engine router at `vendors/moby/api/server/router/image/image_routes.go:472`; success201 follows backend return at:491. Classic reference validation precedes mutation at `vendors/moby/reference/store.go:125`; post-mutation storage failures map to500. The manager therefore treats only exact400/404 and local wire Busy as known no-effect rejection.

A full500 response is still not a commit barrier. The containerd client invokes a remote Create at `vendors/moby/vendor/github.com/containerd/containerd/v2/client/image_store.go:73`; Moby wraps Create errors as System at `vendors/moby/daemon/containerd/image_tag.go:45`. The remote metadata update enters a database transaction at `vendors/moby/vendor/github.com/containerd/containerd/v2/core/metadata/bolt.go:54`, which does not establish that a cancelled/disconnected client's observed error stopped server commit. Keep HTTP500 in the uncertain phase and test a complete outer error followed by initial404 and a later matching alias commit. Do not replace retained quarantine with an HTTP-status-only completion assumption.

The owned preparation consumer now implements that contract with bounded job/pin budgets, private-only tag/removal keys, shared release channels, phase registry/retained joins, a separate two-slot cleanup semaphore and emergency phase recovery on the captured runtime handle. The real harness keeps acquired artifacts through actual RIC calls and container retirement before joining pin cleanup. Both arm64 VZ and amd64/QEMU two-case suites pass; independent scoped image queries are empty afterward.33 new unit/lifecycle cases pass within321 Lambda core tests, plus76 core doctests; the workspace has1,997 passing tests and standard build/fmt/Clippy/rustdoc gates. Provider status/CAS, leased invocation capture, previous-run coordination, allowed pulls and the production executor remain required P1 consumers.

## Persistent daemon identity for historical cleanup

The vendored Moby v28.0.0 source at daemon/id.go:12–34 loads engine-id from its data root or atomically publishes a random UUID; daemon/info.go:54 exposes that installation identity as /info.ID. The [current upstream implementation](https://raw.githubusercontent.com/moby/moby/master/daemon/id.go) has the same persistence contract (checked2026-10-27). This source is not an authenticated identity scheme, and copied/reset engine IDs cannot identify distinct installations safely. The selected modern profile requires canonical nonnil UUIDs rather than guessing from hostname/path/version. Both retained Engine29.5.2 daemons return distinct canonical UUIDs; those live values are intentionally not recorded here.

Rustack's wire opens a fresh Unix connection for every operation. Therefore a prior /info call on a separate socket cannot establish the identity of a later404 or mutation. D76 requires checking ID on each exact HTTP connection, before actual issuance, and never reconnecting after the checked peer closes. Persist the owner/daemon pair before first effect, refuse first binding over either nonempty journal, and carry a private exact-lease binding through ports/envelopes. Source contracts guide the design; actual bound recovery/provider/attach tests supply independent functional evidence. See [runtime§4.9](../../specs/rustack-workflows-20-lambda-runtime-design.md#49-durable-daemon-binding-and-connection-local-identity-fences).

## Complete observed namespace inventories

Pinned Moby daemon/list.go:435–438 stops the container iterator at a positive Limit; the existing Rustack operational limit64 query therefore cannot prove absence of additional owned resources. Use an all-state owner filter without that limit for the startup audit. Pinned image_list.go:161–182 filters individual references using distribution/reference.FamiliarMatch; helpers.go:36 delegates to path.Match, so slash-limited globs do not prove an arbitrary-depth reserved-prefix absence. The [official container listing documentation](https://docs.docker.com/reference/cli/docker/container/ls/) distinguishes all-state and last-N listing, and the [image listing documentation](https://docs.docker.com/reference/cli/docker/image/ls/) documents filtering and dangling-image behavior (checked2026-10-27).

D77 therefore drains recorded effects through the existing identity-checked recoverers, then checks an untruncated owner-container list and a bounded full image-tag prefix scan. Unrecorded owned observations block admission without adoption: a name or label is not a substitute for durable mutation intent. These observations are not retroactive proof of unseen legacy effects and do not grant production readiness. See [runtime§4.10](../../specs/rustack-workflows-20-lambda-runtime-design.md#410-recorded-namespace-retirement-and-complete-observed-inventories).

ImageSummary requires RepoTags in pinned api/swagger.yaml:2230 and the [current upstream schema](https://raw.githubusercontent.com/moby/moby/master/api/swagger.yaml) (checked2026-10-27). A minimal projection may ignore unrelated extension fields, but must not default a missing RepoTags field to empty. Explicit null retains the existing generated-client legacy vector compatibility; malformed/missing data fails before an empty-inventory claim. W223 records this specification correction and its HTTP regression.
