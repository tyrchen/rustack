# Releasing Rustack

Crates, the Docker distribution and GitHub Action code have separate publication
steps. A GitHub release or successful source-build smoke does not establish image
qualification. Complete Desktop, Linux/rootless ARM and dedicated ARM reference
evidence remains required by the [qualification contract](../specs/rustack-workflows-59-release-qualification-design.md).

## Prepare and verify the crate release

The product release is `v0.12.0`. The explicit publish set is
[release-packages.txt](../release-packages.txt): 38 existing packages move from
0.11.0 to 0.12.0; `rustack-core` and `rustack-cloudfront-core` move to 0.11.0;
`rustack-lambda-model` and `rustack-cloudfront-model` move to 0.10.0. Four new
Lambda runtime/Step Functions packages first publish at their existing 0.9.0.
Unchanged model/XML packages retain
their published versions; the workspace default stays at 0.9.0.

The 47th package is `squib-loader 0.2.1`. Its source is the checksum-recorded
upstream 0.2.0 source in `patches/squib-loader`; its package version and
zstd dependency constraint change, and its original `.cargo_vcs_info.json` and
`Cargo.toml.orig` are excluded from packaging because Cargo generates those
reserved files itself.
Original manifests, VCS identity, source hashes and license notice remain
recorded in `provenance.json`. Lambda core
directly constrains `squib-loader ~0.2.1` so consumers outside this workspace also
select the fixed loader. This dependency constraint is intentional even though
Lambda calls the loader through Squib. Cargo strips root patches when packaging;
the loader must therefore be published with the Rustack release, rather than
relying on the workspace overlay. Publishing requires ownership of this crate.

```bash
make release-check test-release
cargo build
cargo test
cargo +nightly fmt
make release-clippy
make verify-workflows-compression audit deny
make publish-dry-run
```

`make verify-release` runs this verification sequence and the release-evidence
negative controls together. `release-clippy` runs the required warnings-as-errors
check and the pedantic check with the four existing workspace exemptions for
generated documentation, module names and missing error/panic documentation
sections. These match `Cargo.toml`; no new lint exemption is introduced.

The dry run packages the complete set together and verifies registry-normalized
dependencies, including unpublished workspace packages, then audits the packaged
CLI lockfile. The package audit catches old published model dependencies that a
workspace-only audit cannot observe. Pre-publication yank lookups for staged
versions may report that the crate/version is not yet on crates.io; vulnerability
findings still fail the command. It uploads nothing and
allows uncommitted preparation changes. Review and commit all manifests,
`Cargo.lock`, changelog and release tooling before the actual publish:

```bash
cargo login
make publish
make release
```

`make publish` first repeats the dry-run/package audit, then uses Cargo's
multi-package dependency ordering and the locked, all-features package build.
Its default is the reviewed release set, not all
workspace members. A deliberately narrower set can be passed as
`PUBLISH_PACKAGES="crate-a crate-b"`; include all unpublished dependencies. If a
publish partly succeeds, check the registry and rerun with only the remaining
packages. Crate versions already uploaded cannot be replaced.

`make release` requires a clean master checkout and a changelog entry matching the
CLI version. It creates an annotated full-version tag on that exact commit and
atomically pushes master and the tag. It never generates a second changelog
commit after tagging or moves an existing full-version tag to another commit.
The tag starts build/Action CI and candidate-image construction. The build
workflow creates GitHub release notes; that release can exist while Docker
qualification is still pending.

## Qualify and promote the exact image

The full-version tag triggers `release-docker.yml`, which builds arm64/amd64
archives without registry login or push. Record its successful run ID. Execute
the required lanes against those exact images at the same release commit; stage
the complete private evidence bundle on the `self-hosted, rustack-qualification`
runner and set the repository variable `WORKFLOWS_EVIDENCE_ROOT` to its absolute
path. The required lanes are `rust`, `native`, `linux-arm64`, `rootless-arm64`,
`desktop`, `reference` and `release`. Historical receipts from another source
cohort cannot satisfy this gate. Linux x86_64 execution remains waived; the amd64
artifact is a distribution build.

Run each command only after its preceding workflow has succeeded and record the
resulting run ID. All dispatches must use the same release ref:

```bash
gh workflow run workflows-evidence.yml --ref v0.12.0 \
  -f candidate_run_id="$CANDIDATE_RUN_ID"

gh workflow run qualify-workflows.yml --ref v0.12.0 \
  -f candidate_run_id="$CANDIDATE_RUN_ID" -f evidence_run_id="$EVIDENCE_RUN_ID"

gh workflow run promote-docker.yml --ref v0.12.0 \
  -f candidate_run_id="$CANDIDATE_RUN_ID" \
  -f qualification_run_id="$QUALIFICATION_RUN_ID" -f version=0.12.0
```

Missing infrastructure or evidence blocks promotion. Promotion rechecks raw
receipts, run/source affiliations, image/archive identities and the CLI version
before registry login. It publishes the same archives without rebuilding and
creates `ghcr.io/tyrchen/rustack:0.12.0`, `:0.12`, `:0` and `:latest` from verified
registry digests. Retain `workflows-promotion-receipt` with the qualified bundle.

## Update the Action

After successful image promotion, a separate job runs the repository's actual
composite Action against `image-tag: 0.12.0` and binds its running container image
ID to the promotion receipt from this workflow run. Only after both checks succeed
does it create or update `v0` and `v0.12` on the promoted commit. This job holds
contents-write permission; the image promotion job does not. The full-version
tag must already resolve to the same commit. Concurrent promotions are serialized
across image publication and alias updates. Alias tag pushes do not match the
full-version-only workflow filters or git-cliff tag pattern.

Consumers can pin both artifacts:

```yaml
- uses: tyrchen/rustack@v0.12.0
  with:
    image-tag: "0.12.0"
```

Use the version only after promotion. `@v0` tracks Action code independently of
the `image-tag` input, whose default is `latest`. The Action invokes the new
`/_health/ready` endpoint and therefore needs a compatible image. It does not
mount a Docker socket or enable the Docker Lambda executor; configure that
topology explicitly as described in the [workflow guide](./workflows-guide.md).
