"""Independent literal sample, causal-flood, cold-stage and accounting controls."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import unittest
import uuid
from types import SimpleNamespace
from unittest.mock import patch

from image_performance_report import validate
from image_performance_support import cold_stages, decode, exact_samples, live, overhead, samples, verify_stages, wait_task_baseline
from settings import ActionError


def warm():
    return {"schemaVersion": 1, "phase": "warm", "referenceQualification": "pending", "os": "linux",
            "architecture": "aarch64", "python": "3.12.3", "sdkVersion": "1.43.118",
            "baselineProcess": {"rssKiB": 10000, "tasks": 8}, "emptyAfterWorkload": True,
            "measurement": {"totalNanos": [15000001] * 100, "handlerNanos": [10000001] * 100,
                            "overhead": {"observedMicros": [5000] * 100,
                                         "p50Micros": 5000, "p95Micros": 5000, "maxMicros": 5000}}}


def identities():
    return [str(uuid.UUID(int=index + 1)) for index in range(20)]


def logs(names):
    return b"\n".join(("INFO rustack_lambda_core::docker::initialization: Image environment ready "
                       "environment_id=" + name + " relay_micros=100 ric_micros=200").encode() for name in names)


class ImageCacheModel:
    first = "sha256:" + "a" * 64
    second = "sha256:" + "b" * 64

    def __init__(self, reference=None, move=False, malformed=None):
        self.reference, self.move, self.malformed = reference, move, malformed
        self.aliases = {reference: self.first} if reference else {}
        self.tags = []

    def dispatch(self, owner, endpoint, engine, arguments, deadline):
        operation = arguments[4]
        if operation == "ls":
            reference = arguments[arguments.index("--filter") + 1].removeprefix("reference=")
            if reference == self.reference and self.malformed is not None: return self.malformed
            return (self.aliases[reference] + "\n").encode() if reference in self.aliases else b""
        if operation == "inspect":
            assert arguments[5] in self.aliases
            return (self.aliases[arguments[5]] + "\n").encode()
        assert operation == "tag"
        immutable, alias = arguments[5:7]
        assert immutable in (self.first, self.second)
        self.tags.append((immutable, alias))
        self.aliases[alias] = immutable
        if self.move: self.aliases[self.reference] = self.second
        return b""


class ImagePerformanceControls(unittest.TestCase):
    def test_should_retain_old_source_ids_across_a_later_fixture_tag_rebuild(self):
        import image_performance_images as source
        model = ImageCacheModel("rustack/lambda-runtime:integration-arm64")
        with patch.object(source, "call", return_value=b"12345678-1234-4234-8234-123456789abc\n"), \
             patch.object(source, "fenced", side_effect=model.dispatch):
            result = source.retain_sources("/tmp/socket", "arm64", None, 100)
        self.assertEqual(result["referenceCount"], 7)
        self.assertEqual(len(result["preserved"]), 1)
        row = result["preserved"][0]
        self.assertEqual(row["id"], model.first)
        self.assertEqual(model.tags, [(model.first, row["cacheAlias"])])
        model.aliases[model.reference] = model.second
        self.assertEqual(model.aliases[row["cacheAlias"]], model.first)

    def test_should_reject_a_source_alias_move_and_preserve_only_the_captured_id(self):
        import image_performance_images as source
        model = ImageCacheModel("rustack/lambda-runtime:integration-arm64", move=True)
        with patch.object(source, "call", return_value=b"12345678-1234-4234-8234-123456789abc\n"), \
             patch.object(source, "fenced", side_effect=model.dispatch), self.assertRaises(ValueError):
            source.retain_sources("/tmp/socket", "arm64", None, 100)
        self.assertEqual(model.tags, [(model.first, source.cache_alias(model.first))])
        self.assertEqual(model.aliases[source.cache_alias(model.first)], model.first)

    def test_should_allow_initial_provisioning_but_reject_invalid_or_ambiguous_source_identity(self):
        import image_performance_images as source
        model = ImageCacheModel()
        with patch.object(source, "call", return_value=b"12345678-1234-4234-8234-123456789abc\n"), \
             patch.object(source, "fenced", side_effect=model.dispatch):
            self.assertEqual(source.retain_sources("/tmp/socket", "arm64", None, 100)["preserved"], [])
        self.assertEqual(model.tags, [])
        for malformed in (b"invalid\n", (model.first + "\n" + model.second + "\n").encode()):
            model = ImageCacheModel("rustack/lambda-runtime:integration-arm64", malformed=malformed)
            with patch.object(source, "call", return_value=b"12345678-1234-4234-8234-123456789abc\n"), \
                 patch.object(source, "fenced", side_effect=model.dispatch), self.assertRaises((ValueError, ActionError)):
                source.retain_sources("/tmp/socket", "arm64", None, 100)
            self.assertEqual(model.tags, [])
        with patch.object(source, "call") as dispatched, self.assertRaises(ValueError):
            source.retain_sources("/tmp/socket", "unrecognized", None, 100)
        dispatched.assert_not_called()

    def test_should_retain_sources_before_any_canonical_fixture_build(self):
        lines = (Path(__file__).resolve().parents[2] / "Makefile").read_text().splitlines()
        start = lines.index("lambda-docker-fixtures:")
        self.assertEqual(lines[start + 1], "\t@$(MAKE) --no-print-directory -j1 workflows-retain-fixture-sources")
        self.assertIn("lambda-runtime-image", lines[start + 2])

    def test_should_propagate_the_original_barrier_failure_with_bounded_private_diagnostics(self):
        import concurrent.futures
        import contextlib
        import io
        import threading
        from image_performance_sdk import observe_flood
        from image_performance_support import MeasurementError
        failure = MeasurementError("flood-observation-expired")
        def waiting(rows, count): raise failure
        collector = SimpleNamespace(condition=threading.Condition(), started={}, released=set(), ended={}, wait_for=waiting)
        future = concurrent.futures.Future()
        output = io.StringIO()
        with contextlib.redirect_stderr(output), self.assertRaises(MeasurementError) as raised:
            observe_flood(collector, {}, 94, "begin", future)
        self.assertIs(raised.exception, failure)
        report = json.loads(output.getvalue().split(" ", 1)[1])
        self.assertEqual((report["phase"], report["expected"], report["outcome"]), ("begin", 94, "pending"))
        self.assertGreaterEqual(report["elapsedMicros"], 0)
        self.assertLess(len(output.getvalue()), 512)

    def test_should_retain_fixed_flood_phase_counts_and_completed_invocation_outcome(self):
        import concurrent.futures
        import threading
        from image_performance_sdk import flood_failure
        collector = SimpleNamespace(condition=threading.Condition(), started={"secret": {}}, released=set(), ended={})
        future = concurrent.futures.Future()
        self.assertEqual(flood_failure(collector, "begin", 2, future),
                         {"phase": "begin", "expected": 2, "begun": 1, "released": 0, "ended": 0, "outcome": "pending"})
        class Failure(Exception):
            response = {"Error": {"Code": "ServiceException", "Message": "function exited with status 1; secret"}}
        future.set_exception(Failure("secret"))
        result = flood_failure(collector, "end", 2, future)
        self.assertEqual(result["phase"], "end")
        self.assertEqual(result["outcome"], "function-exit")
        self.assertNotIn("secret", json.dumps(result))
        future = concurrent.futures.Future(); future.set_result(None)
        self.assertEqual(flood_failure(collector, "begin", 1, future)["outcome"], "unexpected-success")

    def test_should_anchor_imported_content_and_reject_conflicting_cache_aliases(self):
        import image_performance_images as source
        expected = {"id": "sha256:" + "a" * 64}
        alias = "rustack-workflows-cache:d102-" + "a" * 64
        for listing, expected_tags in ((b"", 1), ((expected["id"] + "\n").encode(), 0)):
            tags = []
            def dispatch(owner, endpoint, engine, arguments, deadline):
                if arguments[4] == "ls": return listing
                if arguments[4] == "inspect": return (expected["id"] + "\n").encode()
                self.assertEqual(arguments[4], "tag")
                tags.append(arguments[-3:])
                return b""
            with patch.object(source, "fenced", side_effect=dispatch), \
                 patch.object(source, "image", return_value=expected):
                self.assertEqual(source.anchor(None, "unix:///tmp/socket", expected, 100, "engine"), alias)
                self.assertEqual(len(tags), expected_tags)
                if expected_tags: self.assertEqual(tags, [["tag", expected["id"], alias]])
        with patch.object(source, "fenced", return_value=("sha256:" + "b" * 64 + "\n").encode()) as dispatch, \
             patch.object(source, "image") as inspect:
            with self.assertRaises(ValueError): source.anchor(None, "unix:///tmp/socket", expected, 100, "engine")
            self.assertEqual(dispatch.call_count, 1)
            inspect.assert_not_called()

    def test_should_include_the_initial_task_observation_and_reject_its_late_match(self):
        now = [0]
        def first():
            now[0] += 1000000
            return 15
        self.assertEqual(wait_task_baseline(first, 15, 0, clock=lambda: now[0]), (1000, 15))
        now[0] = 0
        def late():
            now[0] = 2000000000
            return 15
        with self.assertRaises(ValueError): wait_task_baseline(late, 15, 0, clock=lambda: now[0])
        invoked = []
        with self.assertRaises(ValueError):
            wait_task_baseline(lambda: invoked.append(True), 15, 0, clock=lambda: 2000000000)
        self.assertFalse(invoked)
    def test_should_probe_the_actual_readiness_route_inside_the_live_bracket(self):
        from image_performance_sdk import capability
        class Connection:
            path = None
            status = 200
            def request(self, method, path): self.path = path
            def getresponse(self): return self
            def read(self, limit): return b'{"ready":true,"services":{"lambda":"running","s3":"disabled"}}'
        connection = Connection()
        result, duration = capability("http://127.0.0.1:4566", connection, "/_health/ready")
        self.assertEqual(connection.path, "/_health/ready")
        self.assertTrue(result["ready"])
        self.assertLess(duration, 2000000)

    def test_should_bound_remaining_aggregate_budget_and_save_only_the_captured_id(self):
        import image_performance_images as source
        infinity = source.resource.RLIM_INFINITY
        self.assertEqual(source.file_limit(infinity, infinity, 2 * source.LIMIT), source.LIMIT)
        self.assertEqual(source.file_limit(4096, 8192, source.LIMIT), 4096)
        self.assertEqual(source.file_limit(infinity, infinity, 2048), 2048)
        for remaining in (0, True, 2 * source.LIMIT + 1):
            with self.assertRaises(ValueError): source.file_limit(infinity, infinity, remaining)
        immutable = "sha256:" + "a" * 64
        with patch.object(source.shutil, "which", return_value="/usr/bin/docker"), \
             patch.object(source.resource, "getrlimit", return_value=(infinity, infinity)), \
             patch.object(source.resource, "setrlimit") as cap, patch.object(source.os, "execv") as execute:
            source.save("/tmp/docker.sock", "/tmp/archive", immutable, "2048")
            cap.assert_called_once_with(source.resource.RLIMIT_FSIZE, (2048, 2048))
            self.assertEqual(execute.call_args.args[1][-1], immutable)
            self.assertNotIn(source.SOURCES[0], execute.call_args.args[1])

    def test_should_fence_each_effect_and_retain_uncertainty_without_retry(self):
        import image_performance_images as source
        class Owner:
            def __init__(self, replies): self.replies, self.calls = iter(replies), []
            def run(self, arguments, deadline):
                self.calls.append(arguments)
                return SimpleNamespace(code=0, stdout=next(self.replies), stderr=b"")
        expected = "12345678-1234-4234-8234-123456789abc"
        other = b"11111111-1111-4111-8111-111111111111\n"
        before = Owner([other])
        with self.assertRaises(ValueError): source.fenced(before, "unix:///tmp/docker.sock", expected, ["effect"], float("inf"))
        self.assertEqual(len(before.calls), 1)
        after = Owner([expected.encode() + b"\n", b"done", other])
        with self.assertRaises(ValueError): source.fenced(after, "unix:///tmp/docker.sock", expected, ["effect"], float("inf"))
        self.assertEqual(sum(arguments == ["effect"] for arguments in after.calls), 1)

    def test_should_import_each_isolated_entrypoint_and_fail_closed_on_missing_arguments(self):
        for name in ("image_performance_images.py", "linux_image_performance.py",
                     "image_performance_report.py", "image_performance_sdk.py"):
            result = subprocess.run([sys.executable, "-I", "-B", str(Path(__file__).with_name(name))],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
            with self.subTest(name=name):
                self.assertEqual(result.returncode, 2)
                self.assertNotIn(b"Traceback", result.stderr)
                self.assertNotIn(b"ModuleNotFoundError", result.stderr)

    def test_should_use_nearest_ranks_for_twenty_and_one_hundred_ordered_samples(self):
        for count, expected in ((20, (10, 19, 20)), (100, (50, 95, 100))):
            values = list(range(count, 0, -1))
            result = samples(values, count)
            self.assertEqual(result["observedMicros"], values)
            self.assertEqual(tuple(result[key] for key in ("p50Micros", "p95Micros", "maxMicros")), expected)
        for values, count in (([1] * 99, 100), ([1] * 21, 20), ([True] * 100, 100), ([1] * 10, 10)):
            with self.assertRaises(ValueError):
                samples(values, count)

    def test_should_subtract_nanoseconds_before_rounding_and_reject_invalid_handler_clock(self):
        self.assertEqual(overhead(1001, 999), 1)
        self.assertEqual(overhead(2001, 1000), 2)
        self.assertEqual(overhead(1000, 1000), 0)
        for total, handler in ((1, 2), (0, 0), (1, True), (60000000001, 1)):
            with self.assertRaises(ValueError):
                overhead(total, handler)

    def test_should_accept_actual_raw_warm_shape_and_reject_mutated_ranks_count_and_budget(self):
        validate(warm(), "warm")
        for change in ("count", "rank", "handler", "budget", "cleanup", "reference"):
            value = warm()
            if change == "count": value["measurement"]["totalNanos"].pop()
            elif change == "rank": value["measurement"]["overhead"]["p95Micros"] += 1
            elif change == "handler": value["measurement"]["handlerNanos"][0] = 60000000000
            elif change == "cleanup": value["emptyAfterWorkload"] = False
            elif change == "reference": value["referenceQualification"] = "qualified"
            else:
                value["measurement"]["totalNanos"] = [61000001] * 100
                value["measurement"]["overhead"] = samples([51000] * 100, 100)
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate(value, "warm")

    def test_should_reject_boolean_rank_even_when_python_equality_would_match_one(self):
        value = samples([1] * 100, 100)
        value["p50Micros"] = True
        with self.assertRaises(ValueError):
            exact_samples(value, 100)
        names = identities()
        stages = cold_stages(logs(names).replace(b"=100", b"=1").replace(b"=200", b"=1"), names, [2] * 20)
        stages["relay"]["p95Micros"] = True
        with self.assertRaises(ValueError): verify_stages(stages, names, [2] * 20)

    def test_should_affiliate_twenty_exact_cold_stages_by_identity_rather_than_line_order(self):
        names = identities()
        value = cold_stages(logs(list(reversed(names))), names, [301] * 20)
        self.assertEqual(value["relay"]["observedMicros"], [100] * 20)
        self.assertEqual(value["functionRic"]["observedMicros"], [200] * 20)
        for raw in (logs(names[:-1]), logs(names + names[:1]), logs(identities()[1:] + [str(uuid.uuid4())]),
                    logs(names).replace(b"ric_micros=200", b"ric_micros=400"),
                    logs(names).replace(b"relay_micros=100", b"relay_micros=unknown"), b"x" * 32769):
            with self.assertRaises(ValueError):
                cold_stages(raw, names, [301] * 20)

    def test_should_require_live_work_and_a_positive_unexhausted_flood_span(self):
        flood = {"requestId": str(uuid.uuid4()), "beforeBytes": 1024, "afterBytes": 4096}
        live(1, 1, 1999999, False, flood)
        for before, after, ready, done in ((0, 1, 1, False), (1, 0, 1, False),
                                         (1, 1, 2000000, False), (1, 1, 1, True), (True, 1, 1, False)):
            with self.assertRaises(ValueError):
                live(before, after, ready, done, flood)
        for begin, end in ((0, 1), (1024, 1024), (4096, 1024), (1024, 1048576)):
            with self.assertRaises(ValueError):
                live(1, 1, 1, False, dict(flood, beforeBytes=begin, afterBytes=end))

    def test_should_exclude_only_the_unchanged_complete_pre_sdk_log_prefix(self):
        names = identities()
        startup = logs([str(uuid.uuid4()), str(uuid.uuid4())]) + b"\n"
        raw = startup + logs(names)
        value = cold_stages(raw, names, [301] * 20, startup)
        verify_stages(value, names, [301] * 20)
        for baseline in (startup[:-1], b"different\n", raw):
            with self.assertRaises(ValueError):
                cold_stages(raw, names, [301] * 20, baseline)
        changed = copy.deepcopy(value)
        changed["events"][0]["environmentId"] = str(uuid.uuid4())
        with self.assertRaises(ValueError): verify_stages(changed, names, [301] * 20)
        changed = copy.deepcopy(value)
        changed["relay"]["p95Micros"] += 1
        with self.assertRaises(ValueError): verify_stages(changed, names, [301] * 20)

    def test_should_reject_cycle_task_growth_nonlive_brackets_and_missing_priming(self):
        value = warm()
        value["phase"] = "cycles"
        value["measurement"] = {"readiness": samples([100] * 100, 100), "timeout": samples([1200000] * 100, 100),
            "floods": [{"requestId": str(uuid.uuid4()), "beforeBytes": 1024, "afterBytes": 4096} for _ in range(100)],
            "processStats": [{"rssKiB": 10000, "tasks": 8, "runtimeTasks": 15} for _ in range(100)],
            "completedCycles": 100, "containerHighWater": 2, "emptyAfterEachTimeout": True,
            "runtimeTaskBaseline": 15, "taskQuiescence": samples([100] * 100, 100),
            "liveBrackets": [[1, 1, False] for _ in range(100)], "primingInvocations": 1, "primingCleanupConfirmed": True}
        validate(value, "cycles")
        for mutation in ("growth", "bracket", "priming", "timeout-count", "quiescence", "request-reuse"):
            broken = copy.deepcopy(value)
            measured = broken["measurement"]
            if mutation == "growth": measured["processStats"][-1]["runtimeTasks"] = 16
            elif mutation == "bracket": measured["liveBrackets"][0][2] = True
            elif mutation == "priming": measured["primingCleanupConfirmed"] = False
            elif mutation == "timeout-count": measured["timeout"]["observedMicros"].pop()
            elif mutation == "request-reuse": measured["floods"][-1] = measured["floods"][0]
            else: measured["taskQuiescence"] = samples([2000000] * 100, 100)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate(broken, "cycles")

    def test_should_keep_existing_decoder_member_default_and_bound_measurement_expansion(self):
        from metadata import document
        raw = json.dumps(list(range(513))).encode()
        with self.assertRaises(ActionError): document(raw)
        self.assertEqual(decode(raw), list(range(513)))
        for value in (b"[" * 9 + b"0" + b"]" * 9,
                      json.dumps(list(range(4097))).encode(), b" " * 32769,
                      b'{"x":1,"x":2}', b'{"x":NaN}'):
            with self.assertRaises(ActionError): decode(value)
        for limit in (True, 0, 4097):
            with self.assertRaises(ActionError): document(b"{}", member_limit=limit)


if __name__ == "__main__":
    unittest.main()
