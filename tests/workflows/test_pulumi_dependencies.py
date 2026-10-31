"""No-network installation-root lock and private-cache controls."""

import fcntl
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import pulumi_dependencies as fixture


class DependencyOwnershipControls(unittest.TestCase):
    def test_should_serialize_one_installation_with_distinct_cache_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            entered, released, blocked, ready = (threading.Event() for _ in range(4))
            calls, errors = [], []
            original_flock = fcntl.flock

            def observed_lock(fd, operation):
                try:
                    return original_flock(fd, operation)
                except BlockingIOError:
                    blocked.set()
                    raise

            def install(*args, **kwargs):
                calls.append((args, kwargs))
                (source / "node_modules").mkdir()
                entered.set()
                if not released.wait(10):
                    raise TimeoutError("fixture install grant expired")
                ready.set()

            def run(cache):
                try:
                    fixture.prepare(cache)
                except BaseException as error:
                    errors.append(error)

            with patch.object(fixture, "SOURCE", source), patch.object(fixture, "manifest", return_value="fixed-lock"), \
                 patch.object(fixture, "installed", side_effect=lambda _: ready.is_set()), \
                 patch.object(fixture.subprocess, "run", side_effect=install), \
                 patch.object(fixture.fcntl, "flock", side_effect=observed_lock):
                first = threading.Thread(target=run, args=(root / "cache-a",))
                second = threading.Thread(target=run, args=(root / "cache-b",))
                first.start()
                try:
                    self.assertTrue(entered.wait(10), "first install did not acquire its lock")
                    second.start()
                    self.assertTrue(blocked.wait(10), "second cache did not contend on the installation lock")
                    self.assertEqual(len(calls), 1)
                finally:
                    released.set()
                    first.join(10)
                    if second.ident is not None:
                        second.join(10)
                self.assertFalse(first.is_alive())
                self.assertFalse(second.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(len(calls), 1)
            self.assertEqual((source / "node_modules" / ".rustack-iac-lock").read_text(), "fixed-lock")
            for cache in ("cache-a", "cache-b"):
                for kind in ("user", "global"):
                    self.assertEqual((root / cache / "config" / kind).read_bytes(), b"")

    def test_should_reject_nonempty_configuration_even_when_installation_is_ready(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            cache = root / "cache"
            (cache / "config").mkdir(parents=True)
            (cache / "config" / "user").write_text("registry=https://untrusted.invalid\n")
            with patch.object(fixture, "SOURCE", source), patch.object(fixture, "manifest", return_value="fixed-lock"), \
                 patch.object(fixture, "installed", return_value=True), patch.object(fixture.subprocess, "run") as execute:
                with self.assertRaises(AssertionError):
                    fixture.prepare(cache)
                execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
