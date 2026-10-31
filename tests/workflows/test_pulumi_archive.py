"""Controls for the independent archive oracle; no fabricated acceptance state."""
import hashlib
import copy
import time
import io
import json
import tarfile
import tempfile
from pathlib import Path
import unittest

from pulumi_archive import CAP, SIGNATURE, SECRET, checkpoint, document, evidence_directory, exec_complete, exec_stream, unpack

SECRETS = (b"local-key", b"local-secret")
PATH = "backend/.pulumi/stacks/rustack-iac/fixture.json"
ARN = "arn:aws:states:us-east-1:000000000000:stateMachine:Target"
MANIFEST = {"version": "3.273.0", "magic": hashlib.sha256(b"3.273.0").hexdigest(), "time": "2026-10-31T07:00:00Z"}
STACK = "organization/rustack-iac/fixture"


def archive(name, value, kind=tarfile.REGTYPE):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w", format=tarfile.USTAR_FORMAT) as stream:
        item = tarfile.TarInfo(name)
        item.type, item.size = kind, len(value)
        stream.addfile(item, io.BytesIO(value))
    return out.getvalue()


class ArchiveTests(unittest.TestCase):
    def test_should_refuse_truncated_oversized_stderr_or_expired_exec_streams(self):
        def frame(body, kind=1):
            return bytes([kind, 0, 0, 0]) + len(body).to_bytes(4, "big") + body
        valid = frame(b"actual") + frame(b" bytes")
        self.assertEqual(exec_stream(io.BytesIO(valid), time.monotonic() + 2), b"actual bytes")
        invalid = [valid[:7], valid[:-1], frame(b"private error", 2), frame(b"wrong stream", 0),
                   bytes([1, 0, 0, 0]) + (2 * 1024 * 1024 + 1).to_bytes(4, "big"), frame(b"a") * 4097,
                   frame(b"a" * (2 * 1024 * 1024)) * 9]
        for raw in invalid:
            with self.assertRaises(ValueError):
                exec_stream(io.BytesIO(raw), time.monotonic() + 2)
        with self.assertRaises(ValueError):
            exec_stream(io.BytesIO(valid), time.monotonic() - 1)

    def test_should_join_only_matching_unprivileged_successful_exec_identity(self):
        command = ["/var/lang/bin/node", "/var/task/pulumi/evidence_reader.js", "request", "logs"]
        observed = {"ID": "a" * 64, "ContainerID": "b" * 64, "Running": False, "ExitCode": 0,
            "ProcessConfig": {"entrypoint": command[0], "arguments": command[1:], "privileged": False,
                              "tty": False, "user": "1000:1000"}}
        self.assertTrue(exec_complete(observed, "a" * 64, "b" * 64, command))
        self.assertFalse(exec_complete({**observed, "Running": True}, "a" * 64, "b" * 64, command))
        for key, value in (("ID", "c" * 64), ("ContainerID", "c" * 64), ("ExitCode", 1), ("ExitCode", False), ("Running", 0)):
            with self.assertRaises(ValueError):
                exec_complete({**observed, key: value}, "a" * 64, "b" * 64, command)
        for key, value in (("entrypoint", "/bin/sh"), ("arguments", []), ("user", "0"), ("privileged", True), ("tty", True)):
            invalid = copy.deepcopy(observed)
            invalid["ProcessConfig"][key] = value
            with self.assertRaises(ValueError):
                exec_complete(invalid, "a" * 64, "b" * 64, command)

    def test_should_normalize_owned_evidence_alias_and_exclusively_create_private_child(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as directory:
            created = evidence_directory(directory)
            self.assertEqual(created, Path(directory).resolve() / "iac-private")
            self.assertEqual(created.stat().st_mode & 0o777, 0o700)
            with self.assertRaises(FileExistsError):
                evidence_directory(directory)
        for invalid in ("relative", "\x00", "x" * 4097):
            with self.assertRaises(ValueError):
                evidence_directory(invalid)

    def test_should_reject_plaintext_and_json_escaped_secrets(self):
        for secret in SECRETS:
            for raw in (b'{"value":"' + secret + b'"}', json.dumps({"value": "".join("\\u%04x" % c for c in secret)}).encode().replace(b"\\\\u", b"\\u")):
                with self.assertRaises(ValueError):
                    document(raw, SECRETS)
                for name in ("backend/actual.json", "backend/fixture.json.bak", "backend/metadata.yaml"):
                    with self.assertRaises(ValueError):
                        unpack(archive(name, raw), "backend", SECRETS)

    def test_should_scan_json_strings_in_actual_stdout_and_stderr_archive_names(self):
        for suffix in ("stdout", "stderr"):
            for secret in SECRETS:
                escaped = "".join("\\u%04x" % c for c in secret)
                raw = ('CLI diagnostic: {"credential":"' + escaped + '"}\n').encode()
                with self.assertRaises(ValueError):
                    unpack(archive("logs/8-reject-publish." + suffix, raw), "logs", SECRETS)
            for raw in (b'prefix "unclosed', b'prefix "bad\\q"', b'prefix "\xff"'):
                with self.assertRaises(ValueError):
                    unpack(archive("logs/8-reject-publish." + suffix, raw), "logs", SECRETS)
            safe = b'plain output\n{"message":"safe \\u0061"}\n'
            self.assertEqual(unpack(archive("logs/1-version." + suffix, safe), "logs", SECRETS),
                             {"logs/1-version." + suffix: safe})

    def test_should_reject_archive_traversal_specials_truncation_trailing_and_caps(self):
        for name, kind in (("backend/../escape", tarfile.REGTYPE), ("/backend/file", tarfile.REGTYPE),
                           ("backend/link", tarfile.SYMTYPE), ("backend/fifo", tarfile.FIFOTYPE)):
            with self.assertRaises(ValueError):
                unpack(archive(name, b"", kind), "backend", SECRETS)
        valid = archive("backend/actual.json", b'{}')
        for raw in (valid + b"x" * 512, bytes(CAP + 512), valid[:1024]):
            with self.assertRaises(ValueError):
                unpack(raw, "backend", SECRETS)
        self.assertEqual(unpack(valid, "backend", SECRETS), {"backend/actual.json": b'{}'})
        with self.assertRaises(ValueError):
            unpack(archive("backend/huge", b"a" * (2 * 1024 * 1024 + 1)), "backend", SECRETS)

    def test_should_require_live_encrypted_inputs_before_destroy_and_distinct_later_states(self):
        encrypted = {SIGNATURE: SECRET, "ciphertext": "v1:encrypted-private-input"}
        resources = [{"type": "pulumi:pulumi:Stack"}, {"type": "pulumi:providers:aws",
            "inputs": {"accessKey": encrypted, "secretKey": encrypted}},
            {"type": "aws:sfn/stateMachine:StateMachine", "id": ARN}]
        def source(entries):
            raw = json.dumps({"version": 3, "checkpoint": {"stack": STACK, "latest": {"manifest": MANIFEST, "resources": entries}}}).encode()
            return {PATH: raw}, {"path": PATH, "present": True, "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
        files, observation = source(resources)
        checkpoint(files, observation, "pre-destroy", ARN, SECRETS)
        with self.assertRaises(ValueError):
            checkpoint(files, observation, "post-destroy", ARN, SECRETS)
        files, observation = source(resources[:1])
        checkpoint(files, observation, "post-destroy", ARN, SECRETS)
        with self.assertRaises(ValueError):
            checkpoint(files, observation, "pre-destroy", ARN, SECRETS)
        checkpoint({}, {"path": PATH, "present": False, "bytes": 0, "sha256": ""}, "post-removal", ARN, SECRETS)
        with self.assertRaises(ValueError):
            checkpoint(files, {"path": PATH, "present": False, "bytes": 0, "sha256": ""}, "post-removal", ARN, SECRETS)

    def test_should_accept_only_real_stack_and_manifest_with_omitted_empty_resources(self):
        valid = {"version": 3, "checkpoint": {"stack": STACK, "latest": {"manifest": MANIFEST}}}
        def observed(value):
            raw = json.dumps(value).encode()
            return {PATH: raw}, {"path": PATH, "present": True, "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
        files, info = observed(valid)
        checkpoint(files, info, "post-destroy", ARN, SECRETS)
        with self.assertRaises(ValueError):
            checkpoint(files, info, "pre-destroy", ARN, SECRETS)
        with self.assertRaises(ValueError):
            checkpoint(files, {**info, "sha256": "0" * 64}, "post-destroy", ARN, SECRETS)
        invalid = [{}, {"stack": STACK}, {"stack": STACK, "latest": {}},
                   {"stack": "other", "latest": {"manifest": MANIFEST}},
                   {"latest": {"manifest": MANIFEST}}]
        invalid.extend({"stack": STACK, "latest": {"manifest": {**MANIFEST, key: value}}}
                       for key, value in (("version", "0.0.0"), ("magic", "0" * 64), ("time", "not-a-time")))
        for current in invalid:
            files, info = observed({"version": 3, "checkpoint": current})
            with self.assertRaises(ValueError):
                checkpoint(files, info, "post-destroy", ARN, SECRETS)

    def test_should_reject_duplicate_json_keys_and_excessive_depth(self):
        for raw in (b'{"x":1,"x":2}', b"[" * 66 + b"0" + b"]" * 66):
            with self.assertRaises(ValueError):
                document(raw, SECRETS)


if __name__ == "__main__":
    unittest.main()
