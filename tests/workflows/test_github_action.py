"""Actual Action entrypoint controls through private fake CLI processes."""
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

APP = Path(__file__).resolve().parents[2] / "tools/github-action"
sys.path.insert(0, str(APP))
import metadata
from process_owner import Runner
from settings import ActionError, DEFAULTS, Settings


class SettingsControls(unittest.TestCase):
    def test_should_validate_all_domains_before_effects(self):
        Settings.load({})
        with self.assertRaises(ActionError):
            Settings.load({"RUSTACK_ACTION_ACCESS_KEY": "", "RUSTACK_ACTION_SECRET_KEY": "", "RUSTACK_ACTION_S3_SKIP_AUTH": "false"})
        for key, values in {
            "image-tag": ["$(touch marker)", "x;true", "a/b", "a\n", "a" * 129],
            "port": ["0", "01", "65536", "1;true", "4566\n"],
            "wait-timeout": ["0", "01", "301", "1\n"],
            "default-region": ["US-EAST-1", "x y", "-east", "a" * 65],
            "log-level": ["INFO", "info;true"], "persistence": ["True", "false\n"],
            "stepfunctions-skip-auth": ["yes", "true\n"],
            "services": ["s3,s3", "s3,", "s3, lambda", "unknown"],
            "secret-key": ["", "x\ny", "x\ry", "a" * 513, "雪"],
        }.items():
            for value in values:
                with self.subTest(field=key), self.assertRaises(ActionError):
                    Settings.load({"RUSTACK_ACTION_" + key.upper().replace("-", "_"): value})

    def test_should_preserve_literal_authentication_and_redact_debug(self):
        secret = "opaque$(touch marker)`command`%0A=with spaces"
        settings = Settings.load({"RUSTACK_ACTION_SECRET_KEY": secret})
        self.assertEqual(settings.docker_environment()["SECRET_KEY"], secret)
        self.assertNotIn(secret, repr(settings))
        self.assertEqual(Settings.load({"RUSTACK_ACTION_ACCESS_KEY": "", "RUSTACK_ACTION_SECRET_KEY": ""})
                         .caller_environment()["AWS_SECRET_ACCESS_KEY"], "")


class MetadataControls(unittest.TestCase):
    def test_should_reject_deep_duplicate_nonfinite_and_excessive_json_before_acceptance(self):
        for body in [b'{"ready":true,"ready":false}', b'[' * 9 + b'0' + b']' * 9,
                     b'{"number":1e9999}', b'{"number":NaN}', b'"' + b'x' * 257 + b'"',
                     b'"\\ud800"', b'[' + b'0,' * 512 + b'0]']:
            with self.subTest(bodyBytes=len(body)), self.assertRaises(ActionError):
                metadata.document(body)

    def test_should_require_structured_ready_and_local_canonical_identity(self):
        metadata.readiness(b'{"ready":true,"services":{"stepfunctions":"running"}}')
        for body in [b'{"ready":false,"services":{"s3":"running"}}', b'{"ready":true,"services":{}}',
                     b'{"ready":true,"services":{"s3":"starting"}}', b'{"message":"ok"}']:
            with self.assertRaises(ActionError):
                metadata.readiness(body)
        for value in ["tcp://localhost:2375", "unix://relative", "unix:///run/../docker.sock", "unix:///run//docker.sock"]:
            with self.assertRaises(ActionError):
                metadata.endpoint(value)


class ProcessControls(unittest.TestCase):
    def test_should_join_success_and_keep_bounded_output_private(self):
        runner = Runner({"PATH": os.defpath})
        reply = runner.run([sys.executable, "-c", "print('joined')"], time.monotonic() + 3)
        self.assertEqual(reply.stdout, b"joined\n")
        self.assertNotIn("joined", repr(reply))
        self.assertFalse(runner.unsettled)

    def test_should_kill_and_join_a_stalled_actual_child(self):
        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "pid"
            code = "import os,time,pathlib;pathlib.Path(" + repr(str(pid_file)) + ").write_text(str(os.getpid()));time.sleep(30)"
            real_cutoff = time.monotonic() + 5
            start = time.monotonic()
            entered = False
            def clock():
                nonlocal entered
                if time.monotonic() >= real_cutoff:
                    raise ActionError("controlled-child-entry-deadline")
                if pid_file.exists() and pid_file.read_text().isdigit():
                    entered = True
                    return start + 2
                return start
            runner = Runner({"PATH": os.defpath}, clock=clock)
            with self.assertRaises(ActionError):
                runner.run([sys.executable, "-c", code], start + 1)
            self.assertTrue(entered)
            pid = int(pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
            self.assertFalse(runner.unsettled)

    def test_should_never_discharge_a_group_on_permission_denial(self):
        child = SimpleNamespace(pid=1234, poll=lambda: 0)
        with patch("os.killpg", side_effect=PermissionError):
            self.assertTrue(Runner._alive(child))

    def test_should_retire_a_causally_entered_actual_descendant_group(self):
        with tempfile.TemporaryDirectory() as directory:
            ready = Path(directory) / "descendant"
            identities = Path(directory) / "identities"
            descendant = "import os,time,pathlib;pathlib.Path(" + repr(str(ready)) + ").write_text(str(os.getpid()));time.sleep(30)"
            parent = ("import os,time,pathlib,json,signal,subprocess,sys\n"
                      "p=subprocess.Popen([sys.executable,'-c'," + repr(descendant) + "])\n"
                      "def stop(_s,_f):\n p.wait(timeout=2)\n raise SystemExit(0)\n"
                      "signal.signal(signal.SIGTERM,stop)\n"
                      "ready=pathlib.Path(" + repr(str(ready)) + ")\n"
                      "while not ready.exists() or not ready.read_text().isdigit(): time.sleep(.005)\n"
                      "pathlib.Path(" + repr(str(identities)) + ").write_text(json.dumps([os.getpid(),int(ready.read_text())]))\n"
                      "time.sleep(30)\n")
            start, outer = time.monotonic(), time.monotonic() + 5
            entered = None
            def clock():
                nonlocal entered
                if time.monotonic() >= outer:
                    raise ActionError("controlled-descendant-entry-deadline")
                if identities.exists():
                    try:
                        observed = json.loads(identities.read_text())
                    except ValueError:
                        return start
                    if len(observed) == 2 and all(type(pid) is int and pid > 0 for pid in observed):
                        entered = observed
                        return start + 2
                return start
            runner = Runner({"PATH": os.defpath}, clock=clock)
            with self.assertRaises(ActionError):
                runner.run([sys.executable, "-c", parent], start + 1)
            self.assertIsNotNone(entered)
            for pid in entered:
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
            self.assertFalse(runner.unsettled)

    def test_should_close_and_join_stdout_and_stderr_floods(self):
        for stream in ["stdout", "stderr"]:
            runner = Runner({"PATH": os.defpath})
            with self.assertRaises(ActionError):
                runner.run([sys.executable, "-c", "import sys,time;sys." + stream + ".buffer.write(b'x'*131072);time.sleep(30)"],
                           time.monotonic() + 3)
            self.assertFalse(runner.unsettled)


class ActualActionControls(unittest.TestCase):
    def setUp(self):
        metadata_path = os.environ.get("RUSTACK_ACTION_TEST_METADATA")
        if metadata_path is None:
            raise RuntimeError("Run make test-workflows-release-fixture to read actual Action YAML")
        raw = Path(metadata_path).read_bytes()
        assert len(raw) <= 131072
        self.action = json.loads(raw)
        self.assertEqual(self.action["defaults"], DEFAULTS)
        self.root = tempfile.TemporaryDirectory(prefix="case-", dir=Path(metadata_path).parent)
        self.addCleanup(self.root.cleanup)
        self.directory = Path(self.root.name)
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        for name in ("docker", "curl"):
            target = self.bin / name
            target.write_text("#!" + sys.executable + "\n" + Path(__file__).with_name("fake_action_cli.py").read_text())
            target.chmod(0o700)
        (self.bin / "python3").symlink_to(sys.executable)
        self.model = {"mode": "success", "engine": "725b98b4-e588-41b8-9cf5-54a5ad70c89f",
                      "source": "sha256:" + "b" * 64, "cid": "a" * 64}
        self.output = self.directory / "output"
        self.export = self.directory / "env"
        self.output.write_bytes(b"")
        self.export.write_bytes(b"")

    def run_action(self, inputs=None, mode="success", missing=False, host_environment=None):
        values = dict(DEFAULTS, **(inputs or {}))
        self.model.update(mode=mode, port=values["port"], missing=missing)
        (self.bin / "model.json").write_text(json.dumps(self.model))
        environment = {"PATH": str(self.bin) + os.pathsep + os.defpath, "HOME": self.root.name,
                       "RUNNER_TEMP": self.root.name, "GITHUB_OUTPUT": str(self.output), "GITHUB_ENV": str(self.export),
                       "PYTHONDONTWRITEBYTECODE": "1"}
        for name, expression in self.action["environment"].items():
            if expression == "${{ github.action_path }}":
                environment[name] = str(APP.parents[1])
            else:
                prefix, suffix = "${{ inputs.", " }}"
                self.assertTrue(expression.startswith(prefix) and expression.endswith(suffix))
                environment[name] = values[expression[len(prefix):-len(suffix)]]
        environment.update(host_environment or {})
        before = time.monotonic()
        driver = Runner(environment)
        driver.environment = environment
        reply = driver.run(["/bin/bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", self.action["run"]],
                           time.monotonic() + 600, working_directory=self.directory)
        self.assertLessEqual(len(reply.stdout), 32768)
        self.assertLessEqual(len(reply.stderr), 32768)
        events = self.bin / "events.jsonl"
        observed = [json.loads(line) for line in events.read_text().splitlines()] if events.exists() else []
        model = json.loads((self.bin / "model.json").read_text())
        return reply, observed, model, time.monotonic() - before

    def test_should_execute_the_actual_run_body_and_handoff_exact_metadata(self):
        reply, events, model, _ = self.run_action()
        self.assertEqual(reply.code, 0)
        self.assertTrue(model["running"])
        self.assertFalse(model.get("removed", False))
        self.assertIn("container_id=" + model["cid"], self.output.read_text())
        self.assertIn("AWS_ENDPOINT_URL=http://localhost:4566\n", self.export.read_text())
        self.assertFalse(list(self.directory.glob("rustack-action-*")))
        self.assertTrue(any(event["tool"] == "curl" for event in events))

    def test_should_reject_shell_payloads_before_any_child_or_output(self):
        for key, value in [("image-tag", "$(touch injected)"), ("port", "4566;touch injected"),
                           ("wait-timeout", "1$(touch injected)"), ("default-region", "US-EAST-1"),
                           ("log-level", "info'"), ("services", "s3\nSECRET_KEY=bad"),
                           ("stepfunctions-skip-auth", "true;touch injected")]:
            reply, events, _, _ = self.run_action({key: value})
            self.assertNotEqual(reply.code, 0)
            self.assertEqual(events, [])
            self.assertEqual(self.output.read_bytes(), b"")
            self.assertEqual(self.export.read_bytes(), b"")
            self.assertFalse((self.directory / "injected").exists())

    def test_should_keep_credential_metacharacters_literal_and_out_of_argv(self):
        secret = "private$(touch injected)`whoami`%0A=opaque"
        reply, events, model, _ = self.run_action({"secret-key": secret})
        self.assertEqual(reply.code, 0)
        self.assertEqual(model["serverEnvironment"]["SECRET_KEY"], secret)
        self.assertNotIn(secret, json.dumps([event["args"] for event in events]))
        self.assertNotIn(secret, reply.stderr.decode())
        self.assertFalse((self.directory / "injected").exists())
        self.assertIn("AWS_SECRET_ACCESS_KEY=" + secret + "\n", self.export.read_text())

    def test_should_omit_absent_credentials_and_reject_enabled_strict_auth_before_children(self):
        empty = {"access-key": "", "secret-key": ""}
        reply, _, model, _ = self.run_action(empty)
        self.assertEqual(reply.code, 0)
        self.assertNotIn("ACCESS_KEY", model["serverEnvironment"])
        self.assertNotIn("SECRET_KEY", model["serverEnvironment"])
        self.assertIn("AWS_SECRET_ACCESS_KEY=\n", self.export.read_text())

    def test_should_resolve_explicit_context_before_host(self):
        reply, events, _, _ = self.run_action(host_environment={"DOCKER_CONTEXT": "selected", "DOCKER_HOST": "unix:///tmp/wrong.sock"})
        self.assertEqual(reply.code, 0)
        self.assertEqual(events[0]["args"][-2:], ["--", "selected"])
        self.assertTrue(all(e["args"][:2] == ["--host", "unix:///tmp/rustack-action-control.sock"]
                            for e in events if e["tool"] == "docker" and e["args"][0] != "context"))

    def test_should_refuse_an_effective_remote_context_despite_a_local_host_variable(self):
        self.model["contextEndpoint"] = "ssh://remote"
        reply, events, _, _ = self.run_action(host_environment={"DOCKER_CONTEXT": "remote", "DOCKER_HOST": "unix:///tmp/wrong.sock"})
        self.assertNotEqual(reply.code, 0)
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["args"][:2], ["context", "inspect"])

    def test_should_reject_redirect_status_and_disable_ambient_curl_configuration(self):
        reply, events, model, _ = self.run_action(mode="redirect-http")
        self.assertNotEqual(reply.code, 0)
        self.assertTrue(model.get("removed"))
        curl = next(e for e in events if e["tool"] == "curl")
        self.assertEqual(curl["args"][0], "--disable")
        self.assertIn("%{stderr}%{http_code}", curl["args"])

    def test_should_fail_a_fifo_handoff_without_blocking_and_join_exact_cleanup(self):
        self.output.unlink()
        os.mkfifo(self.output, 0o600)
        reply, _, model, elapsed = self.run_action()
        self.assertNotEqual(reply.code, 0)
        self.assertTrue(model.get("removed"))
        self.assertLess(elapsed, 5)

    def test_should_pull_once_only_after_successful_absence(self):
        reply, events, _, _ = self.run_action(missing=True)
        self.assertEqual(reply.code, 0)
        self.assertEqual(sum("pull" in event["args"] for event in events), 1)

    def test_should_never_treat_lookup_failure_as_absence(self):
        reply, events, _, _ = self.run_action(mode="lookup-failure")
        self.assertNotEqual(reply.code, 0)
        self.assertFalse(any("pull" in e["args"] or "run" in e["args"] for e in events))

    def test_should_leave_a_foreign_existing_name_uninspected_and_untouched(self):
        reply, events, _, _ = self.run_action(mode="foreign-name")
        self.assertNotEqual(reply.code, 0)
        self.assertFalse(any("rm" in e["args"] or "container" in e["args"] for e in events))

    def test_should_refuse_unrelated_http_and_remove_only_the_exact_stopped_id(self):
        reply, events, model, _ = self.run_action(mode="unrelated-http")
        self.assertNotEqual(reply.code, 0)
        self.assertTrue(model.get("removed"))
        removal = [e for e in events if "rm" in e["args"]]
        self.assertEqual(len(removal), 1)
        self.assertEqual(removal[0]["args"][-1], model["cid"])
        self.assertFalse(any("--force" in e["args"] or "-f" in e["args"] for e in events))

    def test_should_refuse_ready_false(self):
        reply, _, model, _ = self.run_action(mode="not-ready")
        self.assertNotEqual(reply.code, 0)
        self.assertTrue(model.get("removed"))

    def test_should_refuse_wrong_publication_and_exited_container(self):
        for mode in ("wrong-publication", "exited-cid"):
            with self.subTest(mode=mode):
                reply, _, model, _ = self.run_action(mode=mode)
                self.assertNotEqual(reply.code, 0)
                self.assertTrue(model.get("removed"))
                self.assertEqual(self.output.read_bytes(), b"")

    def test_should_refuse_daemon_rebinding_without_mutating_the_other_engine(self):
        reply, events, _, _ = self.run_action(mode="changed-engine")
        self.assertNotEqual(reply.code, 0)
        self.assertFalse(any("kill" in e["args"] or "rm" in e["args"] for e in events))

    def test_should_honor_real_seconds_and_join_failed_readiness_cleanup(self):
        reply, events, model, elapsed = self.run_action({"wait-timeout": "1"}, mode="never-healthy")
        self.assertNotEqual(reply.code, 0)
        self.assertGreaterEqual(elapsed, 1)
        self.assertLess(elapsed, 8)
        self.assertTrue(model.get("removed"))
        run = next(e for e in events if "run" in e["args"])
        stop = next(e for e in events if "kill" in e["args"])
        self.assertGreaterEqual(stop["at"] - run["at"], 0.95)

    def test_should_never_accept_a_late_http_success(self):
        reply, _, model, _ = self.run_action({"wait-timeout": "1"}, mode="late-http")
        self.assertNotEqual(reply.code, 0)
        self.assertTrue(model.get("removed"))
        self.assertEqual(self.output.read_bytes(), b"")


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    if len(sys.argv) != 2:
        raise SystemExit("Use make test-workflows-action-fixture")
    os.environ["RUSTACK_ACTION_TEST_METADATA"] = sys.argv[1]
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__]))
    print(json.dumps({"schemaVersion": 1, "cases": result.testsRun, "successful": result.wasSuccessful()}))
    raise SystemExit(0 if result.wasSuccessful() else 1)
