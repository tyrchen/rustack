"""Shared history observations preserve pagination, action tracking and omission."""
import unittest

from stepfunctions_history import assert_omitted, history


class Client:
    def __init__(self, pages):
        self.pages = iter(pages)
        self.calls = []

    def get_execution_history(self, **params):
        self.calls.append(params)
        return next(self.pages)


class HistoryControls(unittest.TestCase):
    def test_should_preserve_the_snapshot_cursor_and_query_flags(self):
        client = Client([{"events": [{"id": 3}], "nextToken": "cursor"}, {"events": [{"id": 2}]}])
        self.assertEqual(history(client, "execution", reverse=True, data=False), [{"id": 3}, {"id": 2}])
        self.assertEqual(client.calls, [dict(executionArn="execution", maxResults=7, reverseOrder=True,
                                           includeExecutionData=False),
            dict(executionArn="execution", maxResults=7, reverseOrder=True,
                 includeExecutionData=False, nextToken="cursor")])

    def test_should_keep_the_existing_cli_action_tracking_port(self):
        client = Client([{"events": []}])
        actions = []
        def invoke(selected, method, **params):
            actions.append(method)
            return selected.get_execution_history(**params)
        self.assertEqual(history(client, "execution", invoke=invoke), [])
        self.assertEqual(actions, ["get_execution_history"])

    def test_should_reject_duplicate_oversized_or_unbounded_page_metadata(self):
        for pages in ([{"events": [], "nextToken": "same"}] * 2,
                      [{"events": [{}] * 8}], [{"events": [], "nextToken": "x" * 1025}],
                      [{"events": [], "nextToken": True}],
                      [{"events": [], "nextToken": str(index)} for index in range(201)]):
            with self.assertRaises(AssertionError):
                history(Client(pages), "execution")

    def test_should_require_data_omission_in_every_supported_history_projection(self):
        assert_omitted([{"type": "ExecutionSucceeded", "executionSucceededEventDetails": {}}])
        for event in ({"executionSucceededEventDetails": {"output": "private"}},
                      {"taskSubmittedEventDetails": {"outputDetails": {}}},
                      {"taskScheduledEventDetails": {}}):
            with self.assertRaises(AssertionError):
                assert_omitted([event])


if __name__ == "__main__":
    unittest.main()
