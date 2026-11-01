"""Run exact image resources in native arm64 Linux with retained owned evidence."""
import json
import ipaddress
import os
from pathlib import Path
import re
import platform
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))

from image_performance_images import SOURCES, cache_alias, call, fenced, image
from image_performance_support import MeasurementError, decode, require
from image_performance_report import validate
from linux_performance import Runner, artifact, bounded_read, deadline_guard, fingerprint, profile, safe_path, status

CASES = ("test_should_measure_one_hundred_warm_image_invocations",
         "test_should_measure_twenty_preloaded_cold_image_environments",
         "test_should_reuse_one_image_slot_after_one_hundred_log_flood_timeouts")
PHASES = ("warm", "cold", "cycles")


def network(owner, endpoint, deadline, engine):
    value = decode(fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "network", "inspect", "rustack-d102-linux",
        "--format", '{"id":{{json .Id}},"name":{{json .Name}},"driver":{{json .Driver}},"labels":{{json .Labels}},"ipam":{{json .IPAM.Config}}}'], deadline))
    require(isinstance(value, dict) and re.fullmatch(r"[0-9a-f]{64}", value.get("id", "")) is not None
            and value.get("name") == "rustack-d102-linux" and value.get("driver") == "bridge"
            and value.get("labels", {}).get("rustack.workflows.task") == "D102"
            and len(value.get("ipam", [])) == 1, "linux-performance-network")
    gateway = value["ipam"][0]["Gateway"]
    require(isinstance(gateway, str) and len(gateway) <= 15, "linux-performance-gateway")
    address = ipaddress.IPv4Address(gateway)
    require(any(address in ipaddress.IPv4Network(cidr) for cidr in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")),
            "linux-performance-private-gateway")
    return value, gateway


def qualified(raw, case, phase):
    text = raw.decode("utf-8")
    terminal = re.findall(r"^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; "
                          r"finished in [0-9.]+s$", text, re.M)
    require(len(terminal) == 1 and ("test " + case + " ... ") in text, "linux-resource-summary")
    rows = re.findall(r"(?:^|\.\.\. )WORKFLOWS_IMAGE_PERFORMANCE ([^\r\n]*)$", text, re.M)
    require(len(rows) == 1, "linux-resource-report-count")
    value = decode(rows[0].encode())
    require(value.get("phase") == phase and value.get("os") == "linux" and value.get("architecture") == "aarch64"
            and value.get("referenceQualification") == "pending" and value.get("emptyAfterWorkload") is True
            and value.get("sdkVersion") == "1.43.118", "linux-resource-report-identity")
    require(isinstance(value.get("python"), str) and re.fullmatch(r"3\.12\.[0-9]{1,3}", value["python"]) is not None,
            "linux-resource-python")
    require(value.get("debugAssertions") is True, "linux-resource-build-mode")
    validate(value, phase)
    return value


def guest(binary, server, sdk, prepared, owner, deadline):
    require(sys.platform == "linux" and os.uname().machine == "aarch64" and len(os.sched_getaffinity(0)) == 4,
            "linux-image-platform")
    require(sys.version_info[:2] == (3, 12), "linux-image-python-version")
    sources = decode(bounded_read(prepared, 32768))
    endpoint = "unix:///var/run/docker.sock"
    engine = call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip()
    profile(sources["profile"])
    require(sources["engines"]["target"] == engine and len(sources["images"]) == 3, "linux-prepared-engine")
    for name, row in zip(SOURCES, sources["images"]):
        require(row["name"] == name and image(owner, endpoint, row["image"]["id"], deadline, engine) == row["image"], "linux-prepared-image")
    def caches():
        for row in sources["images"]:
            require(row["cacheAlias"] == cache_alias(row["image"]["id"])
                    and image(owner, endpoint, row["cacheAlias"], deadline, engine) == row["image"],
                    "linux-operator-cache-alias")
    caches()
    bridge, gateway = network(owner, endpoint, deadline, engine)
    before = {"test": fingerprint(binary), "server": fingerprint(server)}
    owner.environment.update(RUSTACK_RUNTIME_DOCKER_SOCKET="/var/run/docker.sock", RUSTACK_RUNTIME_ARCH="arm64",
        RUSTACK_RUNTIME_IMAGE=sources["images"][0]["image"]["id"],
        RUSTACK_RUNTIME_HANDLER_IMAGE=sources["images"][1]["image"]["id"],
        RUSTACK_CUSTOM_IMAGE_A=sources["images"][2]["image"]["id"],
        RUSTACK_RUNTIME_NETWORK=bridge["id"], RUSTACK_RUNTIME_GATEWAY_HOST=gateway,
        RUSTACK_WORKFLOWS_PYTHON="/usr/bin/python3", RUSTACK_WORKFLOWS_SDK_SITE_PACKAGES=str(sdk))
    listing = call(owner, [str(binary), "--ignored", "--list", "--format", "terse"], deadline).decode().splitlines()
    names = [line[:-6] for line in listing if line.endswith(": test")]
    from docker_required_suites import parse_manifest
    required = parse_manifest(bounded_read(Path(__file__).with_name("docker-required-suites.json"), 32768))
    require(len(names) == len(set(names)) == len(required["ordinary-startup"]) and set(names) == set(required["ordinary-startup"])
            and set(CASES) <= set(names), "linux-performance-inventory")
    # The complete canonical resource manifest is checked by the required host preflight;
    # the guest selects these same three exact cases from the actual ELF listing.
    root = Path(tempfile.mkdtemp(prefix="rustack-d102-linux-"))
    reports = []
    for case, phase in zip(CASES, PHASES):
        reply = owner.run([str(binary), case, "--ignored", "--exact", "--nocapture", "--test-threads=1"],
                          min(deadline, time.monotonic() + 480), stdout_limit=65536)
        (root / (phase + ".stdout")).write_bytes(reply.stdout)
        (root / (phase + ".stderr")).write_bytes(reply.stderr)
        require(reply.code == 0 and reply.stderr == b"", "linux-image-resource-failed")
        reports.append(qualified(reply.stdout, case, phase))
    caches()
    require(before == {"test": fingerprint(binary), "server": fingerprint(server)}
            and bridge == network(owner, endpoint, deadline, engine)[0] and not owner.unsettled, "linux-image-postcondition")
    return {"schemaVersion": 1, "referenceQualification": "pending", "binarySha256": before,
            "engine": engine, "network": bridge, "evidenceRoot": str(root), "reports": reports,
            "exactResourcesPassed": list(CASES), "processGroupsJoined": True, "python": platform.python_version()}


def host(selected, build, sdk, owner, deadline):
    selected, build, sdk = profile(selected), safe_path(build), safe_path(sdk)
    require(sdk.is_dir(), "sdk-package-root")
    require(not any(os.path.lexists(build.parent / ("image-guest." + suffix))
                    for suffix in ("stdout", "stderr")), "guest-receipt-exists")
    allocation = status(owner, selected, deadline)
    binary, server = artifact(bounded_read(build, 4 * 1024 * 1024), "docker_startup")
    prepared = build.parent / "image-preparation.json"
    before = {"test": fingerprint(binary), "server": fingerprint(server)}
    reply = owner.run(["colima", "-p", selected, "ssh", "--", "python3", "-I", "-B",
                      str(safe_path(str(Path(__file__).resolve()))), "guest", str(binary), str(server), str(sdk), str(prepared)],
                     min(deadline, time.monotonic() + 1600), stdout_limit=65536)
    # Keep raw failures in the explicit task evidence directory before decoding.
    for suffix, raw in (("stdout", reply.stdout), ("stderr", reply.stderr)):
        with (build.parent / ("image-guest." + suffix)).open("xb") as stream:
            stream.write(raw)
    require(reply.code == 0 and reply.stderr == b"", "linux-image-guest-failed")
    result = decode(reply.stdout, maximum=65536)
    require(result.get("binarySha256") == before and result.get("exactResourcesPassed") == list(CASES)
            and result.get("processGroupsJoined") is True and result.get("referenceQualification") == "pending",
            "linux-image-result-affiliation")
    require(before == {"test": fingerprint(binary), "server": fingerprint(server)}
            and allocation == status(owner, selected, deadline), "linux-image-host-postcondition")
    result["profileConfiguration"] = allocation
    return result


def main():
    try:
        require(len(sys.argv) in (5, 6) and sys.argv[1] in ("host", "guest"), "linux-image-arguments")
        owner = Runner(os.environ if sys.argv[1] == "host" else {"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"})
        seconds = 1650 if sys.argv[1] == "host" else 1500
        with deadline_guard(seconds, owner) as deadline:
            if sys.argv[1] == "host":
                require(len(sys.argv) == 5, "linux-image-host-arguments")
                result = host(*sys.argv[2:], owner, deadline)
            else:
                require(len(sys.argv) == 6, "linux-image-guest-arguments")
                result = guest(*(safe_path(value) for value in sys.argv[2:]), owner, deadline)
            raw = json.dumps(result, separators=(",", ":"), allow_nan=False).encode()
            require(len(raw) <= 65536, "linux-image-result-bound")
            print(raw.decode(), flush=True)
        return 0
    except MeasurementError as error:
        print("linux-image-performance-failed:" + str(error), file=sys.stderr)
        return 2
    except Exception:
        print("linux-image-performance-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
