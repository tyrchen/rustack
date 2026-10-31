# Workflows 50: Pulumi Image-Lambda Consumer Design

Status: Executable design; implementation and actual acceptance pending · Date: 2026-10-30 · Work item: D97 /5.3d

## 1. Outcome and dependencies

Qualify a normal Pulumi AWS provider's STANDARD StateMachine create, explicit refresh, in-place definition/tag update and destroy against the ordinary strict-auth Rustack binary. Independently execute the resulting definition through a real image Lambda before and after update. Existing callback/runtime/API semantics stay unchanged.

Depends on D94's pinned Node image, D95's qualified function SDK environment, D96's retained SDK/server consumer and the [Pulumi process study](../docs/research/study-pulumi-process-ownership.md). The [survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md) and [relay spike](../docs/research/spike-lambda-container-relay.md) retain the original image/network contracts. This slice closes the IaC consumer only; server-in-container, native-x86/reference, Desktop/rootless and final5.4 remain required.

Use a dedicated managed Image Lambda as the IaC runner. Its handler launches the actual pinned CLI, language host and provider inside the already owned function PID namespace/cgroup. A separate pinned Node function supplies the target computation. Host tooling is the existing directly owned Python SDK child; it spawns no Pulumi process. This avoids assuming that Pulumi's independently grouped plugins share one host process group.

## 2. Versions, image and provisioning

Checked2026-10-30 through primary documentation and npm metadata:

| Component | Required pin / source |
|---|---|
| Function base | Existing Node24 index `public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79` |
| CLI and core SDK |3.273.0; [CLI documentation](https://www.pulumi.com/docs/install/) |
| AWS provider/SDK |7.52.1; [StateMachine reference](https://www.pulumi.com/registry/packages/aws/api-docs/sfn/statemachine/) |
| SDK npm integrity | `@pulumi/pulumi`: `sha512-Yh2k0Kry4u5LM730KUO5HZFnqHdlW9ypoQ8zgO0Tqam1G6urFXjIUTUaLxvWrdsm6gKCoyVS6CPdeOGcV+Msvg==`; `@pulumi/aws`: `sha512-EkiZJacWBqwxq+LLeiDA131OWaoFGAnPKKx8aCpcn6lWKyYNqdZIgUhY8OrHefYtirWL5rnabDuPJ9YuluTPQQ==` |
| Runtime control client | Actual bundled DynamoDB3.1105.0 default-provider client, independently source/version checked |
| Linux CLI SHA256 | arm64: `c2e1b600b37d9d84551259b6f391f5035e336dfe63c49412f22d855a91995ed5`; amd64 (`linux-x64` archive): `ec08f86671fd58f0a186540e1cee00066393beaf5c6716b0075f690919800432` |
| Linux provider SHA256 | arm64: `1976366b71dc08d8827a04fbb8eaf50a0d1ec5757726e593b6d7c27e7636daf8`; amd64: `2e705824a2103dd6e3dc66744ba2d55cad55d665476b3ffcd8ce024cf4cab081` |
| Upstream lifecycle | `vendors/pulumi` @ `874c77a73039cf013d6a685a15293843de2b0e01` |

The official CLI [checksum manifest](https://get.pulumi.com/releases/sdk/pulumi-3.273.0-checksums.txt) supplies its SHA256 pins. Provider archive SHA256 values come from the official [release API asset digests](https://api.github.com/repos/pulumi/pulumi-aws/releases/tags/v7.52.1); its [legacy checksum file](https://github.com/pulumi/pulumi-aws/releases/download/v7.52.1/pulumi-aws_7.52.1_checksums.txt) contains SHA1 entries and is not substituted for those SHA256 pins. That checksum file itself matches the release API's published SHA256 `8be82022258a646741d4558d2117ac07e462d4b637e034498a22d089035065cf`. Source-only metadata reads are not image execution.

Add a separately tagged `rustack-pulumi:<selected-architecture>` image. Build only for that selected daemon's Linux architecture. Retain the official base, required Linux CLI/language binaries, the exact AWS plugin and lockfile-resolved Node dependencies in the image; no runtime download/install. Use published official checksums for CLI/plugin artifacts and npm lockfile integrity; reject missing/mismatched artifacts before executable use. Network downloads have explicit time/byte caps:512MiB per archive,2GiB expanded/tool archive,4,096 members,512-byte member paths, no link/special/traversal extraction. Extract only the required tool roots; do not invoke a downloaded installer shell. Record actual binary/package hashes in a small image provenance file and compare runtime versions/hashes during real invocation. The final unpacked fixture image must be≤4GiB.

Make owns provisioning and test entry points. `lambda-pulumi-fixture-image` reuses the existing explicit immutable Node source provisioner and builder; `test-workflows-iac-fixture` runs program/protocol/privacy controls without Docker; `test-workflows-iac-docker` executes the two exact actual resources. Add these to the sequential complete fixture/resource gate. No global Pulumi CLI, plugin, npm project, cloud login or existing user state is changed. Keep preceding fixture images and code artifacts intact.

Do not copy the host's plugin cache or Node modules into the Linux function. Build/install under the selected Linux architecture with scripts disabled for locked npm dependencies. Resolve any build requirement through a pinned explicit step, never an unrecorded postinstall. The prior observed global CLI3.245.0 is not this consumer's toolchain.

## 3. Runtime composition and limits

```text
 SDK host             controller workflow       managed tool Lambda       target workflow / Node Lambda
    │                         │                         │                              │
 1  ├─ Start controller ─────►│─ real observed Invoke ─►│ CLI + language + provider     │
    │                         │                         ├─ Create target via SFN API ──►│
    │◄─ safe created-held row ┼─────────────────────────┤ provider still alive          │
 2  ├─ independently Start target / verify42 ─────────────────────────────────────────►│
    ├─ grant stage via DDB ────────────────────────────►│ up completes / refresh        │
    │◄─ updated-held row ─────┼─────────────────────────┤ same ARN, new definition      │
 3  ├─ independently Start target / verify46 ─────────────────────────────────────────►│
    ├─ grant stage ────────────────────────────────────►│ unsupported updates / recover │
    │◄─ pre-destroy row ─────┼─────────────────────────┤ audit actual encrypted inputs │
 4  ├─ audit checkpoint+logs; grant destroy ────────────►│ destroy, then hold            │
    ├─ verify API absence+post-destroy state; grant rm ►│ stack rm, then hold           │
    ├─ verify active checkpoint absent; grant finish ►│ result / cleanup / return     │
    │◄─ controller SUCCEEDED ┤◄─ invocation settled ────┤                              │
    │                         │                         │                              │
 fail/Stop while created-held ├─ cancel exact call ────►│ whole function+relay retire   │
    │ confirm tool pair absent and business pair preserved; no CLI/PGID shortcut      │
```

Use a dedicated IaC test profile. Enable the existing four-service cohort and exact qualified SDK pair. Two function environments are required: tool memory1,024MiB plus64MiB relay, business256MiB plus64MiB relay, within lambdaDockerMemoryBytes1,610,612,736 (1,536MiB). Keep other profiles unchanged. Use two workflow dispatch slots because the controller holds one real invocation while the independent target needs the other. Keep the ordinary PID/nonroot/read-only/network policy and512MiB noexec temporary storage; preinstall executable tools in the read-only image. `GOMAXPROCS=1` and a small explicit Node worker pool constrain CLI/plugin threads within the existing function PID policy.

The tool function has timeout720s, controller Task780s/execution840s, SDK consumer an original1,020s cutoff, per CLI90s and per DDB SDK send4s. Every CLI/phase wait clamps to the same remaining handler budget and fresh pre/post observation fences. Init remains20s. Missing readiness, source, socket, network or tool version fails rather than skips. Keep the default shared SDK consumer cutoff unchanged; only this named profile receives the larger explicitly bounded cutoff.

## 4. Tool handler, program and control protocol

Deploy the tool with `PackageType=Image` and a fixed handler CMD, and the business function with D94's `node.handler`. Set/read back reserved concurrency1 on each before prewarming. The tool handler validates≤8KiB payload, exact field allowlists, canonical nonce/request identities, function/machine/table names≤64/80/128 ASCII bytes, ARNs≤256 bytes and stage/value enums before IO. Use fresh per-invocation workspace `/tmp/iac-<actual-request-UUID>`; no shared mutable warm workspace.

The handler's private PULUMI_HOME/backend/project/log paths stay under that workspace. Its plugin directory may reference only the verified read-only image plugin root. Set explicit local backend URL, a fresh per-invocation passphrase, disabled automatic plugin acquisition and ignored ambient plugins; clear cloud tokens/profile/session/role/proxy selectors. Do not change HOME. The program declares one explicit provider and one StateMachine with a fixed name/role/region/type and supported definition/tags. Use the provider's documented `sfn` endpoint override to the injected qualified origin, validation/metadata/account lookup skip flags and maxRetries0. Wrap both explicit provider credentials sourced from the injected function environment with `pulumi.secret`, so any checked/defaulted provider inputs are encrypted in local checkpoints. No credentials appear in argv or program source. [Provider inputs](https://www.pulumi.com/registry/packages/aws/api-docs/provider/), [backend/refresh semantics](https://www.pulumi.com/docs/iac/concepts/state-and-backends/), [CLI environment controls](https://www.pulumi.com/docs/iac/cli/environment-variables/).

The ordinary provider declaration is real; do not mock RegisterResource/RPC or synthesize state files. An asynchronous output gate after the actual machine ARN resolves holds the first/second up operation while provider and language processes remain alive. Gate-bearing up commands require `--yes --skip-preview --non-interactive`; the program additionally checks `pulumi.runtime.isDryRun()` and never writes/reports/waits for a gate in a preview. Grant paths bind the invocation nonce, mode, command ordinal and stage; recovery ups have gating disabled so a previous grant cannot release another command. It writes a small private phase file; the tool handler reports a safe DDB row and waits for the corresponding host grant before releasing that program gate. Validate actual descendant CLI/language/provider PIDs and process groups through bounded `/proc` observations (at most256 entries,4KiB per observed file,32 reported PIDs); do not read process environments. A held row must prove the CLI and both actual plugin classes are still running before the host's Stop test.

Control rows contain only nonce, actual invocation UUID, mode/stage, target ARN, revision/definition hashes, values, versions, bounded PID/group identities and booleans. No pair, passphrase, token, raw CLI output or checkpoint content goes into DDB. Bound≤64 rows/2KiB each; gates bind nonce+mode+stage and cannot release another run. The host uses existing real strict SDKs and safe polling. The SDK child spawns no OS subprocess.

## 5. Positive lifecycle and independent oracle

Use one actual local stack and at most16 CLI commands. The expected sequence is version, local login, stack init, initial up, no-change refresh, definition/tag update up, no-change refresh, three rejected unsupported ups each followed by a valid no-change recovery up, destroy and stack removal. Use noninteractive explicit stack/cwd options and fixed argv; every up bypasses previews with --yes --skip-preview. No shell or hidden provider retries. Include a pure dry-run/known-ARN control proving no gate is reported or held. Capture stdout/stderr separately with≤1MiB each per command/16MiB aggregate and fixed redacted host errors.

At initial held up, the host describes the target, checks STANDARD/name/role/definition/tags and starts it with an independent literal expected42. The definition sends value21 to the real business function. After releasing up and explicit refresh, require unchanged ARN/revision/definition and successful no-change CLI status. Update value23 and tags in place; require same ARN, changed revision/exact intended definition and independent computed46 with actual request UUID, Node24 version and selected architecture. Refresh again without changes. Derive expectations from the fixed fixture contract, not engine/Pulumi-produced desired outputs.

For unsupported fields, run actual provider updates with publish=true, valid AWS-shaped ALL logging+destination, and CUSTOMER_MANAGED_KMS_KEY+valid-shaped KMS ARN. Each CLI must fail for the modeled ValidationException; reject a generic crash, missing tool or local schema typo as acceptance. Independently verify the last valid target ARN/definition/revision/tags remain unchanged and executable. Restore the supported program and require no-change recovery before the next negative. No KMS/log resource is provisioned and no unsupported field is silently discarded.

Before destroy, hold the still-active tool invocation and independently audit the actual current post-up checkpoint while the provider resource and both encrypted credential inputs still exist. Bind its path/byte count/SHA256 to the separately observed phase. Do not substitute a backup, empty state or post-destroy checkpoint for this credential audit.

Destroy must actually remove the target: independently wait for StateMachineDoesNotExist and absence from the supported list. After destroy but before stack removal, hold and independently audit a separate actual current checkpoint with the target absent, alongside the API absence. Then grant stack removal; require its actual successful command result and absence of the active checkpoint before granting finish. Backups are not active state, and historical files cannot prove current resource presence/absence. Keep pre-destroy, post-destroy and post-removal identities and expectations distinct. Do not equate a successful preview, missing resource declaration or stack-file deletion with API destroy. The controller is a separate fixture-owned machine and must not be miscounted as an undeleted Pulumi target.

## 6. Failure ownership and privacy

The second exact resource holds initial up after real target creation with actual CLI/language/provider processes alive. The host prewarms the business function, observes both exact function/relay pairs, then Stops the controller with a fixed error. Require ABORTED/Stopped, disappearance of the tool's exact pair and preservation of the business pair. The kernel container lifetime covers independently grouped plugins; CLI PID absence alone is insufficient. Invoke the business function again to prove continued health. Remove only the known fixture target/controller through SDK cleanup after physical retirement; this is compensating fixture cleanup, not a successful Pulumi destroy claim.

The tool handler always joins each accepted CLI child through success/error/timeout and closes clients/timers. It may request graceful CLI cancellation and then terminate that CLI; the retained Lambda owner remains responsible for the entire function namespace. EOF/error in the SDK observer cannot release physical environment or invocation charges early. Reuse the existing image SDK Server/child owner; do not add an unowned host CLI or second runtime lifecycle implementation.

At each pre-destroy, post-destroy and post-removal gate, the tool is still actively invoking. Independently read the exact tool function container's real backend and captured-log directories through bounded read-only Docker archive GETs; verify original engine, exact full CID/source/owner/run/function labels and request-UUID path before each observation. Never execute an archive helper inside the function or extract host files. Cap TAR16MiB total/128 entries/depth8/path512 bytes, regular JSON/log files2MiB each; reject links/special/traversal/truncated/trailing/over-limit data. Scan complete bounded bytes and decoded JSON string values for both injected credential values with fixed errors. In the pre-destroy current checkpoint, require both actual encrypted provider-secret input fields without decrypting them; the host never needs the passphrase. Require real pre-destroy provider/checkpoint/log content, distinct post-destroy current state without the target and actual completed command records. After successful stack rm, explicitly require active-checkpoint absence; an empty/unrelated archive never substitutes for either earlier audit. Deliberate plaintext/escaped-secret and malformed/archive-cap controls prove the oracle.

Finally grant tool return, join the controller invocation, stop/join the server and audit exact Docker resources/source IDs. Retain private evidence on any uncertainty; no global pruning or unrelated-container cleanup. Failed updates retain the last valid resource; an uncertain effect remains owned until its actual physical outcome is settled.

## 7. Frozen inventory, automation and exit

Add exactly these ignored ordinary resources before first actual execution:

- `test_should_manage_a_real_image_workflow_through_pulumi_lifecycle`
- `test_should_retire_the_pulumi_plugin_tree_after_controller_stop`

Preserve all existing names:39 ordinary/72 total/eight suites. New image adds one source to D96's twelve, so each lane preserves thirteen actual IDs (the existing Node base is reused). Freeze the literal CLI/stage/provider-error catalog and image/tool hashes before qualifying execution. Keep native-arm64 VZ and emulated-amd64 QEMU functional scope explicit.

Dependency order: (1) pinned study/design and independent preflight; (2) checksummed image/package provisioning and pure program/control/privacy tests; (3) constrained profile and shared owner cutoff; (4) positive and active-plugin Stop consumers with independent SDK/archive oracles; (5) frozen inventories/focused resources; (6) full72 on both lanes, complete standard/strict/Native/security/text gates and exact zero-remnant/thirteen-source/two-owned-empty-bridge postconditions; (7) independent final source/evidence review, guide/index reconciliation and coherent commit. New dependencies require appropriate npm audit plus existing cargo audit/deny; do not suppress advisories to make this fixture pass.

This design is not implementation or acceptance. No D97 function/image/resource has run. D96's committed qualification remains historical and unchanged; use new D97 evidence paths. Task5.3d closes only after every stated actual lifecycle, negative, privacy and ownership criterion passes. The full M0–M4 objective stays active afterward.
