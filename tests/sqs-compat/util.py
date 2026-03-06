"""Shared helpers for SQS compatibility tests."""

import random
import string
import time


def unique_queue_name(prefix="test"):
    """Generate a unique queue name using timestamp and random suffix."""
    ts = int(time.time() * 1000) % 10_000_000
    suffix = "".join(random.choices(string.ascii_lowercase, k=6))
    return f"{prefix}-{ts}-{suffix}"


def random_name(length=8):
    """Generate a short random alphanumeric string."""
    return "".join(random.choices(string.ascii_lowercase + string.digits, k=length))


def receive_all(client, queue_url, max_wait=5, max_messages=10):
    """Drain all visible messages from a queue.

    Polls repeatedly until no more messages are returned.
    Returns a list of message dicts.
    """
    all_msgs = []
    deadline = time.time() + max_wait
    while time.time() < deadline:
        resp = client.receive_message(
            QueueUrl=queue_url,
            MaxNumberOfMessages=max_messages,
            WaitTimeSeconds=0,
            VisibilityTimeout=30,
        )
        msgs = resp.get("Messages", [])
        if not msgs:
            break
        all_msgs.extend(msgs)
    return all_msgs


def wait_for_messages(client, queue_url, expected, timeout=10, delete=False):
    """Poll until at least `expected` messages are received.

    Returns the collected messages.
    """
    collected = []
    deadline = time.time() + timeout
    while len(collected) < expected and time.time() < deadline:
        resp = client.receive_message(
            QueueUrl=queue_url,
            MaxNumberOfMessages=min(10, expected - len(collected)),
            WaitTimeSeconds=1,
        )
        msgs = resp.get("Messages", [])
        collected.extend(msgs)
        if delete and msgs:
            for m in msgs:
                client.delete_message(
                    QueueUrl=queue_url,
                    ReceiptHandle=m["ReceiptHandle"],
                )
    return collected


def drain_queue(client, queue_url, timeout=5):
    """Delete all messages from a queue by receiving and deleting them."""
    deadline = time.time() + timeout
    count = 0
    while time.time() < deadline:
        resp = client.receive_message(
            QueueUrl=queue_url,
            MaxNumberOfMessages=10,
            WaitTimeSeconds=0,
        )
        msgs = resp.get("Messages", [])
        if not msgs:
            break
        for m in msgs:
            client.delete_message(
                QueueUrl=queue_url,
                ReceiptHandle=m["ReceiptHandle"],
            )
            count += 1
    return count
