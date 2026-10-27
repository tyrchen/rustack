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

## Controlled pull transport preflight

Pinned Moby image_routes.go:35–104 parses fromImage/tag/platform and decodes X-Registry-Auth, then calls backend PullImage; its post-progress error path emits an in-band error after headers/bytes may already have succeeded. Bollard image.rs:120–155 converts streamed errorDetail to failure, confirming that HTTP200 alone is insufficient. A bounded consumer must parse each line, discard progress and join the same HTTP driver, rather than aggregate the stream through ordinary metadata collection. The classic backend's image_pull.go:32–51 can only warn about a platform mismatch; Rustack must still validate selected image metadata itself. Remote cache activity after cancellation is not an owned-container retirement proof.

The [current secrecy documentation](https://docs.rs/secrecy/latest/secrecy/) confirms0.10.3 remains current (checked2026-10-27); reuse the existing workspace cohort and explicit ExposeSecret access, wipe temporary encoded-source JSON and keep sensitive headers/filtering. No ambient Docker configuration or credential helper belongs in the runtime. See [runtime§2.14](../../specs/rustack-workflows-20-lambda-runtime-design.md#214-controlled-if-missing-pulls-and-retained-progress-transport).


The real pull fixture uses `public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79`, selected from the public `:24` manifest list on2026-10-27. The anonymous registry response is772bytes with media type application/vnd.docker.distribution.manifest.list.v2+json; SHA-256 of the raw manifest body gives the pinned digest. Its entries advertise linux/arm64(v8) and linux/amd64. [AWS's Node.js container guide](https://docs.aws.amazon.com/lambda/latest/dg/nodejs-image.html) identifies the public Lambda repository and Node.js24 base. This is an explicit qualification source, not an implicit runtime catalog; callers still choose and allowlist their source. No anonymous registry token or ambient credentials are retained. The initial real preparation reported cacheMiss=true on each dedicated daemon, and a second explicit endpoint call exercises warm pulls. Successful preparation proves selected-platform/default/unpacked/pin validation; this base-image pull test does not execute a Node handler. Existing independent RIC/executor/pool gates establish actual function computation separately. Cache layers/source refs remain daemon-owned and are intentionally retained; complete-prefix postconditions must still show zero runtime private aliases.


## Private registry YAML configuration preflight

The existing config0.15.25 dependency resolves yaml-rust2~0.11.1. The [current YAML event API](https://docs.rs/yaml-rust2/latest/yaml_rust2/parser/enum.Event.html) distinguishes scalars/styles, anchors/tags, maps/sequences and document boundaries; pull events permit immediate rejection of unsupported structure. The [current changelog](https://raw.githubusercontent.com/Ethiraric/yaml-rust2/master/CHANGELOG.md), checked2026-10-27, lists0.12 dependency/MSRV/Marker documentation changes and0.13 an encoding_rs MSRV pin;0.11.1 contains the short-document infinite-loop fix. Reuse that already installed cohort and its pull parser for the fixed shallow credential schema; no second parser/tree or new package version is required. Secret values require quotes, are converted immediately to secret types and never enter raw-event diagnostics. Descriptor-based local private-file loading belongs in retained bounded blocking workers, not config's general plaintext Value tree. See [operations§1.8](../../specs/rustack-workflows-60-operations-design.md#18-validated-docker-settings-and-private-registry-credential-files).


D79's loader uses safe nix OwnedFd open/openat/fstat/read in retained four-worker blocking jobs, preserving input/result permits through observer loss and zeroizing its owned source buffer. Mode/UID checks do not evaluate ACLs; operator-controlled ACLs remain explicit trust preconditions. The enhanced actual pull fixture creates only its own0600 empty auth map, then consumes shared settings→typed settings→loaded anonymous policy. Both retained daemons exercise warm public preparation; nonempty selected auth is independently checked in the scripted bound-wire preparation test, not claimed as a successful private-registry login. Existing [url2.5.8](https://docs.rs/url/latest/url/) and proptest1.12.0 workspace cohorts are direct consumer/test edges only; [current proptest documentation](https://docs.rs/proptest/latest/proptest/) was checked2026-10-27 and no package version is changed.


## Container-origin endpoint and TLS preflight

The [current stable rustls metadata](https://docs.rs/crate/rustls/latest) identifies0.23.46 with explicit aws-lc support, matching the existing workspace cohort. [tokio-rustls](https://docs.rs/tokio-rustls/latest/tokio_rustls/) is now0.26.6; update its compatible patch cohort from0.26.5 and avoid a second version. [webpki-roots](https://docs.rs/webpki-roots/latest/webpki_roots/)1.0.11 embeds Mozilla roots and requires rebuilding to refresh them. Own the TLS/HTTP future, disable any ambient proxy/redirect/retry discovery and select the aws-lc provider explicitly even if unrelated workspace clients enable ring. Dev-only [rcgen0.14.10](https://docs.rs/rcgen/latest/rcgen/) generates ephemeral test keys/certificates without committed credentials; enable only crypto/aws_lc_rs. All versions/API choices checked2026-10-27. A container-origin GET that independently observes the gateway's expected instance is a reachability/misrouting proof, not endpoint authentication. TLS adds normal certificate/name verification. See [runtime§4.11](../../specs/rustack-workflows-20-lambda-runtime-design.md#411-container-origin-gateway-reachability-evidence).

[Lima's default user network](https://lima-vm.io/docs/config/network/user/) documents host.lima.internal/host networking; this guides the retained Colima fixture only. Tests still require an explicit function-visible host and actual successful network observation, never infer readiness from a VM address or host-side TCP success. Operator bridges and temporary listeners have exact fixture owners and are removed only after all attached environments settle. Production's default gateway bind remains unchanged.


## Production startup ownership preflight

[Tokio's current JoinHandle source](https://docs.rs/tokio/latest/src/tokio/runtime/task/join.rs.html) and [watch receiver contract](https://docs.rs/tokio/latest/tokio/sync/watch/struct.Receiver.html), checked2026-10-27, distinguish observer cancellation from owned task lifetime. Retain accepted lower owners before dependent effects, forbid replay after an abandoned qualification, and reuse the existing CleanupTask for one resumable ordered shutdown on the captured runtime. [Stable File locking](https://doc.rust-lang.org/std/fs/struct.File.html) retains exclusion through descriptor lifetime, not pathname existence; producer provenance must be immutable and cannot be invented by scanning an empty namespace. Add a new owner magic as a downgrade barrier and reject legacy/unexplained owner-missing state for production admission. Existing [nix0.31.3 directory APIs](https://docs.rs/nix/0.31.3/nix/dir/struct.Dir.html) provide safe descriptor-relative enumeration for bounded first-publication checks; enable only its existing package's dir feature, with no version change. See [runtime§4.12](../../specs/rustack-workflows-20-lambda-runtime-design.md#412-production-startup-provenance-and-retained-owner-composition).


## Independent custom RIC consuming preflight

The [official Runtime API](https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html) defines the request identity received by GET /next and used by response/error posts. The [ImageConfig schema](https://docs.aws.amazon.com/lambda/latest/api/API_ImageConfig.html) allows empty argv arrays, so a real fixture must distinguish absent/cleared/replaced image defaults and inspect actual argv/cwd. Checked2026-10-27. D82's independent custom Rust binary consumes this protocol without linking Rustack session/relay code; the digest-pinned provided.al2023 base supplies Linux userland only. Its computed result, process counter and image environment marker qualify actual handler execution, not a second control-plane model.

[Current ureq documentation](https://docs.rs/ureq/latest/ureq/) identifies3.5.0 with a changed builder/body API. D82 reuses the existing fixture package's already-locked2.12.1/no-default-features cohort without adding a dependency/version or TLS implementation. Its existing local AgentBuilder source was inspected for explicit proxy disabling, bounded IO and status errors. This is an HTTP-only test RIC using a validated injected loopback address and explicit private collector, not the production gateway HTTP/TLS client. The production aws-lc/WebPKI choices remain unchanged.

A concurrent SDK call or sleep is not evidence that two handlers simultaneously consumed their requests. The two-stage bounded collector observes both active IDs before sharing foreign IDs, then requires both actual rejection reports before releasing either valid completion. The same barrier holds published A across a Successful latest B update. Operator-side mutable source aliases have unique intents separate from runtime-private pins; retained evidence, exact CLI waiters, positive target checks and joined unwind controls prevent a failed test from silently dropping cleanup authority. See [D82 qualification](../../specs/rustack-workflows-70-verification-plan.md#custom-rust-ric-and-immutable-image-revision-qualification-d82).
