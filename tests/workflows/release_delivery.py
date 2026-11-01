"""Immutable candidate sealing and promotion; registry effects are explicit only."""
import hashlib
import json
import os
from pathlib import Path
import re
import tarfile
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
import release_json as storage
from release_json import document
from linux_performance import Runner, bounded_read, deadline_guard, require
from release_qualification import archive_hash, archive_identity, assemble, digest, member
from rootless_functional import source_seal

ROOT = Path(__file__).resolve().parents[2]


def checked(owner, arguments, deadline):
    reply = owner.run(arguments, deadline, stdout_limit=65536)
    require(reply.code == 0 and reply.stderr == b"", "delivery-command-failed")
    return reply.stdout


def source_identity(owner, deadline):
    commit = checked(owner, ["git", "-C", str(ROOT), "rev-parse", "HEAD"], deadline).decode().strip()
    source = source_seal(owner, deadline)
    seal = digest(json.dumps(source, sort_keys=True, separators=(",", ":")).encode())
    return commit, seal


def seal(root, architecture, owner, deadline):
    require(architecture in ("arm64", "amd64"), "delivery-architecture")
    commit, source = source_identity(owner, deadline)
    name = "release-" + architecture + ".tar"
    size, hashed = archive_hash(member(root, name, 2 * 1024 * 1024 * 1024))
    image = "rustack/workflows-release:integration-" + architecture
    raw = checked(owner, ["docker", "image", "inspect", image, "--format",
        '{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}}}'], deadline)
    value = document(raw)
    require(set(value) == {"id", "os", "architecture"} and value["os"] == "linux"
            and value["architecture"] == architecture and re.fullmatch(r"sha256:[0-9a-f]{64}", value["id"]),
            "delivery-image-platform")
    config_digest = archive_identity(root / name, architecture, value["id"])
    if architecture == "arm64":
        from release_image import ARCHIVE_LIMIT, verify
        verify(bounded_read(root / name, ARCHIVE_LIMIT), "arm64", config_digest)
    storage.publish(root / ("candidate-" + architecture + ".json"), {"schemaVersion":1, "commit":commit,
        "sourceSealSha256":source, "architecture":architecture, "archive":name, "archiveBytes":size,
        "archiveSha256":hashed, "imageId":value["id"], "configDigest":config_digest})


def authenticated_api(owner, endpoint, deadline):
    token = os.environ.get("GH_TOKEN", "")
    require(1 <= len(token) <= 4096 and "\n" not in token and "\r" not in token, "delivery-api-credential")
    previous = owner.environment.pop("GH_TOKEN", None)
    owner.environment["GH_TOKEN"] = token
    try:
        return checked(owner, ["gh", "api", endpoint], deadline)
    finally:
        owner.environment.pop("GH_TOKEN", None)
        if previous is not None:
            owner.environment["GH_TOKEN"] = previous


def check_runs(owner, deadline):
    repository, commit = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GITHUB_SHA", "")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "delivery-repository")
    requested = (("CANDIDATE_RUN_ID", ".github/workflows/release-docker.yml"),
                 ("QUALIFICATION_RUN_ID", ".github/workflows/qualify-workflows.yml"))
    if "EVIDENCE_RUN_ID" in os.environ:
        requested = (("CANDIDATE_RUN_ID", ".github/workflows/release-docker.yml"), ("EVIDENCE_RUN_ID", ".github/workflows/workflows-evidence.yml"))
    for variable, workflow in requested:
        run = os.environ.get(variable, "")
        require(re.fullmatch(r"[1-9][0-9]{0,19}", run), "delivery-run-id")
        value = document(authenticated_api(owner, "repos/" + repository + "/actions/runs/" + run, deadline))
        require(value.get("conclusion") == "success" and value.get("status") == "completed"
                and value.get("head_sha") == commit and (workflow is None or value.get("path") == workflow)
                and value.get("head_repository", {}).get("full_name") == repository
                and value.get("event") in ("push", "workflow_dispatch"), "delivery-run-affiliation")


def verify(root, owner, deadline):
    check_runs(owner, deadline)
    report = assemble(root, owner, deadline)
    require(report["candidateRunId"] == os.environ.get("CANDIDATE_RUN_ID"), "delivery-candidate-run-differs")
    saved = document(bounded_read(root / "release-report.json", 1024 * 1024))
    require(report == saved, "delivery-report-changed")
    # Verify loaded config IDs before registry credentials are introduced.
    for architecture, value in report["candidates"].items():
        archive = member(root, value["archive"], 2 * 1024 * 1024 * 1024)
        loaded = owner.run(["docker", "image", "load", "--input", str(archive)], deadline)
        require(loaded.code == 0, "delivery-load-failed")
        raw = checked(owner, ["docker", "image", "inspect", value["imageId"], "--format", "{{.Id}}"], deadline)
        require(raw.decode().strip() == value["imageId"], "delivery-loaded-image-differs")
    return report


def pushed_digest(stdout, stderr):
    require(len(stdout) <= 65536 and len(stderr) <= 65536, "delivery-push-output-bound")
    matches = re.findall(rb"digest: (sha256:[0-9a-f]{64}) size: [0-9]+", stdout + b"\n" + stderr)
    require(len(matches) == 1, "delivery-push-digest")
    return matches[0].decode("ascii")


def registry_manifest(raw, expected_digest, expected_config):
    require(1 <= len(raw) <= 65536, "delivery-manifest-bound")
    require("sha256:" + digest(raw) == expected_digest
            or (raw.endswith(b"\n") and "sha256:" + digest(raw[:-1]) == expected_digest), "delivery-manifest-digest-differs")
    value = document(raw)
    require(value.get("schemaVersion") == 2 and value.get("mediaType") in
            ("application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json")
            and isinstance(value.get("config"), dict) and value["config"].get("digest") == expected_config,
            "delivery-unqualified-registry-config")


def promote(root, owner, deadline):
    report = verify(root, owner, deadline)
    repository = os.environ["GITHUB_REPOSITORY"].lower()
    version = os.environ.get("RELEASE_VERSION", "")
    require(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is not None and len(version) <= 32,
            "delivery-release-version")
    image = "ghcr.io/" + repository.rsplit("/", 1)[0] + "/rustack"
    commit = report["commit"]
    digests = []
    for architecture, value in sorted(report["candidates"].items()):
        target = image + ":candidate-" + commit + "-" + architecture
        checked(owner, ["docker", "image", "tag", value["imageId"], target], deadline)
        reply = owner.run(["docker", "image", "push", target], deadline, stdout_limit=65536)
        require(reply.code == 0, "delivery-push-failed")
        hashed = pushed_digest(reply.stdout, reply.stderr)
        raw = checked(owner, ["docker", "buildx", "imagetools", "inspect", image + "@" + hashed, "--raw"], deadline)
        registry_manifest(raw, hashed, value["configDigest"])
        digests.append(image + "@" + hashed)
    major, minor, _ = version.split(".")
    arguments = ["docker", "buildx", "imagetools", "create"]
    for tag in (version, major + "." + minor, major, "latest"):
        arguments += ["--tag", image + ":" + tag]
    reply = owner.run(arguments + digests, deadline)
    require(reply.code == 0, "delivery-manifest-failed")
    storage.publish(root / "promotion.json", {"schemaVersion":1, "commit":commit, "version":version,
        "registryManifests":digests, "sourceImageIds":{key: value["imageId"] for key, value in report["candidates"].items()},
        "allChildGroupsJoined":not owner.unsettled})


def main():
    try:
        if len(sys.argv) == 2 and sys.argv[1] == "input-runs":
            owner = Runner(os.environ)
            with deadline_guard(60, owner) as deadline:
                check_runs(owner, deadline)
            return 0
        require(len(sys.argv) >= 3, "delivery-arguments")
        mode, root = sys.argv[1], Path(sys.argv[2]).resolve(strict=True)
        owner = Runner(os.environ)
        with deadline_guard(900, owner) as deadline:
            if mode == "seal":
                require(len(sys.argv) == 4, "delivery-seal-arguments")
                seal(root, sys.argv[3], owner, deadline)
            elif mode in ("verify", "promote"):
                require(len(sys.argv) == 3, "delivery-promote-arguments")
                if mode == "verify": verify(root, owner, deadline)
                else: promote(root, owner, deadline)
            else: require(False, "delivery-mode")
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError, RecursionError, tarfile.TarError):
        print("release-delivery-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
