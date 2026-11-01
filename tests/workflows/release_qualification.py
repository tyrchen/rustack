"""Fail-closed, same-source final release evidence assembler (never publishes)."""
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import tarfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
import release_json as storage
from release_json import document
from linux_performance import MIB, QualificationError, Runner, bounded_read, deadline_guard, require
from rootless_functional import source_seal

ROOT = Path(__file__).resolve().parents[2]
LANES = ("rust", "native", "linux-arm64", "rootless-arm64", "desktop", "reference", "release")
HEX = r"[0-9a-f]{64}"
COMMIT = r"[0-9a-f]{40}"
LANE_LOG_BUDGET = 256 * MIB


def digest(value):
    return hashlib.sha256(value).hexdigest()


def member(root, name, maximum, minimum=1):
    require(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9_./-]{1,240}", name)
            and all(part not in ("", ".", "..") for part in name.split("/")), "release-member-path")
    path = root
    for part in name.split("/"):
        path = path / part
        require(not path.is_symlink(), "release-member-symlink")
    metadata = path.stat()
    require(stat.S_ISREG(metadata.st_mode) and minimum <= metadata.st_size <= maximum, "release-member-bound")
    return path


def read(root, name, maximum=MIB, minimum=1):
    return bounded_read(member(root, name, maximum, minimum), maximum)


def archive_hash(path):
    metadata = path.stat()
    require(stat.S_ISREG(metadata.st_mode) and 1 <= metadata.st_size <= 2 * 1024 * MIB, "release-archive-bound")
    size, hashed = 0, hashlib.sha256()
    with path.open("rb") as source:
        while True:
            block = source.read(MIB)
            if not block:
                break
            size += len(block)
            require(size <= metadata.st_size, "release-archive-grew")
            hashed.update(block)
    require(size == metadata.st_size, "release-archive-changed")
    return size, hashed.hexdigest()


def archive_preflight(path):
    size = path.stat().st_size
    require(1024 <= size <= 2 * 1024 * MIB and size % 512 == 0, "release-physical-tar-size")
    position, count, pending = 0, 0, False
    with path.open("rb") as source:
        while position < size:
            header = source.read(512)
            require(len(header) == 512, "release-physical-tar-truncated")
            position += 512
            if not any(header):
                require(not pending and source.read(512) == bytes(512), "release-physical-tar-end")
                while True:
                    block = source.read(65536)
                    if not block:
                        return
                    require(not any(block), "release-physical-tar-trailing")
            count += 1
            require(count <= 2048 and header[156:157] in (b"0", b"\0", b"5", b"x"), "release-physical-tar-type")
            encoded = header[124:136].strip(b" \0")
            checksum = header[148:156].strip(b" \0")
            require(encoded and all(48 <= byte <= 55 for byte in encoded)
                    and checksum and all(48 <= byte <= 55 for byte in checksum), "release-physical-tar-number")
            length = int(encoded, 8)
            require(int(checksum, 8) == sum(header[:148]) + 8 * 32 + sum(header[156:]), "release-physical-tar-checksum")
            padded = ((length + 511) // 512) * 512
            require(length <= 2 * 1024 * MIB and position + padded <= size, "release-physical-tar-body")
            if header[156:157] == b"x":
                from release_image import _timing_pax
                require(not pending and 1 <= length <= 1024, "release-physical-pax-bound")
                _timing_pax(source.read(length))
                source.seek(padded - length, 1)
                pending = True
            else:
                source.seek(padded, 1)
                pending = False
            position += padded
    require(False, "release-physical-tar-missing-end")


def archive_identity(path, architecture, image_id):
    archive_preflight(path)
    configs, manifest, count, total = {}, None, 0, 0
    with tarfile.open(path, mode="r|") as archive:
        for entry in archive:
            count += 1
            total += entry.size
            require(count <= 1024 and total <= 2 * 1024 * MIB and len(entry.name) <= 512
                    and not entry.name.startswith("/")
                    and all(part not in (".", "..") for part in entry.name.rstrip("/").split("/"))
                    and (entry.isfile() or entry.isdir()), "release-archive-member")
            if entry.name == "manifest.json" or (entry.isfile() and entry.size <= MIB
                    and (entry.name.endswith(".json") or re.fullmatch(r"blobs/sha256/" + HEX, entry.name))):
                source = archive.extractfile(entry)
                require(source is not None, "release-archive-config-file")
                raw = source.read(MIB + 1)
                require(len(raw) <= MIB, "release-archive-config-bound")
                if entry.name == "manifest.json":
                    require(manifest is None, "release-archive-duplicate-manifest")
                    manifest = document(raw)
                else:
                    require(entry.name not in configs and len(configs) < 64, "release-archive-config-count")
                    configs[entry.name] = raw
    require(isinstance(manifest, list) and len(manifest) == 1 and isinstance(manifest[0], dict), "release-archive-manifest")
    selected = manifest[0].get("Config")
    require(isinstance(selected, str) and selected in configs, "release-archive-config-reference")
    raw = configs[selected]
    config = document(raw)
    config_digest = "sha256:" + digest(raw)
    require(config.get("architecture") == architecture and config.get("os") == "linux", "release-archive-config-identity")
    current, visited = image_id, set()
    while current != config_digest:
        require(re.fullmatch("sha256:" + HEX, current) is not None and current not in visited and len(visited) < 3,
                "release-archive-identity-graph")
        visited.add(current)
        graph_raw = configs.get("blobs/sha256/" + current[7:])
        require(graph_raw is not None and "sha256:" + digest(graph_raw) == current, "release-archive-addressable-image")
        graph = document(graph_raw)
        if isinstance(graph.get("config"), dict):
            require(graph["config"].get("digest") == config_digest
                    and graph["config"].get("size") == len(raw), "release-archive-graph-config")
            current = config_digest
        else:
            manifests = graph.get("manifests")
            require(isinstance(manifests, list) and len(manifests) == 1 and isinstance(manifests[0], dict),
                    "release-archive-single-platform-index")
            current = manifests[0].get("digest")
    return config_digest


def exact_names(file):
    manifest = document(bounded_read(ROOT / "tests/workflows" / file, 65536))
    return {name for suite in manifest["suites"] for name in suite["tests"]}


def expected_names(lane):
    if lane == "rust":
        return set()
    if lane == "native":
        return exact_names("required-suites.json") | exact_names("action-required-suites.json")
    if lane in ("linux-arm64", "rootless-arm64", "desktop"):
        return exact_names("docker-required-suites.json")
    if lane == "reference":
        from linux_performance import CASES
        return set(CASES) | {"test_should_measure_one_hundred_warm_image_invocations",
            "test_should_measure_twenty_preloaded_cold_image_environments",
            "test_should_reuse_one_image_slot_after_one_hundred_log_flood_timeouts"}
    return {"test_should_execute_image_sdk_and_nested_asl_from_the_release_image",
            "test_should_refuse_unreachable_release_image_gateway_and_join_startup_cleanup"}


def identity(value, commit, seal):
    require(type(value.get("schemaVersion")) is int and value["schemaVersion"] == 1 and value.get("commit") == commit
            and value.get("sourceSealSha256") == seal, "release-source-binding")


def candidate(root, name, commit, seal):
    raw = read(root, name)
    value = document(raw)
    require(set(value) == {"schemaVersion", "commit", "sourceSealSha256", "architecture", "archive",
                          "archiveBytes", "archiveSha256", "imageId", "configDigest"}, "release-candidate-shape")
    identity(value, commit, seal)
    require(value["architecture"] in ("arm64", "amd64")
            and re.fullmatch("sha256:" + HEX, value["imageId"]) is not None, "release-candidate-identity")
    path = member(root, value["archive"], 2 * 1024 * MIB)
    size, hashed = archive_hash(path)
    require(type(value["archiveBytes"]) is int and (size, hashed) ==
            (value["archiveBytes"], value["archiveSha256"]), "release-candidate-archive")
    require(archive_identity(path, value["architecture"], value["imageId"]) == value["configDigest"], "release-candidate-config-digest")
    if value["architecture"] == "arm64":
        from release_image import ARCHIVE_LIMIT, verify
        verify(bounded_read(path, ARCHIVE_LIMIT), "arm64", value["configDigest"])
    return value, digest(raw)


def bound_document(root, name, receipts):
    require(isinstance(name, str) and name in receipts, "release-unbound-producer-report")
    return document(read(root, name))


def release_resource(value, image_id):
    require(isinstance(value, dict) and set(value) == {"schemaVersion", "profile", "mode", "architecture",
            "server", "shutdown", "cleanup"} and type(value["schemaVersion"]) is int and value["schemaVersion"] == 1 and value["profile"] == "release"
            and value["architecture"] == "arm64" and value["mode"] in ("sdk-nested", "gateway-rejection"), "release-consumer-shape")
    server, shutdown, cleanup = value["server"], value["shutdown"], value["cleanup"]
    require(isinstance(server, dict) and isinstance(shutdown, dict) and isinstance(cleanup, dict)
            and server.get("source") == image_id and isinstance(server.get("id"), str)
            and re.fullmatch(HEX, server["id"]) is not None
            and shutdown.get("id") == server["id"] and shutdown.get("engine") == cleanup.get("engine") == server.get("engine")
            and isinstance(server.get("engine"), str) and re.fullmatch(r"[0-9a-f-]{36}", server["engine"])
            and shutdown.get("physicallyStopped") is True and type(shutdown.get("exitCode")) is int
            and (shutdown["exitCode"] == 0 if value["mode"] == "sdk-nested" else 1 <= shutdown["exitCode"] <= 255)
            and cleanup.get("schemaVersion") == 1 and cleanup.get("serverAbsent") is True
            and cleanup.get("volumeAbsent") is True and cleanup.get("uncertain") is False,
            "release-consumer-candidate-or-cleanup")
    sources = cleanup.get("sourcesPreserved")
    require(isinstance(sources, list) and len(sources) == 3 and all(isinstance(row, list) and len(row) == 2 for row in sources)
            and sources[0][1] == image_id, "release-consumer-source-preservation")


def execution_resources(root, qualification, receipts, seal, engine, endpoint):
    from rootless_functional import SUITES
    from rootless_resources import verify_output
    from docker_required_suites import parse_manifest
    required_raw = bounded_read(ROOT / "tests/workflows/docker-required-suites.json", 65536)
    required = parse_manifest(required_raw)
    count = sum(len(names) for names in required.values())
    report = bound_document(root, qualification["executionReport"], receipts)
    entry = bound_document(root, qualification["executionEntry"], receipts)
    require(report.get("schemaVersion") == 1 and report.get("engine") == entry.get("engine") == engine
            and report.get("scope") == "complete-functional-inventory" and report.get("failed") == []
            and report.get("attempted") == report.get("passed") == count
            and report.get("allChildGroupsJoined") is True and entry.get("sourceSealSha256") == seal
            and entry.get("manifestSha256") == digest(required_raw)
            and isinstance(entry.get("environment"), dict)
            and "unix://" + entry["environment"].get("RUSTACK_RUNTIME_DOCKER_SOCKET", "") == endpoint,
            "release-platform-execution-affiliation")
    rows, cases = report.get("rows"), qualification["caseReceipts"]
    require(isinstance(rows, list) and isinstance(cases, list) and len(rows) == len(cases) == count,
            "release-platform-case-count")
    seen = set()
    for row, case in zip(rows, cases):
        require(isinstance(case, dict) and set(case) == {"suite", "name", "intent", "result", "stdout", "stderr"}
                and isinstance(row, dict) and row.get("suite") == case["suite"] and row.get("name") == case["name"]
                and type(row.get("code")) is int and row["code"] == 0 and case["suite"] in required and case["name"] in required[case["suite"]]
                and case["name"] not in seen, "release-platform-exact-case")
        seen.add(case["name"])
        for role, basename in (("intent", "intent.json"), ("result", "result.json"), ("stdout", "stdout"), ("stderr", "stderr")):
            require(case[role] in receipts and Path(case[role]).name == basename
                    and Path(case[role]).parent.name == Path(row.get("evidence", "")).name,
                    "release-platform-case-binding")
        intent = bound_document(root, case["intent"], receipts)
        result = bound_document(root, case["result"], receipts)
        require(type(result.get("code")) is int and result["code"] == 0 and result.get("stdoutSha256") == receipts[case["stdout"]]
                and result.get("stderrSha256") == receipts[case["stderr"]], "release-platform-case-result")
        expected = ["cargo", "test", "--locked", "--target-dir", entry.get("targetDirectory")]
        expected += SUITES[case["suite"]] + ["--", "--ignored", "--exact", case["name"], "--show-output", "--test-threads=1"]
        require(intent.get("argv") == expected, "release-platform-case-argv")
        verify_output(read(root, case["stdout"], 16 * MIB), {case["name"]})


def command_producer(root, details, hashed, seal, commands):
    require(set(details) == {"gates", "report", "entry", "intents"}
            and details["gates"] == [name for name, _ in commands], "release-rust-gates")
    require(isinstance(details["intents"], list) and len(details["intents"]) == len(commands), "release-rust-intents")
    report = bound_document(root, details["report"], hashed)
    entry = bound_document(root, details["entry"], hashed)
    require(report.get("schemaVersion") == entry.get("schemaVersion") == 1
            and report.get("sourceSealSha256") == entry.get("sourceSealSha256") == seal
            and report.get("allChildGroupsJoined") is True
            and entry.get("gates") == [{"name":name, "argv":argv} for name, argv in commands]
            and isinstance(report.get("gates"), list) and len(report["gates"]) == len(commands), "release-rust-producer")
    for row, (gate, argv), intent_path in zip(report["gates"], commands, details["intents"]):
        require(isinstance(row, dict) and row.get("name") == gate and row.get("argv") == argv
                and type(row.get("code")) is int and row["code"] == 0, "release-rust-command")
        logs = [name for name in hashed if Path(name).name == gate + ".log"]
        require(len(logs) == 1 and hashed[logs[0]] == row.get("logSha256"), "release-rust-log-binding")
        intent = bound_document(root, intent_path, hashed)
        invocation = intent.get("argv")
        require(isinstance(invocation, list) and len(invocation) == len(argv) + 5
                and invocation[1:3] == ["-I", "-B"] and Path(invocation[3]).name == "gate_command.py"
                and Path(invocation[4]).name == gate + ".log" and invocation[5:] == argv, "release-rust-intent-argv")
        if gate != "fmt":
            require(len(read(root, logs[0], 16 * MIB)) > 0, "release-empty-rust-log")
        if gate in ("test", "feature-test", "doctest", "feature-doctest"):
            log = read(root, logs[0], 16 * MIB).decode()
            summaries = re.findall(r"^test result: ok\. [0-9]+ passed; ([0-9]+) failed;", log, re.MULTILINE)
            require(summaries and all(value == "0" for value in summaries), "release-rust-test-summary")


def lane(root, lane_path, label, commit, seal, candidate_ids):
    raw = read(root, lane_path)
    value = document(raw)
    require(set(value) == {"schemaVersion", "lane", "commit", "sourceSealSha256", "status",
            "allChildGroupsJoined", "tests", "receipts", "qualification"}, "release-lane-shape")
    identity(value, commit, seal)
    require(value["lane"] == label and value["status"] == "passed"
            and value["allChildGroupsJoined"] is True, "release-lane-not-passed")
    tests = value["tests"]
    expected = expected_names(label)
    require(isinstance(tests, list) and all(isinstance(row, str) for row in tests)
            and len(tests) == len(set(tests)) and set(tests) == expected, "release-exact-inventory")
    receipts = value["receipts"]
    require(isinstance(receipts, list) and 1 <= len(receipts) <= 512, "release-receipt-count")
    limits = {name:1 for name in expected}
    if label == "native":
        from native_gates import DOUBLED
        require(set(DOUBLED) <= expected and len(set(DOUBLED)) == 3, "release-native-catalog-drift")
        limits.update({name:2 for name in DOUBLED})
    seen, observed, hashed = set(), Counter(), {}
    total = 0
    release_resources = []
    for row in receipts:
        require(isinstance(row, dict) and set(row) == {"path", "sha256", "code", "groupJoined"}
                and type(row["code"]) is int and row["code"] == 0 and row["groupJoined"] is True
                and row["path"] not in seen, "release-receipt-shape")
        seen.add(row["path"])
        path = member(root, row["path"], 16 * MIB, minimum=0)
        total += path.stat().st_size
        require(total <= LANE_LOG_BUDGET, "release-lane-log-budget")
        content = bounded_read(path, 16 * MIB)
        require(digest(content) == row["sha256"], "release-receipt-hash")
        hashed[row["path"]] = row["sha256"]
        text = content.decode("utf-8")
        if label == "release":
            for record in re.findall(r"(?:^|\.\.\. )WORKFLOWS_RELEASE_RESOURCE ([^\r\n]*)$", text, re.MULTILINE):
                require(len(record) <= 16384 and len(release_resources) < 2, "release-consumer-receipt-bound")
                release_resources.append(document(record.encode()))
        require(not re.search(r"^test result: FAILED|^test .* \.\.\. FAILED$", text, re.MULTILINE), "release-failed-test")
        if label in ("linux-arm64", "rootless-arm64", "desktop", "reference"):
            continue  # These inventories come from bound typed producers below.
        pattern = r"^test ([A-Za-z0-9_:]+) \.\.\. (?!ignored\b)" if label == "native" else r"^test ([A-Za-z0-9_:]+) \.\.\. ok$"
        for match in re.finditer(pattern, text, re.MULTILINE):
            test_name = match.group(1)
            require(len(test_name) <= 512, "release-test-name-bound")
            if test_name in expected:
                observed[test_name] += 1
                require(observed[test_name] <= limits[test_name], "release-repeated-resource")
    if label not in ("linux-arm64", "rootless-arm64", "desktop", "reference"):
        require(all(observed[test] == limits[test] for test in expected), "release-missing-or-repeated-test")
    qualification = value["qualification"]
    require(isinstance(qualification, dict), "release-qualification-shape")
    if label == "rust":
        from rust_gates import GATES
        command_producer(root, qualification, hashed, seal, GATES)
    elif label == "reference":
        require(set(qualification) == {"referenceQualification", "os", "architecture", "physicalCpus", "exclusive",
            "pureP95Micros", "warmOverheadP95Micros", "coldEndToEndP95Micros", "loadCycles", "pureReport", "imageReport"}
            and qualification["referenceQualification"] == "qualified" and qualification["os"] == "linux"
            and qualification["architecture"] == "arm64" and qualification["exclusive"] is True
            and type(qualification["physicalCpus"]) is int and qualification["physicalCpus"] >= 4,
            "release-reference-attestation")
        for key, cap in (("pureP95Micros", 100000), ("warmOverheadP95Micros", 50000), ("coldEndToEndP95Micros", 5000000)):
            require(type(qualification[key]) is int and 0 < qualification[key] <= cap, "release-reference-budget")
        require(type(qualification["loadCycles"]) is int and qualification["loadCycles"] == 100, "release-load-count")
        from linux_performance import CASES, validate_report
        from image_performance_report import validate as validate_image
        from linux_image_performance import CASES as IMAGE_CASES
        pure = bound_document(root, qualification["pureReport"], hashed)
        images = bound_document(root, qualification["imageReport"], hashed)
        require(pure.get("processGroupsJoined") is True and pure.get("exactResourcesPassed") == list(CASES)
                and isinstance(pure.get("reports"), list) and len(pure["reports"]) == 2, "release-reference-pure-producer")
        for report, kind in zip(pure["reports"], ("pureLatency", "loopResponsiveness")):
            validate_report(report, kind)
        require(images.get("processGroupsJoined") is True and images.get("exactResourcesPassed") == list(IMAGE_CASES)
                and len(set(CASES) | set(IMAGE_CASES)) == 5 and set(CASES) | set(IMAGE_CASES) == expected
                and isinstance(images.get("reports"), list)
                and len(images["reports"]) == 3, "release-reference-image-producer")
        for report, phase in zip(images["reports"], ("warm", "cold", "cycles")):
            require(report.get("os") == "linux" and report.get("architecture") in ("arm64", "aarch64"), "release-reference-image-platform")
            validate_image(report, phase)
        require(qualification["pureP95Micros"] == pure["reports"][0]["measurement"]["latency"]["p95Micros"]
                and qualification["warmOverheadP95Micros"] == images["reports"][0]["measurement"]["overhead"]["p95Micros"]
                and qualification["coldEndToEndP95Micros"] == images["reports"][1]["measurement"]["latency"]["p95Micros"]
                and qualification["loadCycles"] == images["reports"][2]["measurement"]["completedCycles"], "release-reference-summary-differs")
    elif label in ("linux-arm64", "rootless-arm64", "desktop"):
        require(set(qualification) == {"os", "architecture", "engineId", "rootless", "desktop", "zeroOwnedRemnants", "platformReport", "executionReport", "executionEntry", "caseReceipts"}
            and qualification["os"] == "linux" and qualification["architecture"] == "arm64"
            and qualification["rootless"] is (label == "rootless-arm64")
            and qualification["desktop"] is (label == "desktop") and qualification["zeroOwnedRemnants"] is True
            and isinstance(qualification["engineId"], str) and 1 <= len(qualification["engineId"]) <= 128,
            "release-platform-kind")
        actual = bound_document(root, qualification["platformReport"], hashed)
        require(actual.get("schemaVersion") == 1 and actual.get("sourceSealSha256") == seal
                and actual.get("containersAbsent") is actual.get("volumesAbsent") is actual.get("privateAliasesAbsent") is True
                and actual.get("allChildGroupsJoined") is True and isinstance(actual.get("daemon"), dict), "release-platform-producer")
        daemon = actual["daemon"]
        require(daemon.get("os") == "linux" and daemon.get("architecture") in ("arm64", "aarch64")
                and daemon.get("engineId") == qualification["engineId"]
                and isinstance(daemon.get("securityOptions"), list)
                and ("name=rootless" in daemon["securityOptions"]) is qualification["rootless"]
                and isinstance(daemon.get("operatingSystem"), str)
                and daemon["operatingSystem"].startswith("Docker Desktop") is qualification["desktop"], "release-platform-attribution")
        execution_resources(root, qualification, hashed, seal, daemon["engineId"], actual.get("endpoint"))
    elif label == "release":
        require(qualification == {"imageId": candidate_ids["arm64"], "zeroOwnedRemnants": True}, "release-tested-candidate")
        require(len(release_resources) == 2 and {row.get("mode") for row in release_resources} ==
                {"sdk-nested", "gateway-rejection"}, "release-consumer-modes")
        for row in release_resources:
            release_resource(row, candidate_ids["arm64"])
    else:
        from native_gates import GATES, HTTP
        require(set(qualification) == {"nativeChildrenJoined", "gates", "report", "entry", "intents"}
                and qualification["nativeChildrenJoined"] is True, "release-native-cleanup")
        details = {key:value for key, value in qualification.items() if key != "nativeChildrenJoined"}
        command_producer(root, details, hashed, seal, GATES)
        for gate, _ in GATES:
            paths = [name for name in hashed if Path(name).name == gate + ".log"]
            require(len(paths) == 1, "release-native-gate-log")
            text = read(root, paths[0], 16 * MIB).decode()
            summaries = re.findall(r"^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored;", text, re.MULTILINE)
            require(summaries and all(row[0] == "ok" and row[2] == "0" for row in summaries), "release-native-terminal-summary")
            if gate == "action":
                require(summaries == [("ok", "1", "0", "0")], "release-action-terminal-summary")
            else:
                match = re.search(r"^test " + re.escape(HTTP) + r" \.\.\. (?!ignored)", text, re.MULTILINE)
                require(match is not None, "release-native-http-start")
                terminal = re.search(r"^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored;",
                                     text[match.end():], re.MULTILINE)
                require(terminal is not None and terminal.groups() == ("ok", "1", "0", "0"), "release-native-http-terminal")
    return {"path": lane_path, "sha256": digest(raw), "receipts": hashed}


def assemble(root, owner, deadline):
    bundle = document(read(root, "bundle.json"))
    require(set(bundle) == {"schemaVersion", "commit", "sourceSealSha256", "candidateRunId", "candidates", "lanes"}
            and type(bundle["schemaVersion"]) is int and bundle["schemaVersion"] == 1 and isinstance(bundle["candidateRunId"], str)
            and re.fullmatch(r"[1-9][0-9]{0,19}|local", bundle["candidateRunId"]) is not None and re.fullmatch(COMMIT, bundle["commit"]) is not None
            and re.fullmatch(HEX, bundle["sourceSealSha256"]) is not None, "release-bundle-shape")
    commit = owner.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], deadline)
    require(commit.code == 0 and commit.stderr == b"" and commit.stdout.decode().strip() == bundle["commit"], "release-commit")
    source_before = source_seal(owner, deadline)
    seal = digest(json.dumps(source_before, sort_keys=True, separators=(",", ":")).encode())
    require(seal == bundle["sourceSealSha256"], "release-current-source")
    require(isinstance(bundle["candidates"], list) and len(bundle["candidates"]) == 2
            and isinstance(bundle["lanes"], dict) and set(bundle["lanes"]) == set(LANES), "release-missing-lane")
    candidates, candidate_hashes = {}, {}
    for name in bundle["candidates"]:
        value, hashed = candidate(root, name, bundle["commit"], seal)
        require(value["architecture"] not in candidates, "release-duplicate-candidate")
        candidates[value["architecture"]] = value
        candidate_hashes[name] = hashed
    require(set(candidates) == {"arm64", "amd64"}, "release-candidate-architectures")
    ids = {key: value["imageId"] for key, value in candidates.items()}
    lanes = {label: lane(root, bundle["lanes"][label], label, bundle["commit"], seal, ids) for label in LANES}
    require(source_seal(owner, deadline) == source_before, "release-source-changed-during-validation")
    return {"schemaVersion": 1, "status": "passed", "commit": bundle["commit"], "sourceSealSha256": seal,
            "candidateRunId": bundle["candidateRunId"], "bundleSha256": digest(read(root, "bundle.json")), "candidates": candidates,
            "candidateSeals": candidate_hashes, "lanes": lanes, "waivers": ["D100-linux-x86_64-execution"],
            "allChildGroupsJoined": not owner.unsettled}


def main():
    try:
        require(len(sys.argv) == 3, "release-arguments")
        root, output = Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2])
        owner = Runner(os.environ)
        with deadline_guard(300, owner) as deadline:
            report = assemble(root, owner, deadline)
            storage.publish(output, report)
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError, RecursionError, tarfile.TarError):
        print("release-qualification-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
