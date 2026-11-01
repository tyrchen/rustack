"""Import-safe bounded history observations shared by ordinary SDK consumers."""


def history(sfn, arn, reverse=False, data=True, *, invoke=None):
    events, tokens = [], set()
    token = None
    while True:
        params = dict(executionArn=arn, maxResults=7, reverseOrder=reverse, includeExecutionData=data)
        if token is not None:
            params["nextToken"] = token
        page = (sfn.get_execution_history(**params) if invoke is None
                else invoke(sfn, "get_execution_history", **params))
        assert isinstance(page["events"], list) and len(page["events"]) <= 7
        events.extend(page["events"])
        assert len(events) <= 1000
        token = page.get("nextToken")
        if token is None:
            return events
        assert isinstance(token, str) and 1 <= len(token.encode()) <= 1024 and token.isascii()
        assert token not in tokens and len(tokens) < 200
        tokens.add(token)


def assert_omitted(events):
    for event in events:
        for details in ["executionStartedEventDetails", "executionSucceededEventDetails", "stateEnteredEventDetails",
                        "stateExitedEventDetails", "taskSucceededEventDetails", "taskSubmittedEventDetails",
                        "lambdaFunctionScheduledEventDetails", "lambdaFunctionSucceededEventDetails"]:
            if details in event:
                assert not {"input", "output", "inputDetails", "outputDetails"} & set(event[details])
        assert "taskScheduledEventDetails" not in event
