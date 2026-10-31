# Study: Pulumi CLI and plugin process ownership

Status: Complete · Date: 2026-10-30 · Vendor pin: `vendors/pulumi` @ `874c77a73039cf013d6a685a15293843de2b0e01` (`v3.273.0`)

## Question and decision

Can the D97 IaC consumer safely treat a directly owned Pulumi CLI PID or its process group as ownership of every language/provider process?

**No.** Plugins create independent process groups. Normal Pulumi shutdown owns plugin cancellation and pipe drains, but killing the CLI's group alone does not cover those groups. Run the pinned CLI and plugins inside a managed Image Lambda's existing PID namespace/cgroup. Retain the real Lambda invocation above that process tree, and qualify controller Stop while actual provider/language processes are alive. A second image function supplies the independently observed business computation. This adopts the existing Docker lifecycle instead of adding a host process-tree supervisor.

This study establishes source behavior and the resulting implementation constraint. It does not claim an actual Pulumi lifecycle, CLI binary/container acceptance or process-termination experiment; those are D97's required consumers.

## Architecture map

```text
┌─ Pulumi CLI / backend cancellation scope ─────────────────────────┐
│ first signal: cancel        second signal: terminate              │
│ ┌─ plugin Context / Host ───────────────────────────────────────┐ │
│ │ retained Plugin handles: RPC connection, kill, wait, pipes     │ │
│ │   ├── language host process: its own PGID ── Node program     │ │
│ │   └── resource provider process: its own PGID ── AWS API      │ │
│ │                                                              │ │
│ │ Close → interrupt plugin group → existing wait promise        │ │
│ │       → five-second allowance → group/process kill if needed │ │
│ │       → drain attached stdout/stderr before releasing sinks  │ │
│ └──────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
Outer CLI group kill alone does not run these independent Close paths.
```

The map describes upstream's ordinary process boundaries. The downstream managed-image choice is specified separately in the [D97 design](../../specs/rustack-workflows-50-iac-design.md).

## Source map and hot path

The upstream [README](../../vendors/pulumi/README.md) describes CLI/SDK-based infrastructure management. The relevant runtime boundary is the CLI's Go plugin abstraction, rather than the Node resource declaration alone.

1. [`Plugin`](../../vendors/pulumi/pkg/resource/plugin/plugin.go#L114) retains RPC connection, streams, a kill function and a wait function. These are lifetime handles; the resource's ARN is not a process-ownership handle. Its internal diagnostic buffer uses a mutex and can emit unstructured plugin output after premature exit. Rustack must keep fixture diagnostics bounded/private and avoid forwarding arbitrary raw provider errors.
2. The executable path calls `exec.Command`, [`RegisterProcessGroup`](../../vendors/pulumi/pkg/resource/plugin/plugin.go#L597), then `Start`. One goroutine waits for the child and closes its output writers. The completion promise is shared with shutdown; the parent does not create a replacement process to observe completion.
3. The Unix [group helper](../../vendors/pulumi/sdk/go/common/util/cmdutil/child_unix.go#L48) sets `Setpgid=true`. Its [kill path](../../vendors/pulumi/sdk/go/common/util/cmdutil/child_unix.go#L27) signals the negative plugin PID, targeting that plugin's group. This does not imply membership in the CLI's own group.
4. The plugin's [once-only kill closure](../../vendors/pulumi/pkg/resource/plugin/plugin.go#L640) first interrupts the plugin group, gives its existing wait promise five seconds, then kills the group and process on expiry. This joins the same launched plugin; a new status lookup is not a substitute.
5. [`Plugin.Close`](../../vendors/pulumi/pkg/resource/plugin/plugin.go#L746) closes its RPC connection, invokes that kill closure and waits for attached output/error drain channels. Sinks must outlive these drains. Premature-crash diagnostics may include unstructured output, so a successful process exit is not itself a privacy audit.
6. [`Context.Close`](../../vendors/pulumi/pkg/resource/plugin/context.go#L347) synchronously releases that context's plugins through the host before completing its sinks and tracing lifetime. The [Host contract](../../vendors/pulumi/pkg/resource/plugin/host.go#L82) distinguishes context release, host lifetime and advisory provider cancellation.

The actual CLI [default host](../../vendors/pulumi/pkg/host/host.go#L573) serializes context release with plugin-load request channels, collects matching providers and reference-counted shared runtimes, and joins concurrent close operations. Loads still in flight can remain cached until host close. Its [final Close](../../vendors/pulumi/pkg/host/host.go#L724) takes the exclusive plugin lock, then joins resource/analyzer shutdown before language hosts and closes context servers last. This is why caller-owned host closure remains a backstop, rather than assuming every context release has already seen every load. The [Node language host](../../vendors/pulumi/sdk/nodejs/cmd/pulumi-language-nodejs/main.go#L988) also owns its program subprocess: cancellation interrupts it, with a five-second WaitDelay.

For a finite set of launched plugins, ordinary close traverses retained handles and their output completion, rather than scanning arbitrary host processes. This source reading supplies no benchmark or universal bound for a hung CLI/plugin. The five-second allowance is a per-plugin implementation detail, not a Rustack whole-command deadline.

## Cancellation and failure boundaries

The backend [cancellation scope](../../vendors/pulumi/pkg/backend/cancellation_scope.go#L52) registers SIGINT and SIGTERM. The first signal requests cancellation; a subsequent signal requests termination. Its [Close](../../vendors/pulumi/pkg/backend/cancellation_scope.go#L42) unregisters notification and joins the existing signal goroutine. Immediate termination may leave remote resources; local process cleanup and API resource cleanup therefore need separate assertions.

The [generic termination helper](../../vendors/pulumi/sdk/go/common/util/cmdutil/term.go#L98) interrupts a registered group, waits through a caller-supplied cooldown and escalates on expiry. Do not transplant its direct process/group assumption into a claim that every independently grouped plugin belongs to one outer CLI group.

D97's ordinary SDK/ASL server still owns remote state machines and both function resources. A successful IaC destroy must remove the exact target resource. An intentionally interrupted create/update must preserve its private state evidence; the fixture explicitly removes only its known target/controller resources after the tool invocation has physically retired. Process termination must not silently masquerade as a successful IaC destroy. The DIY backend [refuses normal stack removal while non-root resources remain](../../vendors/pulumi/pkg/backend/diy/backend.go#L1092); its [removal path](../../vendors/pulumi/pkg/backend/diy/state.go#L414) moves/deletes current checkpoints and removes history. Audit encrypted provider inputs while the pre-destroy checkpoint still contains the provider, then separately verify post-destroy state and post-removal absence.

## Adopt and avoid

Adopt retained wait/drain ownership and distinct cancellation versus termination. Use fixed CLI/provider/SDK versions and a private local backend, with automatic plugin acquisition and ambient plugin discovery disabled during invocation. Read the actual plugin processes while the tool handler is causally gated, then require disappearance of the tool's exact function/relay pair after Stop while the business pair survives.

Avoid relying on CLI PID absence, one process-group signal, elapsed sleeps or a successful preview as full acceptance. Do not mount the Docker socket into the tool function or let the tool own its container. Keep CLI/plugins preinstalled in the read-only image; writable private state belongs in the normal bounded `/tmp` profile. The image build and actual D97 lifecycle must verify compatibility with that noexec temporary mount.

## Versions and primary API evidence

Checked2026-10-30: the official [CLI installer documentation](https://www.pulumi.com/docs/install/) reports3.273.0; npm metadata independently reports `@pulumi/pulumi`3.273.0 and `@pulumi/aws`7.52.1 with published integrity values. Existing global CLI3.245.0 and unrelated cached plugins are not the new qualification toolchain.

The current [StateMachine API](https://www.pulumi.com/registry/packages/aws/api-docs/sfn/statemachine/) supports the basic STANDARD resource. The [provider reference](https://www.pulumi.com/registry/packages/aws/api-docs/provider/) exposes an `sfn` endpoint override and local-implementation skip flags. [Backend documentation](https://www.pulumi.com/docs/iac/concepts/state-and-backends/) distinguishes explicit refresh from ordinary updates; [CLI environment documentation](https://www.pulumi.com/docs/iac/cli/environment-variables/) documents isolated `PULUMI_HOME` and disabled ambient/automatic plugin acquisition. These are API inputs, not evidence that Rustack has consumed them.

## Remaining executable gates

The D97 design must qualify the pinned image build, actual create/refresh/in-place update/destroy, independent42/46 computations, explicit unsupported-profile rejection, checkpoint/diagnostic credential omission and Stop with active language/provider processes. No source-only result closes task5.3d or final M4 acceptance.
