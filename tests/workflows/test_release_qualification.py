"""Literal delivery controls; no daemon, network, build or publication."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch

import release_delivery as delivery
import release_json
import release_qualification as subject
from rust_gates import GATES
from settings import ActionError

COMMIT, SEAL = "a" * 40, "b" * 64
IMAGE = "sha256:" + "c" * 64
OTHER = "sha256:" + "d" * 64
ENGINE = "7bbe02f2-0571-42f9-a4a3-a36a81425357"


def store(root, name, value):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    raw = value if isinstance(value, bytes) else json.dumps(value, separators=(",", ":")).encode()
    path.write_bytes(raw)
    return {"path":name, "sha256":subject.digest(raw), "code":0, "groupJoined":True}


def envelope(label):
    return {"schemaVersion":1, "lane":label, "commit":COMMIT, "sourceSealSha256":SEAL,
            "status":"passed", "allChildGroupsJoined":True, "tests":sorted(subject.expected_names(label)),
            "receipts":[], "qualification":{}}


def resource(mode="sdk-nested"):
    return {"schemaVersion":1, "profile":"release", "mode":mode, "architecture":"arm64",
        "server":{"id":"e" * 64, "engine":ENGINE, "source":IMAGE, "nonce":"f" * 32, "volume":"owned"},
        "shutdown":{"id":"e" * 64, "engine":ENGINE, "physicallyStopped":True,
                    "exitCode":0 if mode == "sdk-nested" else 1},
        "cleanup":{"schemaVersion":1, "engine":ENGINE, "serverAbsent":True, "volumeAbsent":True,
            "uncertain":False, "sourcesPreserved":[["release", IMAGE], ["sdk", OTHER], ["relay", OTHER]]}}


def release_lane(root):
    value = envelope("release")
    modes = {"test_should_execute_image_sdk_and_nested_asl_from_the_release_image":"sdk-nested",
             "test_should_refuse_unreachable_release_image_gateway_and_join_startup_cleanup":"gateway-rejection"}
    for index, name in enumerate(value["tests"]):
        raw = ("test " + name + " ... ok\nWORKFLOWS_RELEASE_RESOURCE " + json.dumps(resource(modes[name]))
               + "\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n").encode()
        value["receipts"].append(store(root, "release/case%d.log" % index, raw))
    value["qualification"] = {"imageId":IMAGE, "zeroOwnedRemnants":True}
    store(root, "release.json", value)
    return value


def rust_lane(root):
    value = envelope("rust")
    rows, intents = [], []
    for name, argv in GATES:
        log = b"" if name == "fmt" else b"Finished verification\ntest unrelated::resource ... ok\n"
        if name in ("test", "feature-test", "doctest", "feature-doctest"):
            log += b"test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s\n"
        receipt = store(root, "rust/" + name + ".log", log)
        value["receipts"].append(receipt)
        rows.append({"name":name, "argv":argv, "code":0, "seconds":1, "logSha256":receipt["sha256"]})
        intent = "rust/" + name + ".intent.json"
        intents.append(intent)
        value["receipts"].append(store(root, intent, {"argv":["/python", "-I", "-B", "/gate_command.py",
                                                           "/evidence/" + name + ".log"] + argv, "cutoffMonotonic":100}))
    value["receipts"].append(store(root, "rust/report.json", {"schemaVersion":1, "sourceSealSha256":SEAL,
        "gates":rows, "allChildGroupsJoined":True}))
    value["receipts"].append(store(root, "rust/entry.json", {"schemaVersion":1, "sourceSealSha256":SEAL,
        "ownerPid":100, "gates":[{"name":name, "argv":argv} for name, argv in GATES]}))
    value["qualification"] = {"gates":[name for name, _ in GATES], "report":"rust/report.json",
                              "entry":"rust/entry.json", "intents":intents}
    store(root, "rust.json", value)
    return value


def reference_lane(root):
    from linux_performance import CASES
    from linux_image_performance import CASES as IMAGE_CASES
    from test_linux_performance import report as pure
    from test_image_performance import warm, identities, logs
    from image_performance_support import cold_stages, samples
    warm_report = warm()
    cold_report = copy.deepcopy(warm_report)
    cold_report["phase"] = "cold"
    names = identities()
    cold_report["measurement"] = {"latency":samples([301] * 20, 20), "environments":names,
        "uniqueContainers":40, "initialization":cold_stages(logs(names), names, [301] * 20)}
    cycle_report = copy.deepcopy(warm_report)
    cycle_report["phase"] = "cycles"
    cycle_report["measurement"] = {"readiness":samples([10] * 100, 100), "timeout":samples([1000000] * 100, 100),
        "floods":[{"requestId":"00000000-0000-0000-0000-%012x" % (index + 1), "beforeBytes":1024, "afterBytes":2048}
                  for index in range(100)], "processStats":[{"rssKiB":10000,"tasks":8,"runtimeTasks":4}] * 100,
        "completedCycles":100, "containerHighWater":2, "emptyAfterEachTimeout":True, "runtimeTaskBaseline":4,
        "taskQuiescence":samples([100] * 100, 100), "liveBrackets":[[1,1,False]] * 100,
        "primingInvocations":1, "primingCleanupConfirmed":True,
        "controlChannels":{"prepared":100,"readyBeforeInvoke":100,"sameConnection":100,"closed":100}}
    value = envelope("reference")
    value["receipts"] = [
        store(root, "reference/pure.json", {"processGroupsJoined":True,
            "exactResourcesPassed":list(CASES),
            "reports":[pure(), pure("loopResponsiveness")]}),
        store(root, "reference/images.json", {"processGroupsJoined":True,"exactResourcesPassed":list(IMAGE_CASES),"reports":[warm_report,cold_report,cycle_report]})]
    value["qualification"] = {"referenceQualification":"qualified", "os":"linux", "architecture":"arm64",
        "physicalCpus":4, "exclusive":True, "pureP95Micros":95, "warmOverheadP95Micros":5000,
        "coldEndToEndP95Micros":301, "loadCycles":100, "pureReport":"reference/pure.json", "imageReport":"reference/images.json"}
    store(root, "reference.json", value)
    return value


def platform_lane(root, execution_engine=ENGINE):
    from rootless_functional import SUITES
    from docker_required_suites import parse_manifest
    value = envelope("rootless-arm64")
    manifest_raw = (subject.ROOT / "tests/workflows/docker-required-suites.json").read_bytes()
    required = parse_manifest(manifest_raw)
    rows, cases = [], []
    for suite in SUITES:
        for name in sorted(required[suite]):
            directory = "platform/case-%03d" % (len(rows) + 1)
            output = ("test " + name + " ... inherited SDK output\nok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n").encode()
            stdout = store(root, directory + "/stdout", output)
            stderr = store(root, directory + "/stderr", b"")
            arguments = ["cargo", "test", "--locked", "--target-dir", "/tmp/target"] + SUITES[suite]
            arguments += ["--", "--ignored", "--exact", name, "--show-output", "--test-threads=1"]
            intent = store(root, directory + "/intent.json", {"argv":arguments})
            result = store(root, directory + "/result.json", {"code":0,"stdoutSha256":stdout["sha256"],"stderrSha256":stderr["sha256"]})
            value["receipts"].extend([stdout,stderr,intent,result])
            rows.append({"suite":suite,"name":name,"code":0,"evidence":"/original/" + Path(directory).name})
            cases.append({"suite":suite,"name":name,"stdout":stdout["path"],"stderr":stderr["path"],
                          "intent":intent["path"],"result":result["path"]})
    value["receipts"].append(store(root, "platform/execution.json", {"schemaVersion":1,"engine":execution_engine,
        "scope":"complete-functional-inventory","attempted":len(rows),"passed":len(rows),"failed":[],
        "allChildGroupsJoined":True,"rows":rows}))
    value["receipts"].append(store(root, "platform/entry.json", {"engine":execution_engine,"sourceSealSha256":SEAL,
        "manifestSha256":subject.digest(manifest_raw), "environment":{"RUSTACK_RUNTIME_DOCKER_SOCKET":"/tmp/rootless.sock"},
        "targetDirectory":"/tmp/target"}))
    value["receipts"].append(store(root, "platform/platform.json", {"schemaVersion":1,"sourceSealSha256":SEAL,
        "containersAbsent":True,"volumesAbsent":True,"privateAliasesAbsent":True,"allChildGroupsJoined":True,
        "endpoint":"unix:///tmp/rootless.sock", "daemon":{"engineId":ENGINE,"os":"linux","architecture":"aarch64",
        "operatingSystem":"Ubuntu","securityOptions":["name=rootless"]}}))
    value["qualification"] = {"os":"linux","architecture":"arm64","engineId":ENGINE,"rootless":True,"desktop":False,
        "zeroOwnedRemnants":True,"platformReport":"platform/platform.json","executionReport":"platform/execution.json",
        "executionEntry":"platform/entry.json","caseReceipts":cases}
    store(root, "platform.json", value)
    return value


def native_lane(root, adjustment=0, http_passes=1):
    from native_gates import GATES as commands, DOUBLED, HTTP
    value = envelope("native")
    rows, intents = [], []
    resource_names = subject.exact_names("required-suites.json")
    lines = []
    for name in sorted(resource_names - {HTTP}):
        count = 2 if name in DOUBLED else 1
        if name == DOUBLED[0]: count += adjustment
        for _ in range(count):
            lines.append("test " + name + " ... child output\nok")
    lines += ["test " + HTTP + " ... inherited uv output\nok", "test result: ok. %d passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s" % http_passes]
    bodies = {"native":("\n".join(lines) + "\n").encode(),
        "action":b"test test_should_execute_actual_action_process_controls ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n"}
    for name, argv in commands:
        receipt = store(root, "recorded/" + name + ".log", bodies[name])
        value["receipts"].append(receipt)
        rows.append({"name":name,"argv":argv,"code":0,"seconds":1,"logSha256":receipt["sha256"]})
        intent = "recorded/" + name + ".intent.json"
        intents.append(intent)
        value["receipts"].append(store(root, intent, {"argv":["/python","-I","-B","/gate_command.py","/recorded/" + name + ".log"] + argv}))
    value["receipts"].append(store(root, "recorded/report.json", {"schemaVersion":1,"sourceSealSha256":SEAL,"gates":rows,"allChildGroupsJoined":True}))
    value["receipts"].append(store(root, "recorded/entry.json", {"schemaVersion":1,"sourceSealSha256":SEAL,"gates":[{"name":name,"argv":argv} for name,argv in commands]}))
    value["qualification"] = {"nativeChildrenJoined":True,"gates":[name for name,_ in commands],"report":"recorded/report.json","entry":"recorded/entry.json","intents":intents}
    store(root, "native.json", value)
    return value


class ReleaseQualificationControls(unittest.TestCase):
    def test_should_preserve_the_requested_envelope_path_and_hash_for_every_lane_branch(self):
        factories = (("rust", "rust.json", rust_lane), ("native", "native.json", native_lane),
                     ("reference", "reference.json", reference_lane),
                     ("rootless-arm64", "platform.json", platform_lane),
                     ("release", "release.json", release_lane))
        for label, lane_path, factory in factories:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                factory(root)
                checked = subject.lane(root, lane_path, label, COMMIT, SEAL, {"arm64":IMAGE})
                self.assertEqual(checked["path"], lane_path)
                self.assertEqual(checked["sha256"], subject.digest((root / lane_path).read_bytes()))

    def test_should_bind_both_actual_release_modes_to_the_candidate_and_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release_lane(root)
            subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":IMAGE})
            with self.assertRaises(ValueError):
                subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":OTHER})
            for field, changed in (("source", OTHER), ("id", "short")):
                row = resource()
                row["server"][field] = changed
                with self.subTest(field=field), self.assertRaises(ValueError): subject.release_resource(row, IMAGE)
            row = resource()
            row["cleanup"]["uncertain"] = True
            with self.assertRaises(ValueError): subject.release_resource(row, IMAGE)

    def test_should_reject_A_consumer_evidence_even_when_envelope_claims_candidate_B(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = release_lane(root)
            value["qualification"]["imageId"] = OTHER
            store(root, "release.json", value)
            with self.assertRaises(ValueError): subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":OTHER})

    def test_should_require_rust_producer_intents_and_preserve_empty_fmt_log(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = rust_lane(root)
            subject.lane(root, "rust.json", "rust", COMMIT, SEAL, {})
            for mutation in ("missing-report", "unjoined", "wrong-source", "empty-log", "failed-command"):
                changed = copy.deepcopy(value)
                if mutation == "missing-report": changed["qualification"]["report"] = "missing.json"
                elif mutation == "unjoined": changed["allChildGroupsJoined"] = False
                elif mutation == "wrong-source": changed["sourceSealSha256"] = "0" * 64
                elif mutation == "empty-log":
                    changed["receipts"][0] = store(root, "rust/build.log", b"")
                else: changed["receipts"][0]["code"] = 101
                store(root, "rust.json", changed)
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    subject.lane(root, "rust.json", "rust", COMMIT, SEAL, {})
                value = rust_lane(root)

    def test_should_reject_missing_duplicate_or_changed_raw_resource_results(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = release_lane(root)
            for mutation in ("missing", "duplicate", "failed", "hash"):
                changed = copy.deepcopy(value)
                if mutation == "missing": changed["receipts"].pop()
                elif mutation == "duplicate": changed["receipts"].append(changed["receipts"][0])
                elif mutation == "failed": changed["receipts"][0]["code"] = 101
                else: changed["receipts"][0]["sha256"] = "0" * 64
                store(root, "release.json", changed)
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":IMAGE})

    def test_should_reject_arbitrary_rust_log_names_without_producer_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = envelope("rust")
            value["receipts"] = [store(root, name + ".log", b"claimed success") for name, _ in GATES]
            value["qualification"] = {"gates":[name for name, _ in GATES]}
            store(root, "rust.json", value)
            with self.assertRaises(ValueError): subject.lane(root, "rust.json", "rust", COMMIT, SEAL, {})

    def test_should_validate_recorded_native_interleaving_and_exact_existing_stage_multiplicities(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native_lane(root)
            subject.lane(root, "native.json", "native", COMMIT, SEAL, {})
            for adjustment in (-1, 1):
                native_lane(root, adjustment=adjustment)
                with self.subTest(adjustment=adjustment), self.assertRaises(ValueError):
                    subject.lane(root, "native.json", "native", COMMIT, SEAL, {})
            native_lane(root, http_passes=2)
            with self.assertRaisesRegex(ValueError, "http-terminal"):
                subject.lane(root, "native.json", "native", COMMIT, SEAL, {})

    def test_should_bind_every_platform_case_to_the_same_executing_engine(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            platform_lane(root)
            subject.lane(root, "platform.json", "rootless-arm64", COMMIT, SEAL, {})
            platform_lane(root, "56b0fabd-452d-46a5-ac75-5b4063ab4061")
            with self.assertRaisesRegex(ValueError, "execution-affiliation"):
                subject.lane(root, "platform.json", "rootless-arm64", COMMIT, SEAL, {})

    def test_should_reject_symlinks_absolute_paths_and_parent_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            store(root, "real", b"receipt")
            (root / "link").symlink_to(root / "real")
            for name in ("../real", "/real", "x//real", "link"):
                with self.subTest(name=name), self.assertRaises(ValueError): subject.member(root, name, 1024)

    def test_should_preflight_large_extensions_before_tarfile_can_allocate(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "large.tar"
            for kind in (tarfile.XHDTYPE, tarfile.GNUTYPE_LONGNAME):
                header = tarfile.TarInfo("extension")
                header.type, header.size = kind, 2 * 1024 * 1024
                with path.open("wb") as output:
                    output.write(header.tobuf())
                    output.truncate(3 * 1024 * 1024)
                with patch.object(tarfile, "open", side_effect=AssertionError("eager parser entered")), self.assertRaises(ValueError):
                    subject.archive_identity(path, "arm64", IMAGE)

    def test_should_match_archive_config_bytes_architecture_and_manifest_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "image.tar"
            config = b'{"os":"linux","architecture":"arm64"}'
            identity = "sha256:" + subject.digest(config)
            with tarfile.open(path, "w") as archive:
                for name, raw in (("config.json", config), ("manifest.json", b'[{"Config":"config.json","Layers":[]}]')):
                    entry = tarfile.TarInfo(name); entry.size = len(raw)
                    archive.addfile(entry, io.BytesIO(raw))
            subject.archive_identity(path, "arm64", identity)
            with self.assertRaises(ValueError): subject.archive_identity(path, "arm64", OTHER)
            with self.assertRaises(ValueError): subject.archive_identity(path, "amd64", identity)

    def test_should_recompute_reference_metrics_from_raw_producer_samples(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = reference_lane(root)
            subject.lane(root, "reference.json", "reference", COMMIT, SEAL, {})
            value["qualification"]["warmOverheadP95Micros"] = 1
            store(root, "reference.json", value)
            with self.assertRaisesRegex(ValueError, "summary-differs"):
                subject.lane(root, "reference.json", "reference", COMMIT, SEAL, {})

    def test_should_reject_missing_duplicate_or_reordered_reference_producer_names(self):
        for name in ("pure", "images"):
            for mutation in ("missing", "duplicate", "reordered"):
                with self.subTest(name=name, mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    value = reference_lane(root)
                    path = "reference/" + name + ".json"
                    report = json.loads((root / path).read_bytes())
                    original = report["exactResourcesPassed"]
                    if mutation == "missing":
                        report.pop("exactResourcesPassed")
                    elif mutation == "duplicate":
                        report["exactResourcesPassed"] = [original[0]] * len(original)
                    else:
                        report["exactResourcesPassed"] = list(reversed(original))
                    receipt = store(root, path, report)
                    value["receipts"] = [receipt if row["path"] == path else row for row in value["receipts"]]
                    store(root, "reference.json", value)
                    with self.assertRaisesRegex(ValueError, "producer"):
                        subject.lane(root, "reference.json", "reference", COMMIT, SEAL, {})

    def test_should_enforce_aggregate_lane_log_bytes_at_the_exact_boundary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = release_lane(root)
            total = sum((root / row["path"]).stat().st_size for row in value["receipts"])
            with patch.object(subject, "LANE_LOG_BUDGET", total):
                subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":IMAGE})
            with patch.object(subject, "LANE_LOG_BUDGET", total - 1), self.assertRaisesRegex(ValueError, "log-budget"):
                subject.lane(root, "release.json", "release", COMMIT, SEAL, {"arm64":IMAGE})

    def test_should_reject_duplicate_deep_nonfinite_and_oversized_json(self):
        for raw in (b'{"a":1,"a":2}', b'{"v":NaN}', b'[' * 17 + b'0' + b']' * 17,
                    b'{"v":123456789012345678901}', b'"' + b'x' * 16385 + b'"'):
            with self.assertRaises(ValueError): release_json.document(raw)

    def test_should_scope_api_token_to_the_owned_gh_call_and_remove_it_on_failure(self):
        owner = Mock(environment={"PATH":"/tools"})
        def observe(arguments, deadline, stdout_limit):
            self.assertEqual(owner.environment["GH_TOKEN"], "private-token")
            self.assertNotIn("private-token", " ".join(arguments))
            raise OSError("controlled failure")
        owner.run.side_effect = observe
        with patch.dict(os.environ, {"GH_TOKEN":"private-token"}), self.assertRaises(OSError):
            delivery.authenticated_api(owner, "repos/owner/repo/actions/runs/1", 100)
        self.assertEqual(owner.environment, {"PATH":"/tools"})

    def test_should_emit_only_fixed_cli_errors_after_owned_command_retirement_failure(self):
        import docker_functional
        import image_refresh_build
        import platform_receipt
        import release_bundle
        with tempfile.TemporaryDirectory() as temporary:
            selected = Path(temporary)
            cases = [(subject,"assemble",["program",temporary,str(selected / "report")]),
                     (delivery,"verify",["program","verify",temporary]),
                     (release_bundle,"pack",["program",temporary,str(selected / "copy")]),
                     (docker_functional,"run",["program",str(selected / "config"),str(selected / "evidence")]),
                     (platform_receipt,"run",["program","/tmp/socket",str(selected / "platform")]),
                     (image_refresh_build,"run",["program",str(selected / "build"),"/tmp/socket","docker buildx"])]
            for module, name, arguments in cases:
                stderr = io.StringIO()
                with self.subTest(module=module.__name__), patch.object(module.sys,"argv",arguments), \
                     patch.object(module,name,side_effect=ActionError("controlled-retirement")), contextlib.redirect_stderr(stderr):
                    self.assertEqual(module.main(), 2)
                    self.assertNotIn("Traceback", stderr.getvalue())
                    self.assertNotIn("controlled-retirement", stderr.getvalue())

    def test_should_use_the_push_digest_and_reject_foreign_remote_config(self):
        raw = json.dumps({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                          "config":{"digest":IMAGE},"layers":[]}).encode()
        digest = "sha256:" + subject.digest(raw)
        pushed = ("tag: digest: " + digest + " size: 123\n").encode()
        self.assertEqual(delivery.pushed_digest(pushed, b""), digest)
        delivery.registry_manifest(raw, digest, IMAGE)
        with self.assertRaises(ValueError): delivery.registry_manifest(raw, digest, OTHER)
        with self.assertRaises(ValueError): delivery.registry_manifest(raw + b" ", digest, IMAGE)
        with self.assertRaises(ValueError): delivery.pushed_digest(pushed + pushed, b"")


if __name__ == "__main__":
    unittest.main()
