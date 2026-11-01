"""Importing a consumer must not parse CLI arguments or construct SDK clients."""
import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import server_container_sdk


class ServerContainerImportControls(unittest.TestCase):
    def test_should_compare_actual_image_ids_without_widening_private_alias_observation(self):
        pair = {"function": "a" * 64, "relay": "b" * 64}
        sources = {"function": "sha256:" + "c" * 64, "relay": "sha256:" + "d" * 64}
        name, network = "ServerSdkControl", "e" * 64
        views = {}
        for role, cid in pair.items():
            views["/containers/" + cid + "/json"] = {
                "Id": cid, "Image": sources[role], "State": {"Running": True},
                "Config": {"Image": "private-alias" if role == "function" else sources[role],
                           "Env": ["AWS_LAMBDA_FUNCTION_NAME=" + name]},
                "HostConfig": {"ReadonlyRootfs": True, "Privileged": False, "PublishAllPorts": False,
                               "NetworkMode": "container:" + pair["relay"] if role == "function" else network},
                "Mounts": [{"Type": "tmpfs", "Destination": "/tmp"}]}
        class Metadata:
            def get(self, path):
                return views[path]  # Non-container paths cannot escape this independent reader.
        def observe():
            return server_container_sdk.environment(Metadata(), "gateway", None, name,
                sources["function"], sources["relay"], network)
        with patch.object(server_container_sdk, "inspect_sdk_origin", return_value=pair):
            self.assertEqual(observe(), pair)
            for role, cid in pair.items():
                view = views["/containers/" + cid + "/json"]
                view["Image"] = "sha256:" + "f" * 64
                with self.assertRaises(AssertionError):
                    observe()
                view["Image"] = sources[role]

    def test_should_import_the_consumer_without_argv_or_sdk_side_effects(self):
        source = Path(__file__).with_name("server_container_sdk.py")
        specification = importlib.util.spec_from_file_location("server_container_import_control", source)
        module = importlib.util.module_from_spec(specification)
        output = io.StringIO()
        with patch.object(sys, "argv", ["import-control", "deliberately-not-cli-arguments"]), \
             patch.object(sys, "stdout", output), \
             patch("boto3.client", side_effect=AssertionError("client created during import")) as client:
            specification.loader.exec_module(module)
        self.assertTrue(callable(module.exercise))
        self.assertEqual(output.getvalue(), "")
        client.assert_not_called()


if __name__ == "__main__":
    unittest.main()
