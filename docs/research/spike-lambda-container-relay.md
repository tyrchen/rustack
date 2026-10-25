# Spike: Can Docker Attach Drive an Unmodified Lambda RIC Image?

Status: PASS with platform qualifications · Date: 2026-10-25 · Consumer: [runtime design](../../specs/rustack-workflows-20-lambda-runtime-design.md)

## Question

Can a relay own an environment-local loopback Runtime API and exchange binary control frames through Docker attach while an ordinary Lambda image runs in the same network namespace, without published Runtime API ports or host networking?

## Method

The retained standalone crate is [tools/workflows-spikes](../../tools/workflows-spikes/Cargo.toml), with [relay](../../tools/workflows-spikes/src/bin/relay.rs), [driver](../../tools/workflows-spikes/src/bin/driver.rs), pinned [relay image](../../tools/workflows-spikes/Relay.Dockerfile) and [official-RIC fixture image](../../tools/workflows-spikes/Handler.Dockerfile). It is an executable feasibility experiment, not the production executor or hardened external CBOR parser.

Host: macOS 27 arm64, Apple M5, 64 GiB RAM. Docker CLI 29.9.2, Engine 29.5.2, Colima 0.10.3, Lima 2.2.1, QEMU 11.1.2 and Cargo Lambda 1.9.2. A dedicated arm64 VZ VM uses 4 CPUs/6 GiB; a separate x86_64 QEMU VM uses 2 CPUs/4 GiB. Neither mounts the host home or forwards its SSH agent. The x86_64 VM is **emulated hardware**, not native x86 performance evidence. Docker Desktop, rootless Docker and a native x86 runner remain required M4 lanes.

Reproduction requires Docker, Cargo Lambda/Zig, and buildx. On Homebrew, set the builder executable explicitly if Docker plugin discovery is unconfigured:

```sh
make workflows-relay-spike \
  WORKFLOWS_DOCKER_SOCKET=/path/to/docker.sock \
  WORKFLOWS_ARCH=arm64 \
  WORKFLOWS_IMAGE_BUILDER=/opt/homebrew/bin/docker-buildx
```

Repeat with amd64 and its own socket. Make builds the Linux relay, builds both digest-pinned fixture images, runs actual invocations and removes all created containers. It never starts another AWS emulator or contacts an AWS service API. Registry image provisioning still requires network access on a cold cache.

## Findings

1. Binary, non-TTY, bidirectional Docker attach works with the selected Bollard API. Attach is established before relay start; Hello and Ready are received without relying on replayed logs. The RIC's first GET /next produces a distinct BootstrapReady signal.
2. The official Python 3.13 Lambda image enters its normal RIC path when AWS_LAMBDA_RUNTIME_API points to 127.0.0.1:9001 in the shared netns. Two calls compute 42 and 7 from different input values; the handler's counter advances 1→2. These are real handler results, not a transport echo.
3. Handler-side evidence confirms UID 1000, memory.max=201326592, pids.max=256 and read-only rootfs. The driver independently checks that the relay publishes no ports. This proves these controls in the measured VMs; it does not establish hostile multi-tenant isolation.
4. A raised ValueError travels through `/error`. The cancellation probe waits for the exact invocation's busy-start log marker before removing the sleeping function. Close/Ack succeeds, followed by removal of both owned containers. A single successful response is therefore not the entire evidence.
5. Seven framing tests cover fragmented binary data, oversized prefixes, trailing CBOR data, clean/truncated EOF and redacted Debug; five doc examples compile/pass. The standalone crate passes strict pedantic/boundary Clippy and rustdoc. Its own dependency audit reports zero vulnerabilities.
6. Initial single-run cold observations were 122 ms on arm64 and 2,526 ms on emulated amd64. These are feasibility observations under development load, **not p95 performance qualification**. Retained normalized run evidence is [arm64](../../tests/workflows/evidence/relay-arm64.json) and [emulated amd64](../../tests/workflows/evidence/relay-amd64.json), including EOF and label-reconciliation observations.

## Decision

**GO with qualifications:** retain the relay architecture and pay its two-container overhead in the environment budget. Configure DNS/extra-host entries on the relay/netns owner. Keep control transport separate from function SDK gateway reachability. The experiment is not permission to copy its reduced protocol validation or failure handling into production unchanged.

## Risks and implementation requirements

Production P1 still needs strict bounded CBOR preflight, typed variants, complete Runtime API identity/error behavior, cancellation supervision, quarantine/reconciliation, image revision lifecycle and dependency-log filtering. The real attach EOF comparison found StdinOnce=false leaves the relay running after input shutdown; true delivers EOF and ends it before cleanup. The production contract now requires true. Cleanup also reconciles a run-unique label, including a deliberately unrecorded create result, instead of relying only on received IDs. The experiment caught a Close/read-worker race: a reader must not terminate the environment before the owner consumes queued Close. Retain single-owner settlement in production. SIGKILL cleanup remains a startup reconciliation responsibility; there is no crash-recovery claim.
