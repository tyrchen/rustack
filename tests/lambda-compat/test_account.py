"""Tests for Lambda account settings."""


class TestGetAccountSettings:
    def test_should_return_account_settings(self, lamb):
        resp = lamb.get_account_settings()
        assert "AccountLimit" in resp
        assert "AccountUsage" in resp
        limit = resp["AccountLimit"]
        assert limit["TotalCodeSize"] > 0
        assert limit["ConcurrentExecutions"] > 0
        assert limit["CodeSizeZipped"] > 0
        assert limit["CodeSizeUnzipped"] > 0

    def test_should_report_function_count(self, function_factory, lamb):
        function_factory("acct-count")
        resp = lamb.get_account_settings()
        assert resp["AccountUsage"]["FunctionCount"] >= 1
