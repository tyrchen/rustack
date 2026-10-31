# Rootless Docker Platform Qualification

Status: bootstrap/attestation dependency implemented and independently reviewed; actual rootless daemon, read-only recovery and root-owned timeout retirement pass. Full kernel/resource platform qualification remains pending.

Phase: P5/5.3. Decision: [D103](./rustack-workflows-99-decisions.md#d103--qualify-an-isolated-rootless-daemon-with-effective-resource-controls). Apply the [required matrix](./rustack-workflows-70-verification-plan.md#2-required-test-matrix), [image performance contract](./rustack-workflows-57-image-performance-design.md) and D100's x86 waiver. Desktop and the dedicated reference remain separate requirements.

## 1. Scope and dependency order

First provision and attest a real rootless daemon. Then qualify the complete current Docker resource inventory, including official Python/Node/custom RIC, provided ZIP, SDK callbacks, Pulumi, archive/recovery, server-container and release-image consumers. Rootless bootstrap success alone does not close P5/5.3. Do not replace required cases with a minimal Invoke smoke or suppress kernel policy failures. Keep the existing D102 evidence and every existing VM/context intact.

```text
┌─ explicit Make / bounded host process owner ──────────────────────┐
│ fresh native ARM VZ profile → retained ownership receipt          │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
┌─ separate Linux VM: infrastructure rootful daemon retained ──────┐
│ signed packages → unprivileged systemd user service              │
│ rootless Docker: private user socket / local disk / cgroup v2    │
│ identity + UID mapping + delegated cpu/memory/pids attestation   │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
 immutable fixture transport → kernel probe → complete resource gate
 → exact owned-resource absence + retained source-image audit
```

## 2. Isolated provisioning and installation

Expose `workflows-rootless-provision`, `workflows-rootless-attest` and `test-workflows-rootless-controls` through Make. The default profile is `rustack-workflows-rootless-arm64`. Provision accepts only a fresh profile, on macOS arm64, using the existing D101 native VZ provisioning contract: 4 vCPUs, 8 GiB, 24 GiB local disk, read-only repository/target mounts, no Rosetta/binfmt, no context activation or SSH-config mutation. Four virtual CPUs do not prove physical dedication.

Create a private, exclusive host evidence directory before any VM mutation. Retain an entry receipt with schema version, profile, random nonce, rootless-script hash and original command intent. A failed or interrupted provisioning attempt retains its directory/profile and must not be silently retried against the partially provisioned VM. Attestation is read-only and may be repeated from that receipt; missing or conflicting receipts fail. Never stop/restart/delete a preexisting VM, daemon or user service. The new VM's infrastructure rootful daemon stays running and must have a different engine ID from the rootless daemon.

The explicit `workflows-rootless-resume-setup` target may reuse an owned failed-provisioning VM only when a read-only guest probe proves that no rootless state/entry directory, Docker user service, user daemon socket or rootless data directory exists. Require the original private host entry, exact profile/allocation and target mount path; create a new exclusive evidence directory, preserve the old entry/hash and bind the current script/nonce before dispatch. This handles failures before any guest setup effect (for example a trusted OS-release symlink rejected before the entry). A guest with any setup entry fails this path. Never use it to rerun partial package/service installation. The second attempt has one1,200s cutoff; the prior evidence remains unchanged.

If installation completed but only its final report validator failed, `workflows-rootless-recover-attestation` performs a new90s read-only capture. Bind the original host entry/nonce and guest entry/exact retained script hash; require the original failure outcome, no successful result, and a complete, hash-verified zero-exit result for every durable command intent. Require exactly one successful official setup invocation, both equal historical rootless info observations and the historical rootful engine ID. Every exact recorded root transient unit must be absent before re-attestation. Re-observe actual UID/maps/socket/delegation/package/engine state and require historical engine equality. Write only a new host receipt; never rewrite failed guest evidence, rerun installation, restart a service or accept an uncertain privileged job. Later attestation of this capture uses the same immutable installation proof.

Inside this newly owned VM only, install `uidmap`, `dbus-user-session`, `slirp4netns` and `docker-ce-rootless-extras`. Use Ubuntu's signed repositories and Docker's signed official apt repository, with the official Docker key fingerprint checked before repository configuration. Pin rootless extras to the installed infrastructure Engine version, recording that version rather than upgrading the existing Engine opportunistically. Reject an unavailable exact package. Record the currently published version separately from the compatibility pin. All package operations use argv, bounded output and finite deadlines; no downloaded convenience script is piped to a shell. Repository/key files are created exclusively or accepted only with identical contents. Do not disable AppArmor or unprivileged-namespace restrictions globally.

Colima may already supply the canonical Docker `docker.list` and `/etc/apt/keyrings/docker.asc`. Before creating any source, inspect at most64 active apt source files, bounded64KiB each. Reuse only the exact arm64/noble/stable official HTTPS source with a root-owned regular0644 source/key and the pinned primary key fingerprint. Reject duplicate or unfamiliar Docker source configuration. Do not add a second Signed-By path for an existing repository. New system files use root-owned same-directory temporary files, fsync and atomic no-replace link publication; any existing destination must be a root-owned regular0644 exact-content match. Never overwrite a concurrent destination.

Require nonzero guest UID, private `/run/user/<uid>`, at least 65,536 subordinate UIDs/GIDs and cgroup v2. Prove `cpu`, `memory`, `pids` in the user manager's systemd DelegateControllers, available controllers and enabled subtree; require its subtree control file to be owned/writable by the guest user. Independently prove the daemon service's declared/available controllers and writable delegated control file. Availability alone does not prove delegation. Reload the newly provisioned guest's user-unit catalog and start its package-provided DBus socket before the installer. Install/start the official user service using `dockerd-rootless-setuptool.sh install --force` with a separate private `DOCKER_CONFIG`, so any installer-created CLI context cannot change the host or ordinary guest client context. Use an explicit Unix endpoint thereafter. Enable lingering only for this newly owned guest user. Keep the installed user service and VM retained after qualification. Their continued existence is explicit infrastructure, not evidence that workload cleanup passed.

## 3. Attestation and trust boundaries

Attestation must independently inspect the running user service's MainPID and `/proc` identity: real/effective/saved/fs UIDs are the unprivileged guest UID; daemon UID/GID namespace mappings include that user; the service is active with delegation. `docker info` must identify Linux arm64, `name=rootless`, cgroup v2/systemd and no CPU/memory/PIDs support warnings. Record both engine IDs, exact socket owner/mode/path, service PID, package versions, controller set, UID/GID mappings, kernel and observed VM allocation. A rootful daemon reachable through an unprivileged user's docker group fails rootless attestation.

Use a bounded strict JSON document, reject duplicate/unknown fields and bool-as-integer values, cap collection counts and strings, and use fixed local diagnostic codes. No credentials, payloads, environment dumps or arbitrary command errors appear in public diagnostics. Raw bounded command receipts stay in the private evidence directory. Attestation preserves source fingerprints and profile allocation before/after; altered code, profile or engine identities invalidate the capture.

Preserve actual Unix socket permission/special bits. Accept owner read/write with optional group read/write and sticky bit (`0600`, `0660`, `01600`, `01660`) only beneath the private0700 runtime directory; reject world access and setuid/setgid. Socket group need not equal the guest's primary group: Docker's daemon-side group may map through subordinate GIDs. Record the primary guest GID, exact GID map and socket GID; derive the unique namespace socket GID from that map. The server-container lane must use that derived group. Do not equate a mapped host GID with a container group number.

## 4. Workload qualification

Bootstrap reports `platformQualification=pending`. Before changing this to qualified, execute the existing actual Rust kernel probe against the rootless socket and prove effective CPU quota, memory/no-swap, PIDs, nonroot, read-only root, bounded noexec tmpfs, capability drop and no-new-privileges. Docker accepting create options is insufficient. A missing controller is a failure, never an accepted reduced policy.

Use the same exact source-controlled resource inventory as D102, updated only for intentional new cases. Capture/retain immutable fixture IDs before export and after import; preserve all source aliases and operator cache anchors. An explicit owned SSH Unix forwarding socket may connect host-native test binaries to the user daemon; its process group and private local socket remain owned/joined. Container-origin endpoints must be observed reachable, not copied from rootful bridge addresses. Namespace-local rootless container addresses do not establish host reachability. For server-container consumers, prove the actual mounted socket's mapped group rather than assuming the rootful socket GID.

Record every original owner PID/handle, exact test names and summaries, source/artifact hashes, all-state containers/volumes/private aliases before and after, and preservation of unrelated source resources. Fail on unexpected skipped/extra/missing resources, namespace leftovers, changed source IDs or cleanup uncertainty. Remove only exact nonce-owned empty networks. Preserve failed owners/evidence and never force-prune or delete user images. Reuse D102 performance workloads for rootless responsiveness/resource evidence; do not label the VM a dedicated performance reference.

## 5. Budgets, controls and exit

Host provisioning has one 1,500 s absolute cutoff, including VM creation (330 s maximum), guest installation (1,100 s maximum), receipt IO and attestation. Guest package commands are individually capped at 300 s, service operations at 60 s and read-only queries at 15 s. Existing Runner owns direct child process groups with its original 3 s retirement budget; a host SSH timeout does not by itself prove guest termination. The guest retains its own absolute cutoff and on-disk final outcome. A late/missing outcome is unqualified and needs read-only reconciliation before any further mutation. Attestation has a separate 90 s overall cutoff. No async unbounded subprocess dispatch.

An unprivileged process owner cannot assume signalling authority over sudo descendants. Each privileged setup command therefore runs as a uniquely named recorded root-manager transient systemd service, with Type=exec, no restart, RuntimeMaxSec ending at least6s before the caller cutoff,2s start/stop bounds, KillMode=control-group and SendSIGKILL. The manager retains and bounds the job independently of the SSH/controller. No broad unit matching or service restart is allowed. Publish/fsync a private exact unit/argv/cutoff intent before dispatch; retain the result separately afterward, including on nonzero exit. Failed intent publication prevents the effect. Host and guest entries retain the exact script bytes. A late/unknown command fails setup rather than inferring successful retirement from the outer sudo PID.

After the intent fsync, recheck that the root job's recorded lifetime plus the full6s reserve still fits the original caller cutoff. If local IO consumed that slack, fail without dispatch. Never extend the caller deadline or silently recalculate the recorded unit after its intent was published.

Normal controls must cover fresh/existing profile rejection, malformed receipt/nonce, duplicate JSON, socket substitution, rootful masquerade, PID UID mismatch, missing delegation, invalid namespace mapping, unavailable exact package, key fingerprint mismatch and before/after engine or artifact changes. Tests must not install packages, start VMs or depend on a real Docker daemon.

Exit requires actual retained rootless infrastructure, effective kernel-probe proof, the complete resource inventory with exact postconditions, relevant standard/strict/Native/security/text checks for source changes, and independent read-only review with all valid findings fixed. Provisioning/attestation can land as an explicitly incomplete dependency; the ledger must retain the full resource/platform exit until executed.

## 6. Official sources

Checked 2026-10-31: [rootless prerequisites and installation](https://docs.docker.com/engine/security/rootless/), [systemd and resource delegation](https://docs.docker.com/engine/security/rootless/tips/), [Ubuntu namespace policy and networking limitations](https://docs.docker.com/engine/security/rootless/troubleshoot/), [signed Ubuntu repository installation](https://docs.docker.com/engine/install/ubuntu/). These describe setup constraints, not Rustack qualification evidence.

Executed evidence and retained failed attempts: [rootless bootstrap review](../docs/research/workflows-rootless-platform-review.md). The installed native VZ profile has a real UID501 daemon, version-matched29.5.2 extras, distinct rootful/rootless engine IDs, effective manager delegation and derived socket namespace GID991.25 normal controls, a root-owned TERM-ignoring timeout/physical-retirement control and independent source review pass. The existing Rust kernel probe and complete79-resource inventory have not run against this daemon; platformQualification stays pending.
