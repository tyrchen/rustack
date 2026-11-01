"""Release-set checks, exact-commit tags and post-promotion Action aliases."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def run(arguments):
    return subprocess.check_output(arguments, cwd=ROOT, text=True, timeout=60).strip()


def release_version(metadata):
    packages = [p for p in metadata["packages"] if p["name"] == "rustack-cli"]
    if len(packages) != 1 or VERSION.fullmatch(packages[0]["version"]) is None:
        raise ValueError("release requires one CLI with a numeric release version")
    return packages[0]["version"]


def check(metadata, selected):
    packages = {p["name"]: p for p in metadata["packages"]}
    if not selected or len(selected) != len(set(selected)):
        raise ValueError("release set must be nonempty and unique")
    for name in selected:
        if name not in packages or packages[name].get("publish") == []:
            raise ValueError("release set contains an unknown or private package: " + name)
    for name, package in packages.items():
        if package.get("publish") == []:
            continue
        if name not in selected and any(d["name"] in selected and d.get("kind") != "dev"
                                        for d in package["dependencies"]):
            raise ValueError("release set omits dependent: " + name)
    loader = packages.get("squib-loader", {})
    core = packages.get("rustack-lambda-core", {})
    constraints = [d["req"] for d in core.get("dependencies", []) if d["name"] == "squib-loader"]
    if ("squib-loader" not in selected or loader.get("version") != "0.2.1"
            or constraints != ["~0.2.1"]):
        raise ValueError("release must include the registry-safe Squib loader security floor")
    return release_version(metadata)


def tag(version, execute=run):
    if execute(["git", "status", "--porcelain"]):
        raise ValueError("commit all release changes before tagging")
    if execute(["git", "branch", "--show-current"]) != "master":
        raise ValueError("release tags must be created on master")
    if "## [" + version + "]" not in (ROOT / "CHANGELOG.md").read_text():
        raise ValueError("changelog is missing the release version")
    commit = execute(["git", "rev-parse", "HEAD"])
    name = "v" + version
    existing = execute(["git", "tag", "--list", name])
    if existing:
        if execute(["git", "rev-parse", name + "^{commit}"]) != commit:
            raise ValueError("release tag already identifies a different commit")
    else:
        execute(["git", "tag", "--annotate", name, "--message", "Release " + name, commit])


def promotion_identity(version, environment, execute=run):
    """Fence version, checkout and immutable release tag before registry writes."""
    repository = environment.get("GITHUB_REPOSITORY", "")
    commit = environment.get("GITHUB_SHA", "")
    if (VERSION.fullmatch(version) is None
            or len(repository) > 200
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", repository) is None
            or re.fullmatch(r"[0-9a-f]{40}", commit) is None):
        raise ValueError("invalid Action release identity")
    if environment.get("RELEASE_VERSION") != version:
        raise ValueError("promotion version differs from the CLI version")
    if execute(["git", "rev-parse", "HEAD"]) != commit:
        raise ValueError("Action checkout differs from the promoted source")
    endpoint = "repos/" + repository
    tagged = json.loads(execute(["gh", "api", endpoint + "/commits/v" + version]))
    if tagged.get("sha") != commit:
        raise ValueError("immutable release tag differs from the promoted source")
    return endpoint, commit


def action_aliases(version, environment, execute=run):
    # This command is invoked only after exact-image promotion and the actual Action smoke.
    endpoint, commit = promotion_identity(version, environment, execute)
    major, minor, _ = version.split(".")
    for alias in ("v" + major, "v" + major + "." + minor):
        refs = json.loads(execute(["gh", "api", endpoint + "/git/matching-refs/tags/" + alias]))
        exact = [r for r in refs if r.get("ref") == "refs/tags/" + alias]
        if len(exact) > 1:
            raise ValueError("ambiguous Action alias")
        if exact:
            if exact[0].get("object", {}).get("sha") == commit:
                continue
            execute(["gh", "api", "--method", "PATCH", endpoint + "/git/refs/tags/" + alias,
                     "-f", "sha=" + commit, "-F", "force=true"])
        else:
            execute(["gh", "api", "--method", "POST", endpoint + "/git/refs",
                     "-f", "ref=refs/tags/" + alias, "-f", "sha=" + commit])


def promoted_image(version, commit, receipt, execute=run):
    """Bind the actual Action container to the exact image promoted by this run."""
    expected = {"schemaVersion", "commit", "version", "registryManifests",
                "sourceImageIds", "allChildGroupsJoined"}
    if (not isinstance(receipt, dict) or set(receipt) != expected
            or type(receipt["schemaVersion"]) is not int or receipt["schemaVersion"] != 1 or receipt["commit"] != commit
            or receipt["version"] != version or receipt["allChildGroupsJoined"] is not True):
        raise ValueError("promotion receipt differs from the Action release")
    images = receipt["sourceImageIds"]
    if (not isinstance(images, dict) or set(images) != {"arm64", "amd64"}
            or any(not isinstance(value, str) or re.fullmatch(r"sha256:[0-9a-f]{64}", value) is None
                   for value in images.values())):
        raise ValueError("invalid promoted image identities")
    manifests = receipt["registryManifests"]
    if (not isinstance(manifests, list) or len(manifests) != 2
            or any(not isinstance(value, str) or re.fullmatch(
                r"ghcr\.io/[a-z0-9_.-]{1,100}/rustack@sha256:[0-9a-f]{64}", value) is None
                   for value in manifests)):
        raise ValueError("invalid promoted registry manifests")
    actual = execute(["docker", "container", "inspect", "rustack", "--format", "{{.Image}}"])
    if actual != images["arm64"]:
        raise ValueError("Action container differs from the qualified promoted image")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "version", "tag", "promotion-check",
                                            "promoted-image", "package-audit", "action-aliases"))
    args = parser.parse_args()
    try:
        metadata = json.loads(run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))
        selected = (ROOT / "release-packages.txt").read_text().split()
        version = check(metadata, selected)
        if "RELEASE_VERSION" in os.environ and os.environ["RELEASE_VERSION"] != version:
            raise ValueError("promotion version differs from the CLI version")
        if args.command == "version":
            print(version)
        elif args.command == "tag":
            tag(version)
        elif args.command == "action-aliases":
            action_aliases(version, os.environ)
        elif args.command == "promotion-check":
            promotion_identity(version, os.environ)
        elif args.command == "promoted-image":
            _, commit = promotion_identity(version, os.environ)
            path = Path(os.environ["RUSTACK_PROMOTION_RECEIPT"])
            with path.open("rb") as source:
                raw = source.read(65537)
            if len(raw) > 65536:
                raise ValueError("promotion receipt exceeds byte limit")
            promoted_image(version, commit, json.loads(raw), run)
        elif args.command == "package-audit":
            lockfile = Path(metadata["target_directory"]) / "package" / ("rustack-cli-" + version) / "Cargo.lock"
            print(run(["cargo", "audit", "--file", str(lockfile)]))
        else:
            print("Release v" + version + ": " + str(len(selected)) + " packages; reverse dependency closure checked")
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, "release failed: " + str(error) + "\n")


if __name__ == "__main__":
    main()
