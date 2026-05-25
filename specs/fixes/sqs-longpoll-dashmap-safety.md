# SQS long-poll and queue registry safety fix

## Context

PR #20 fixed lost wakeups for SQS long polling by removing the `Notify`
side-channel and calling `fulfill_pending_long_polls()` directly. The merged
code improves long-poll responsiveness, but it leaves two safety issues:

1. `RustackSqs::get_queue()` still returns a `DashMap::Ref`. Provider methods
   hold that guard while awaiting queue actor replies. Concurrent queue delete,
   create, and message operations can therefore block each other on a DashMap
   shard and produce a service-level deadlock.
2. The actor calls `fulfill_pending_long_polls()` after every command. That
   makes cheap commands scan the full pending long-poll list even when the
   command cannot make messages visible, creating CPU amplification when many
   clients hold long polls on an empty queue.

## Goals

- Release all DashMap guards before any `.await` in SQS provider paths.
- Preserve existing queue delete and actor shutdown semantics.
- Keep PR #20's correctness improvement: pending long polls wake when messages
  become immediately visible.
- Avoid scanning pending long polls after commands that cannot make messages
  visible.
- Add focused unit coverage for the actor wakeup decision so the behavior does
  not depend only on ignored integration tests.

## Design

### Queue registry ownership

Store `Arc<QueueHandle>` in the provider registry:

```rust
DashMap<String, Arc<QueueHandle>>
```

`get_queue()` returns a cloned `Arc<QueueHandle>`. The DashMap guard is dropped
before the caller awaits actor communication. This avoids cloning the
`JoinHandle` inside `QueueHandle`, which is not cloneable and does not need to
be owned by each request.

For registry iteration that awaits actor calls, first collect cloned handles
into a `Vec<Arc<QueueHandle>>`, then await outside the DashMap iterator guard.

### Actor wakeup decisions

Make `QueueActor::handle_command()` return `true` only when a command may have
made messages visible to pending long polls:

- immediate successful standard `SendMessage`
- successful FIFO `SendMessage` that enqueued a new message
- FIFO `DeleteMessage`, because it can unblock the next message in the group
- `ChangeMessageVisibility` with timeout `0`, because it immediately returns
  the message to the queue

Return `false` for receive registration, read-only commands, tag/attribute
metadata changes, purge, failed sends, delayed sends, FIFO dedup hits, and
nonzero visibility changes.

`periodic_cleanup()` remains responsible for delayed message promotion and
expired visibility timeouts.

## Verification

Run:

```sh
cargo fmt --check
cargo test -p rustack-sqs-core --lib
cargo check -p rustack-sqs-core -p rustack-integration --tests
cargo clippy -p rustack-sqs-core -p rustack-integration --tests -- -D warnings
```

After opening the PR, verify all GitHub Actions checks pass.
