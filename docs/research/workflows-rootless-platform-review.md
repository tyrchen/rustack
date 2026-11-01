# Rootless Docker Bootstrap and Attestation Review

Date:2026-10-31. Scope: [D103 bootstrap dependency](../../specs/rustack-workflows-58-rootless-platform-design.md), P5/5.3. Status: implemented and independently reviewed; full rootless platform/M4 qualification remains open. No further x86 validation is performed.

## Contract and observed environment

The installed profile is `rustack-workflows-rootless-arm64-v2`: native ARM VZ,4vCPUs,8GiB,24GiB local disk, read-only repository/target mounts and no context activation, Rosetta or binfmt. This is not a dedicated physical reference machine. The first failed profile `rustack-workflows-rootless-arm64` remains retained and unqualified; no existing VM was restarted, stopped or deleted.

| Observation | Final evidence |
|---|---|
| Real rootless Engine | `7bbe02f2-0571-42f9-a4a3-a36a81425357`, Docker29.5.2, Linux aarch64, `name=rootless` |
| Separate infrastructure Engine | `fbfd8647-6291-4660-a0c0-e913f1f82266` |
| User service / dockerd | MainPID2311 / dockerd2361; all actual process UIDs501 |
| Primary guest identity | UID501/GID1000 |
| Socket | `/run/user/501/docker.sock`, UID501, mapped GID525278, mode01660, inode46; parent0700 |
| Socket group inside daemon namespace | GID991, derived from the actual subordinate map; not guessed from the host GID |
| Resource prerequisites | cgroup v2/systemd, no daemon warnings, cpu/memory/pids declared and enabled in user-manager subtree; manager/service control files owned/writable by UID501 |
| Compatible rootless extras | `5:29.5.2-1~ubuntu.24.04~noble`; no Engine upgrade |
| Published repository candidate | `5:29.9.2-1~ubuntu.24.04~noble`, independently recorded from signed apt metadata |

The primary Docker signing key fingerprint is pinned/checked before apt consumption. Existing canonical Colima Docker repository/key files are reused rather than duplicated. Guest package operations run under exact recorded root-manager transient units, independently bounded even if the unprivileged SSH/controller disappears. Client context changes by the official installer are confined to its private DOCKER_CONFIG.

## Failed attempts and read-only reconciliation

Do not relabel the three original failures as successful installation owners:

1. Original host handle18601 joins2. VM creation succeeds; apt update rejects two Docker Signed-By paths. No package installation or rootless user service occurs. `/tmp/rustack-d103-bootstrap-first.log`, `/tmp/rustack-d103-first-preconditions.json` and `/Users/tchen/.target/workflows-rootless-d103` retain the host evidence; guest entry/command records remain in the first VM. The conflicting task-created apt source remains with that failed, unqualified VM.
2. Handle56590 joins2 before any guest entry/setup effect: the strict no-symlink reader rejects `/etc/os-release`. The corrected code reads canonical `/usr/lib/os-release`. `/tmp/rustack-d103-bootstrap-second.log`, `/tmp/rustack-d103-second-source` and `/Users/tchen/.target/workflows-rootless-d103-v2` retain this attempt. No second setup is dispatched until a fresh read-only probe proves all rootless state/service/socket/data absent.
3. Explicit resumed owner57532 joins2 after all22 guest commands return0 and the actual daemon starts. Its final observer rejects legitimate sticky socket mode01660 and a mapped socket GID distinct from the primary guest GID. `/tmp/rustack-d103-bootstrap-resumed.log`, `/Users/tchen/.target/workflows-rootless-d103-v2-resumed` and the exact guest entry/source/intents/results/raw hashes preserve the failure. No package/service operation is rerun to fix report validation.

The recovery path requires a complete contiguous command inventory, all zero-exit results with matching raw hashes, one successful official installer, both equal historical rootless info reads and the historical rootful identity. Every exact root transient job must be absent. It then independently re-observes actual process/namespace/socket/controller/package/engine state. New host receipts bind old source bytes/nonce and the current script; the original failed guest records are unchanged.

Read-only recovery handle40810 joins0. Final-source capture79104 joins0 under `/Users/tchen/.target/workflows-rootless-d103-v2-final`; `/tmp/rustack-d103-final-attestation.log` retains its Make output. The source SHA256 is `a127607623ef137e422a6facb4ab2f52d78400271adb58ea239f9a190e0ce5e4`. Both qualification fields remain pending. The documented ordinary `workflows-rootless-attest` entry point also rechecks this recovered capture without guest mutation: original handle87420 joins0, retained in `/tmp/rustack-d103-repeat-attestation.log`.

## Verification and independent review

`make test-workflows-rootless-controls` passes25 normal controls. They cover rootful masquerade; UID/map/socket substitution; missing, unavailable or merely visible controllers; package/key/source conflicts; private source/intent ownership; exact root job timing; delayed fsync preventing dispatch; concurrent file publication; fresh-only setup resumption; mapped sticky sockets; and missing/corrupt/live-job recovery proof. No VM, Docker daemon or package installation is used by these controls.

The actual privileged-owner control handle53204 joins0. A root UID0 Python process ignores TERM and sleeps60s inside the exact root systemd invocation path. The configured runtime/stop budget kills it after10,312ms; PID/PGID3030 and its exact unit are independently absent afterward. `/tmp/rustack-d103-root-owner-control.log` and `/home/tchen.guest/.local/state/rustack-rootless-owner-d103` retain intent, raw output, nonzero service result and physical absence. The unit is `rustack-d103-c4dcae2735d349b69d2a75df6595bff4-001.service`. This qualifies privileged setup ownership; it is not the Lambda kernel probe.

The independent read-only reviewer rechecks delegation, atomic root-file publication, signed existing-repository reuse, root-owned lifetime bounds, durable pre-effect intents, post-fsync timing reserve, fresh-only resumption, recovery and mapped socket identity. No remaining concrete in-slice source findings. [W427–W434](../../specs/rustack-workflows-93-review.md#d103-rootless-bootstrap-review-w427w434) record the corrections and observed failures.

`/tmp/rustack-d103-postconditions.json` independently verifies the original three VM allocations and default Docker context unchanged, the original source/measurement engines unchanged, all32/15 original image IDs retained and zero containers/volumes on those engines. `/tmp/rustack-d103-rootless-inventory.json` retains four successful explicit-endpoint queries proving the new rootless engine ID and zero all-state containers, volumes and images before workload provisioning. No image build, retag, deletion, prune or publication occurs in this dependency.

This change is Python provisioning/control automation plus English specs/docs. New Make entries invoke metadata/provisioning/attestation only; no Rust source, artifact, dependency, feature, toolchain or Rust build/test target changes. Rust verification is not applicable to this slice; retain D102's existing Rust evidence without relabeling it as rootless execution. Text/link/index and diff checks apply here.

## Remaining platform exit

| Requirement | Status |
|---|---|
| Real rootless user daemon and effective delegation prerequisites | Pass |
| Signed/version-matched installation and bounded privileged ownership | Pass |
| Immutable install proof, independent/repeat attestation and source preservation | Pass |
| Actual Lambda Rust kernel probe under rootless policy | Pending |
| Immutable fixture transport and complete79-resource rootless inventory | Pending |
| Rootless server-container mapped socket access and endpoint proofs | Pending |
| Rootless performance/load and exact workload cleanup | Pending |
| Desktop, dedicated reference, remaining recovery/load, W373 and full M4 | Pending |

Continue from this retained installed daemon. Do not substitute bootstrap success for any pending row, rerun installation, or acquire an x86 runner.
