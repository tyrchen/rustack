"""A stale computation, environment or public host origin cannot qualify."""
import unittest

from server_container_oracle import failure_origin, internal_origin, result, warm


class ServerContainerOracleControls(unittest.TestCase):
    def test_should_project_only_bounded_allowlisted_source_coordinates(self):
        try:
            result({}, 50, 3, "arm64")
        except AssertionError as error:
            self.assertRegex(failure_origin(error), r"^server_container_oracle\.py:[1-9][0-9]{0,3}$")
        try:
            exec(compile("raise ValueError('local-secret')", "credential-bearing-path.py", "exec"))
        except ValueError as error:
            self.assertEqual(failure_origin(error), "unknown")
        self.assertEqual(failure_origin(ValueError("local-secret")), "unknown")

    def output(self):
        return {"computed": 50, "counter": 3, "versions": {"s3": "3.1105.0", "dynamodb": "3.1105.0"},
                "selectorsCleared": True, "nodeVersion": "24.22.1", "architecture": "arm64"}

    def test_should_require_the_literal_fresh_computation_counter_and_sdk_cohort(self):
        result(self.output(), 50, 3, "arm64")
        for changes in ({"computed": 46}, {"counter": 2}, {"computed": True}, {"counter": True},
                        {"nodeVersion": "24.0.0"}, {"selectorsCleared": False},
                        {"architecture": "x64"}, {"versions": {"s3": "3.1105.0"}}, {"extra": 1}):
            with self.subTest(changes=changes), self.assertRaises(AssertionError):
                result(dict(self.output(), **changes), 50, 3, "arm64")

    def test_should_reject_the_host_published_origin_as_the_internal_gateway(self):
        valid = "http://rustack-server-" + "a" * 32 + ":4566/"
        self.assertEqual(internal_origin(valid), valid)
        for value in ("http://127.0.0.1:4566/", valid.replace(":4566", ":49153"),
                      valid.replace("http:", "https:"), valid + "path", valid.rstrip("/"),
                      valid.replace("a" * 32, "A" * 32)):
            with self.assertRaises(AssertionError):
                internal_origin(value)

    def test_should_require_the_same_distinct_complete_environment_pair(self):
        pair = {"function": "a" * 64, "relay": "b" * 64}
        self.assertEqual(warm(None, pair), pair)
        self.assertEqual(warm(pair, dict(pair)), pair)
        for current in ({"function": "c" * 64, "relay": "b" * 64},
                        {"function": "a" * 64, "relay": "a" * 64},
                        {"function": "short", "relay": "b" * 64}, dict(pair, extra="c" * 64)):
            with self.assertRaises(AssertionError):
                warm(pair, current)


if __name__ == "__main__":
    unittest.main()
