# Study: Bollard's Docker Response and Attach Boundaries

Status: source review complete; no new runtime experiment · Date:2026-10-25 · Vendor pin: `vendors/bollard` @ `f9ec79e7546edfb674b1066c44a68815cb52251c` (`v0.21.1`)

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
