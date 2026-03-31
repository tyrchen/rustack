# SQS Integration Test Suites Research

**Date:** 2026-03-02
**Purpose:** Survey of available SQS compatibility/conformance test suites for validating an SQS-compatible server implementation, analogous to how MinIO Mint tests S3 compatibility and ScyllaDB Alternator tests DynamoDB compatibility.

---

## Table of Contents

1. [Key Finding: No "Mint for SQS" Exists](#1-key-finding-no-mint-for-sqs-exists)
2. [ElasticMQ Test Suite (SoftwareMill)](#2-elasticmq-test-suite-softwaremill)
3. [LocalStack SQS Tests](#3-localstack-sqs-tests)
4. [Moto SQS Tests](#4-moto-sqs-tests)
5. [GoAWS Test Suite (Admiral-Piett)](#5-goaws-test-suite-admiral-piett)
6. [AWS SDK Integration Tests](#6-aws-sdk-integration-tests)
7. [Lyft fake_sqs Acceptance Tests](#7-lyft-fake_sqs-acceptance-tests)
8. [SQSLite (JavaScript)](#8-sqslite-javascript)
9. [SmoothMQ Tester](#9-smoothmq-tester)
10. [NerveMQ (Rust)](#10-nervemq-rust)
11. [Comparison Matrix](#11-comparison-matrix)
12. [Protocol Considerations (JSON vs Query)](#12-protocol-considerations-json-vs-query)
13. [Recommendations for Rustack SQS](#13-recommendations-for-rustack-sqs)

---

## 1. Key Finding: No "Mint for SQS" Exists

Unlike S3, where MinIO Mint provides a self-contained, Docker-based, multi-SDK conformance test suite, **no equivalent exists for SQS**. The SQS ecosystem lacks:

- A single, community-standard conformance test runner
- A Docker-packaged, point-and-shoot test suite
- A multi-SDK compatibility test framework

This gap exists because:
- SQS's API surface is relatively small (23 operations) compared to S3's 100+ operations
- Most SQS testing in practice uses mocking libraries (Moto) or local emulators (ElasticMQ, LocalStack) rather than conformance suites
- Fewer open-source SQS-compatible implementations exist compared to S3
- SQS behavior is harder to test deterministically (timing-dependent visibility timeouts, long polling, message ordering)

**The closest equivalent to "Mint for SQS" is the ElasticMQ test suite**, which is the most comprehensive, publicly available, and actively maintained SQS API conformance test suite. However, it is tightly coupled to ElasticMQ's embedded server setup and uses both AWS SDK v1 and v2 for Java/Scala.

---

## 2. ElasticMQ Test Suite (SoftwareMill)

**Repository:** https://github.com/softwaremill/elasticmq
**Test path:** `rest/rest-sqs-testing-amazon-java-sdk/src/test/scala/org/elasticmq/rest/sqs/integration/`
**Language:** Scala (ScalaTest) using AWS SDK for Java v1 and v2
**License:** Apache-2.0
**Stars:** 2,816
**Latest release:** v1.6.16 (2026-02-16)
**Last updated:** 2026-03-02 (actively maintained)

### Overview

ElasticMQ is the most popular open-source SQS-compatible server (2.8k stars). Its test suite is specifically designed to validate SQS API compatibility using the official AWS Java SDK. Tests run against both AWS SDK v1 and v2 through an abstracted `SqsClient` trait.

### Architecture

The test suite uses a layered abstraction:

1. **`SqsClient` trait** -- Abstract interface for SQS operations (create queue, send/receive message, etc.)
2. **`AwsSdkV1SqsClient` / `AwsSdkV2SqsClient`** -- Concrete implementations using AWS Java SDK v1 and v2
3. **`SQSRestServerWithSdkV1Client` / SQSRestServerWithSdkV2Client`** -- Test fixtures that start an embedded ElasticMQ server and wire up the SDK client
4. **`IntegrationTestsBase`** -- Common test utilities and assertions
5. **Test trait mixins** -- Individual test modules mixed into the main test suite

### Test Organization

```
AmazonJavaMultiSdkTestSuite (abstract)
  with QueueOperationsTests        (9 tests)
  with QueueAttributesTests        (8 tests)
  with MessageOperationsTests      (16 tests)
  with FifoQueueTests              (6 tests)
  with DeadLetterQueueTests        (3 tests)
  with MessageMoveTaskTests        (9 tests)
  with CreateQueueRaceConditionTests (2 tests)
  with FifoDeduplicationTests      (3 tests)
  with MessageAttributesTests      (5 tests)
  with TracingTests                (5 tests)
  with HealthCheckTests            (1 test)
  ------------------------------------------------
  TOTAL: 67 tests x 2 SDK versions = 134 test executions
```

Concrete test classes:
- `AmazonJavaSdkV1TestSuite` -- Runs all tests with AWS SDK v1
- `AmazonJavaSdkV2TestSuite` -- Runs all tests with AWS SDK v2

### SQS Operations Covered

| Operation | Covered | Test File |
|-----------|---------|-----------|
| CreateQueue | Yes | QueueOperationsTests, FifoQueueTests |
| DeleteQueue | Yes | QueueOperationsTests |
| GetQueueUrl | Yes | QueueOperationsTests |
| ListQueues | Yes | QueueOperationsTests |
| PurgeQueue | Yes | QueueOperationsTests |
| GetQueueAttributes | Yes | QueueAttributesTests |
| SetQueueAttributes | Yes | QueueAttributesTests |
| TagQueue | Yes | QueueOperationsTests |
| UntagQueue | Yes | QueueOperationsTests |
| ListQueueTags | Yes | QueueOperationsTests |
| AddPermission | Yes | QueueOperationsTests |
| RemovePermission | Yes | QueueOperationsTests |
| SendMessage | Yes | MessageOperationsTests |
| ReceiveMessage | Yes | MessageOperationsTests |
| DeleteMessage | Yes | MessageOperationsTests |
| ChangeMessageVisibility | Yes | MessageOperationsTests |
| SendMessageBatch | Yes | MessageOperationsTests |
| DeleteMessageBatch | Yes | MessageOperationsTests |
| ChangeMessageVisibilityBatch | Yes | MessageOperationsTests |
| StartMessageMoveTask | Yes | MessageMoveTaskTests |
| ListMessageMoveTasks | Yes | MessageMoveTaskTests |
| CancelMessageMoveTask | Yes | MessageMoveTaskTests |
| ListDeadLetterSourceQueues | Yes | DeadLetterQueueTests |

**Coverage: 23/23 SQS operations** (complete API surface)

### Feature Coverage

- Standard queues: send, receive, delete, visibility timeout, delay
- FIFO queues: message group ID, deduplication ID, content-based deduplication
- Dead letter queues: redrive policy, source queue listing
- Message move tasks: start, list, cancel
- Message attributes: string, binary, number (with custom types)
- System attributes: AWSTraceHeader
- Queue attributes: visibility timeout, delay seconds, max message size, retention period
- Batch operations: send, delete, change visibility
- Error handling: invalid parameters, missing parameters, queue not found
- Race conditions: concurrent queue creation
- 1MB message size boundary testing

### Ease of Pointing at Arbitrary Endpoint

**Difficulty: Medium-High**

The test suite is tightly coupled to ElasticMQ's embedded server:
- `SQSRestServerWithSdkV2Client` starts an `SQSRestServer` in `before` hooks
- The endpoint is hardcoded to `http://localhost:9321`
- The SQS client is configured with `endpointOverride(new URI(serviceEndpoint))`

To point at an arbitrary endpoint, you would need to:
1. Extract the test traits (QueueOperationsTests, MessageOperationsTests, etc.) from the ElasticMQ build
2. Create a new test base that configures the AWS SDK client to point at your endpoint instead of starting an embedded ElasticMQ server
3. Handle differences in account ID and region configuration

Alternatively, you could fork the test module and modify `SQSRestServerWithSdkV2Client` to skip server startup and use an external endpoint URL.

### Pros
- Most comprehensive SQS test suite available (67 unique tests, full API coverage)
- Tests both AWS SDK v1 and v2
- Clean abstraction through `SqsClient` trait makes it theoretically portable
- Well-structured test organization by feature area
- Actively maintained (2.8k stars, regular releases)
- Tests error conditions and edge cases, not just happy paths
- Apache-2.0 license

### Cons
- Scala/JVM ecosystem -- requires JDK + sbt to build and run
- Tightly coupled to ElasticMQ's embedded server setup
- Not designed as a standalone conformance suite
- Would require significant effort to decouple from ElasticMQ
- ElasticMQ-specific features tested (relaxed limits mode, custom context paths)

---

## 3. LocalStack SQS Tests

**Repository:** https://github.com/localstack/localstack
**Test path:** `tests/aws/services/sqs/`
**Language:** Python (pytest + boto3)
**License:** Apache-2.0
**Stars:** 64,500+
**Last updated:** 2026-03-02 (actively maintained)

### Overview

LocalStack contains extensive SQS integration tests in its `tests/aws/services/sqs/` directory. The tests are written using pytest + boto3 and include snapshot-based validation against real AWS responses.

### Test Files

| File | Description |
|------|-------------|
| `test_sqs.py` | Main SQS test suite (~130+ test methods, ~3,000 lines) |
| `test_sqs.snapshot.json` | Snapshot data for response validation |
| `test_sqs.validation.json` | Validation data for AWS parity checking |
| `test_sqs_developer_api.py` | LocalStack-specific developer API tests |
| `test_sqs_move_task.py` | Message move task tests |
| `test_sqs_move_task.snapshot.json` | Snapshot data for move task tests |
| `test_sqs_move_task.validation.json` | Move task validation data |
| `utils.py` | Test utilities |

### Test Count

Approximately **130+ test methods** in the main `TestSqsProvider` class, plus additional tests in `test_sqs_move_task.py` and `test_sqs_developer_api.py`.

### SQS Operations Covered

- Queue management: create, list, delete, get URL/attributes, set attributes
- Message operations: send, receive, delete, batch operations
- Visibility timeout: receive, change, extend, terminate
- FIFO queues: message groups, deduplication, ordering
- Dead letter queues: redrive policy
- Message attributes and system attributes
- Queue tagging
- Long polling (wait time seconds)
- Message retention
- Error handling and edge cases

### Test Markers/Decorators

| Marker | Purpose |
|--------|---------|
| `@markers.aws.validated` | Tests validated against real AWS |
| `@markers.aws.only_localstack` | LocalStack-specific tests |
| `@markers.requires_in_process` | Requires in-process execution |
| `@markers.aws.needs_fixing` | Known issues requiring fixes |
| `@markers.snapshot.*` | Snapshot testing with verification |
| `@pytest.mark.parametrize` | Parameterized test cases |

### Ease of Pointing at Arbitrary Endpoint

**Difficulty: High**

- Tests are tightly coupled to LocalStack's infrastructure (fixtures, markers, snapshot system)
- Requires LocalStack-specific test fixtures (`sqs_queue`, `sqs_create_queue`, `aws_sqs_client`)
- Snapshot validation depends on LocalStack's test framework
- The `@markers.aws.validated` decorator has specific semantics in LocalStack's CI
- Would require extracting the core boto3 test logic and removing LocalStack dependencies

### Pros
- Large test suite (~130+ tests)
- Python/pytest + boto3 -- the dominant pattern for AWS testing
- Many tests validated against real AWS (`@markers.aws.validated`)
- Comprehensive error condition testing
- Snapshot-based response validation
- Apache-2.0 license

### Cons
- Tightly coupled to LocalStack's test infrastructure
- Cannot be run as-is against an arbitrary SQS endpoint
- Depends on LocalStack-specific fixtures and markers
- Some tests are LocalStack-specific (`@markers.aws.only_localstack`)
- Extracting the pure boto3 logic would be a significant effort

---

## 4. Moto SQS Tests

**Repository:** https://github.com/getmoto/moto
**Test path:** `tests/test_sqs/`
**Language:** Python (pytest + boto3)
**License:** Apache-2.0
**Stars:** 8,240
**Latest release:** 5.1.21 (2026-02-08)
**Last updated:** 2026-03-02 (actively maintained)

### Overview

Moto is a Python library for mocking AWS services. Its SQS test suite validates the mock implementation's correctness against expected SQS behavior.

### Test Files and Counts

| File | Test Count | Description |
|------|-----------|-------------|
| `test_sqs.py` | 127 | Main SQS tests |
| `test_sqs_cloudformation.py` | 7 | CloudFormation integration |
| `test_server.py` | 4 | Server mode tests |
| `test_sqs_integration.py` | 3 | Integration tests |
| `test_sqs_message_attributes.py` | 3 | Message attribute tests |
| `test_sqs_authentication.py` | 1 | Authentication tests |
| `test_sqs_multiaccount.py` | 1 | Multi-account tests |
| **TOTAL** | **146** | |

### SQS Operations Covered (from test_sqs.py)

Comprehensive coverage including:
- Queue CRUD (create, delete, get URL, list with prefix)
- Queue attributes (get, set, KMS encryption, policy)
- Message send/receive (with/without attributes, unicode, XML characters, delays)
- Message size validation (1KB-256KB boundary testing, max message size attribute)
- Visibility timeout (change, batch change, inflight messages)
- Batch operations (send, delete, change visibility)
- FIFO queues (creation, deduplication, message groups, ordering, throughput limits)
- Dead letter queues (redrive policy, source queue listing)
- Queue permissions (add, remove, errors)
- Queue tags (tag, untag, list, errors)
- Queue purging
- Message retention period
- Long polling (wait time seconds)
- Error handling (invalid parameters, non-existent queues)
- Multi-region support

### Moto SQS Implementation Status

**Implemented (20/23 operations):**
- add_permission, change_message_visibility, change_message_visibility_batch
- create_queue, delete_message, delete_message_batch, delete_queue
- get_queue_attributes, get_queue_url
- list_dead_letter_source_queues, list_queue_tags, list_queues
- purge_queue, receive_message, remove_permission
- send_message, send_message_batch, set_queue_attributes
- tag_queue, untag_queue

**Not implemented (3/23):** cancel_message_move_task, list_message_move_tasks, start_message_move_task

### Ease of Pointing at Arbitrary Endpoint

**Difficulty: Very High (Not Designed for This)**

Moto tests use the `@mock_aws` decorator which intercepts all boto3 calls before they leave the process. Tests never make actual HTTP requests. The tests **cannot be run against an external SQS endpoint** without completely rewriting them to remove the mock decorators.

However, Moto has a "server mode" (`moto_server`) that exposes an HTTP endpoint, and 4 tests in `test_server.py` validate this. These server-mode tests could theoretically be pointed at any SQS-compatible endpoint, but they cover very few operations.

### Pros
- 146 tests covering most SQS operations
- Well-organized by feature area
- Good edge case and error condition coverage
- Python/pytest + boto3 ecosystem
- Apache-2.0 license
- Very actively maintained

### Cons
- **Cannot run against external endpoints** -- uses mock decorator pattern
- Tests validate Moto's internal mock, not wire protocol compatibility
- Would need complete rewrite to remove `@mock_aws` decorators
- No message move task coverage
- Tests may not match real AWS behavior exactly (Moto is a mock, not a validated implementation)

---

## 5. GoAWS Test Suite (Admiral-Piett)

**Repository:** https://github.com/Admiral-Piett/goaws
**Language:** Go (standard testing + testify)
**License:** MIT
**Stars:** 830
**Latest release:** v0.5.4 (2025-12-19)
**Last updated:** 2026-02-24

### Overview

GoAWS is a Go-based SQS/SNS clone for local development testing. It has two test layers: unit tests for internal logic and smoke tests that exercise the full HTTP stack.

### Test Organization

**SQS Smoke Tests (59 tests)** -- Integration tests using AWS SDK for Go v2:

| File | Tests | Operations |
|------|-------|-----------|
| `sqs_create_queue_test.go` | 6 | CreateQueue |
| `sqs_delete_queue_test.go` | 2 | DeleteQueue |
| `sqs_get_queue_url_test.go` | 4 | GetQueueUrl |
| `sqs_get_queue_attributes_test.go` | 6 | GetQueueAttributes |
| `sqs_set_queue_attributes_test.go` | 4 | SetQueueAttributes |
| `sqs_list_queues_test.go` | 6 | ListQueues |
| `sqs_send_message_test.go` | 6 | SendMessage |
| `sqs_send_message_batch_test.go` | 6 | SendMessageBatch |
| `sqs_receive_message_test.go` | 5 | ReceiveMessage |
| `sqs_delete_message_test.go` | 2 | DeleteMessage |
| `sqs_delete_message_batch_test.go` | 8 | DeleteMessageBatch |
| `sqs_purge_queue_test.go` | 2 | PurgeQueue |
| `sqs_change_message_visibility_test.go` | 2 | ChangeMessageVisibility |

**SQS Unit Tests (85 tests)** -- Internal logic tests:

| File | Tests |
|------|-------|
| `create_queue_test.go` | 7 |
| `delete_queue_test.go` | 3 |
| `get_queue_url_test.go` | 3 |
| `get_queue_attributes_test.go` | 7 |
| `set_queue_attributes_test.go` | 6 |
| `list_queues_test.go` | 5 |
| `send_message_test.go` | 6 |
| `send_message_batch_test.go` | 7 |
| `receive_message_test.go` | 7 |
| `delete_message_test.go` | 1 |
| `delete_message_batch_test.go` | 7 |
| `purge_queue_test.go` | 4 |
| `change_message_visibility_test.go` | 7 |
| `queue_attributes_test.go` | 5 |
| `gosqs_test.go` | 10 |

**Total: 144 SQS tests** (59 smoke + 85 unit)

### SQS Operations Covered

| Operation | Smoke Tests | Unit Tests |
|-----------|:-----------:|:----------:|
| CreateQueue | Yes | Yes |
| DeleteQueue | Yes | Yes |
| GetQueueUrl | Yes | Yes |
| GetQueueAttributes | Yes | Yes |
| SetQueueAttributes | Yes | Yes |
| ListQueues | Yes | Yes |
| SendMessage | Yes | Yes |
| SendMessageBatch | Yes | Yes |
| ReceiveMessage | Yes | Yes |
| DeleteMessage | Yes | Yes |
| DeleteMessageBatch | Yes | Yes |
| PurgeQueue | Yes | Yes |
| ChangeMessageVisibility | Yes | Yes |
| TagQueue | No | No |
| UntagQueue | No | No |
| ListQueueTags | No | No |
| AddPermission | No | No |
| RemovePermission | No | No |
| ListDeadLetterSourceQueues | No | No |
| StartMessageMoveTask | No | No |
| ListMessageMoveTasks | No | No |
| CancelMessageMoveTask | No | No |
| ChangeMessageVisibilityBatch | No | No |

**Coverage: 13/23 SQS operations**

### Ease of Pointing at Arbitrary Endpoint

**Difficulty: Medium**

The smoke tests start an internal GoAWS server, but the test infrastructure uses the AWS SDK for Go v2 with an endpoint override. To point at an arbitrary endpoint:
1. Modify `smoke_tests/fixtures/fixtures.go` to skip starting the internal server
2. Configure the endpoint URL to point at your server
3. Run `go test ./smoke_tests/...`

The smoke tests are the most decoupled from the GoAWS implementation and would be easiest to adapt.

### Pros
- Clean smoke test architecture using AWS SDK for Go v2
- Go -- easy to build and run cross-platform
- MIT license
- Reasonably well-maintained
- Good test isolation (each test creates/cleans up its own queues)

### Cons
- Only covers 13/23 SQS operations (no tags, permissions, DLQ, batched visibility, move tasks)
- No FIFO queue tests in smoke tests
- Tightly coupled to GoAWS's server startup
- Would need forking to use independently

---

## 6. AWS SDK Integration Tests

### AWS SDK for Java v2

**Repository:** https://github.com/aws/aws-sdk-java-v2
**Test path:** `services/sqs/src/it/java/software/amazon/awssdk/services/sqs/`
**Language:** Java (JUnit)

The AWS SDK for Java v2 includes minimal SQS integration tests designed to run against real AWS:
- `IntegrationTestBase.java` -- Base class with credential loading and test utilities
- `SqsIntegrationTest.java` -- 1 test (clock skew correction)
- `MessageAttributesIntegrationTest.java` -- Message attribute tests
- `RequestBatchManagerSqsIntegrationTest.java` -- Batch manager tests
- `SqsPolicyIntegrationTest.java` -- Queue policy tests
- `SqsConcurrentPerformanceIntegrationTest.java` -- Performance tests

**Smoke test feature file** (`sqs.feature`):
```gherkin
Scenario: Making a request
  When I call the "ListQueues" API
  Then the value at "QueueUrls" should be a list

Scenario: Handling errors
  When I attempt to call the "GetQueueUrl" API with:
    | QueueName | fake_queue |
  Then I expect the response error code to be
    "AWS.SimpleQueueService.NonExistentQueue"
```

**Verdict:** Too minimal for conformance testing. Only 2 smoke test scenarios. The integration tests require AWS credentials and test against real AWS.

### AWS SDK for Rust

**Repository:** https://github.com/awslabs/aws-sdk-rust
**No dedicated SQS integration tests found.** The Rust SDK relies on Smithy protocol tests for correctness.

### Rusoto (Legacy Rust AWS SDK)

**Repository:** https://github.com/rusoto/rusoto
**Test path:** `integration_tests/tests/sqs.rs`
**Test count:** 4 async test functions

Covers basic operations: list queues, create queue, send/receive message, delete. Too minimal for conformance testing. Rusoto is also deprecated in favor of the official AWS SDK for Rust.

### AWS Doc SDK Examples

**Repository:** https://github.com/awsdocs/aws-doc-sdk-examples
**SQS examples exist in:** Python, Ruby, .NET, Rust, PHP, Kotlin, C++, Java

These are documentation examples with basic tests, not conformance suites. The Rust SQS example only includes a single "hello world" binary.

**Verdict:** AWS does not publish a dedicated SQS conformance test suite.

---

## 7. Lyft fake_sqs Acceptance Tests

**Repository:** https://github.com/lyft/fake_sqs (also: https://github.com/iain/fake_sqs)
**Language:** Ruby (RSpec)
**License:** MIT
**Stars:** 16 (lyft fork)
**Last updated:** 2024-03-27 (essentially unmaintained)

### Overview

A Ruby implementation of a local SQS service with acceptance tests using the AWS Ruby SDK v1.

### Test Files

- `spec/acceptance/message_actions_spec.rb` -- SendMessage, ReceiveMessage, DeleteMessage, DeleteMessageBatch, SendMessageBatch, visibility timeout
- `spec/acceptance/queue_actions_spec.rb` -- CreateQueue, GetQueueUrl, ListQueues, DeleteQueue, SetQueueAttributes, GetQueueAttributes
- `spec/unit/` -- 11 unit test files for internal components

### Approximate Test Count

~15-20 acceptance test cases covering basic queue and message operations.

### Ease of Pointing at Arbitrary Endpoint

**Difficulty: Medium**

The acceptance tests use `AWS::SQS.new` with a configured endpoint. You could modify the spec_helper to point at a different endpoint.

### Verdict

Too few tests, unmaintained, uses deprecated AWS Ruby SDK v1. Not recommended.

---

## 8. SQSLite (JavaScript)

**Repository:** https://github.com/jennyEckstein/sqslite
**Language:** JavaScript
**License:** ISC
**Stars:** 76
**Last updated:** 2026-02-20

### Overview

SQSLite is a lightweight JavaScript implementation of Amazon SQS aimed at being as close to live SQS as possible. It can be used as a CLI application or npm module.

### Tests

The repository includes tests but the exact count and coverage are unclear from the available data. The project is a decoupled alternative to LocalStack focused solely on SQS.

### Verdict

Small community (76 stars), unclear test suite quality. Not recommended as a primary conformance suite, but worth noting as a lightweight alternative for basic smoke testing.

---

## 9. SmoothMQ Tester

**Repository:** https://github.com/poundifdef/SmoothMQ
**Language:** Go
**License:** MIT
**Stars:** 2,280
**Last updated:** 2026-02-10

### Overview

SmoothMQ is a drop-in SQS replacement backed by SQLite. It includes a simple tester (`cmd/smoothmq/tester/tester.go`) that uses the AWS SDK for Go v2.

### Test Approach

The "tester" is a load/stress testing tool, not a conformance test suite. It:
- Sends messages in parallel (configurable number of senders)
- Receives messages in parallel (configurable number of receivers)
- Validates message throughput and delivery
- Supports batch operations and delay seconds

### Verdict

Not a conformance test suite -- purely a load testing tool. Not useful for API compatibility validation.

---

## 10. NerveMQ (Rust)

**Repository:** https://github.com/fortress-build/nervemq
**Language:** Rust
**License:** MIT
**Stars:** 89
**Last updated:** 2026-02-14

### Overview

NerveMQ is a portable, SQS-compatible message queue backed by SQLite, written in Rust. It is the most directly comparable project to what Rustack is building.

### Test Status

The integration tests at `tests/integration.rs` are **entirely commented out** with a `// FIXME: These all need to be rewritten due to changes during development` note. The commented-out code shows tests for namespace management and queue operations, but none are functional.

### Verdict

No usable tests. The project itself is young and has commented-out test infrastructure.

---

## 11. Comparison Matrix

| Criteria | ElasticMQ Tests | LocalStack Tests | Moto Tests | GoAWS Smoke Tests | Custom pytest+boto3 | Custom Rust SDK |
|----------|:--------------:|:----------------:|:----------:|:-----------------:|:-------------------:|:--------------:|
| **Test count** | 67 (x2 SDKs) | ~130+ | 146 | 59 | Varies | Varies |
| **Language** | Scala/JVM | Python/pytest | Python/pytest | Go | Python/pytest | Rust |
| **Run against any endpoint** | Medium effort | High effort | No (mock) | Medium effort | Yes | Yes |
| **API operation coverage** | 23/23 (100%) | ~20/23 | 20/23 | 13/23 (57%) | Varies | Varies |
| **FIFO queue coverage** | Good | Good | Good | No | Varies | Varies |
| **DLQ coverage** | Good | Good | Good | No | Varies | Varies |
| **Move task coverage** | Yes | Yes | No | No | Varies | Varies |
| **Error condition testing** | Good | Good | Good | Moderate | Varies | Varies |
| **Validated against real AWS** | No | Yes (markers) | No | No | N/A | N/A |
| **Docker support** | No | No | No | No | N/A | N/A |
| **License** | Apache-2.0 | Apache-2.0 | Apache-2.0 | MIT | N/A | N/A |
| **Active maintenance** | Very active | Very active | Very active | Active | N/A | N/A |
| **Setup complexity** | High (JVM/sbt) | High (LocalStack) | Low (pip) | Low (go test) | Low | Low |

---

## 12. Protocol Considerations (JSON vs Query)

### Background

SQS historically used the `awsQuery` protocol (XML request/response). In late 2023, AWS added `aws-json-1.0` protocol support. Newer AWS SDK versions (Java SDK v2 2.21+, boto3 1.28.82+, JS SDK v3 3.447+) default to the JSON protocol.

### Impact on Test Suites

| Test Suite | Protocol Used | Notes |
|-----------|---------------|-------|
| ElasticMQ | Both (JSON issues) | Has known issues with `x-amz-json-1.0` error serialization (Issue #903) and MD5 checksums (Issue #947) |
| LocalStack | Both (fixed) | Required multiple PRs to fix JSON protocol handling (PR #9710, PR #11726) |
| Moto | Both (internal mock) | Protocol is handled internally |
| GoAWS | Query (likely) | Uses `x-www-form-urlencoded` request handling |

### Key Implication for Rustack

Since Rustack's SQS will use `@awsJson1_0` with `@awsQueryCompatible` (matching the Smithy model), our implementation must handle both protocols. Test suites using newer AWS SDKs will send JSON requests by default. ElasticMQ's JSON protocol issues provide useful reference for what can go wrong:

- Error responses must be properly formatted for JSON protocol
- MD5 checksum calculation differs between protocols for message attributes
- Content-Type negotiation must work correctly

---

## 13. Recommendations for Rustack SQS

### Recommended Multi-Layer Testing Strategy

#### Tier 1: Rust Native Tests (Must Have -- Day 1)

**Approach:** Write integration tests using `aws-sdk-sqs` in Rust.

```rust
use aws_sdk_sqs::Client;
use aws_config::BehaviorVersion;

async fn create_test_client() -> Client {
    let config = aws_config::defaults(BehaviorVersion::latest())
        .endpoint_url("http://localhost:4566")
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(
            aws_credential_types::Credentials::new("test", "test", None, None, "test")
        )
        .load()
        .await;

    Client::new(&config)
}

#[tokio::test]
#[ignore]
async fn test_should_create_queue_send_receive_delete() {
    let client = create_test_client().await;
    // CreateQueue, SendMessage, ReceiveMessage, DeleteMessage, DeleteQueue
}
```

**Coverage target:** All 23 SQS operations, FIFO queues, DLQ, visibility timeout, batch operations, error responses.

**Rationale:** Fastest feedback loop, compile-time type safety, tests the exact SDK our users would use. The AWS SDK for Rust defaults to JSON protocol, which validates our primary protocol implementation.

#### Tier 2: pytest + boto3 Test Suite (Must Have -- Early Development)

**Approach:** Write a standalone pytest + boto3 conformance test suite from scratch, inspired by LocalStack and Moto test patterns but designed as a standalone, endpoint-configurable suite.

```python
import boto3
import pytest

@pytest.fixture
def sqs_client(endpoint_url):
    return boto3.client(
        "sqs",
        endpoint_url=endpoint_url,
        region_name="us-east-1",
        aws_access_key_id="test",
        aws_secret_access_key="test",
    )

def test_create_queue_and_get_url(sqs_client):
    response = sqs_client.create_queue(QueueName="test-queue")
    queue_url = response["QueueUrl"]
    assert "test-queue" in queue_url

    url_response = sqs_client.get_queue_url(QueueName="test-queue")
    assert url_response["QueueUrl"] == queue_url
```

**Why write our own instead of adopting an existing suite:**

1. **No existing suite is directly reusable** -- All examined suites are tightly coupled to their respective implementations
2. **ElasticMQ tests require JVM/Scala** -- Adding a full JVM build dependency for tests is heavy
3. **Moto tests cannot run against external endpoints** -- Mock decorator pattern prevents reuse
4. **LocalStack tests depend on LocalStack infrastructure** -- Too many framework dependencies
5. **pytest + boto3 is the standard** -- Aligns with the dominant pattern in the ecosystem (used by ScyllaDB Alternator, Moto, LocalStack)

**Reference material for writing tests:** Use the following as inspiration for test cases and edge cases:
- ElasticMQ test traits for the best-structured test organization
- Moto's `test_sqs.py` for the most comprehensive list of test cases (127 tests)
- LocalStack's `test_sqs.py` for AWS-validated behavior

**Estimated effort:** ~40-60 tests to cover the core SQS operations and features, taking 2-3 days if referencing existing test suites.

#### Tier 3: Multi-SDK Validation (Nice to Have -- Later)

Once the pytest suite is solid, consider adding:
- **Node.js/TypeScript** tests using `@aws-sdk/client-sqs` -- validates JavaScript SDK JSON protocol handling
- **Go** tests using `aws-sdk-go-v2` -- validates Go SDK compatibility
- **Adapt GoAWS smoke tests** -- Fork the 59 smoke tests and point at our endpoint (relatively easy, MIT license)

### What NOT to Adopt

| Option | Reason to Skip |
|--------|---------------|
| **Moto tests as-is** | Cannot run against external endpoints; mock decorator pattern |
| **LocalStack tests as-is** | Too many LocalStack-specific dependencies |
| **ElasticMQ tests as-is** | Requires full JVM/Scala/sbt toolchain |
| **fake_sqs tests** | Deprecated Ruby SDK v1, unmaintained, too few tests |
| **NerveMQ tests** | Entirely commented out, non-functional |
| **SmoothMQ tester** | Load testing tool, not conformance suite |

### CI Pipeline Design

```
PR Checks (fast feedback):
  - cargo build
  - cargo test (Tier 1 Rust integration tests with --ignored)
  - cargo clippy, cargo fmt

Merge / Nightly (thorough validation):
  - Start rustack
  - Run pytest + boto3 suite (Tier 2) -- all SQS tests
  - Upload test artifacts (pytest output, pass/fail counts)
```

### Implementation Priority

1. **Immediate:** Rust integration tests in `tests/integration/` using `aws-sdk-sqs`
2. **Week 1-2:** Bootstrap pytest + boto3 test suite (queue CRUD, message send/receive/delete)
3. **Week 2-3:** Expand pytest suite (FIFO, DLQ, batch operations, error handling)
4. **Week 3-4:** Add message move task tests, edge cases, boundary conditions
5. **Later:** Consider multi-SDK validation (Go, Node.js)

### Tracking Progress

```
SQS Test Results:
- Queue Operations:           X/Y passing
  - CreateQueue, DeleteQueue, GetQueueUrl, ListQueues, PurgeQueue
- Queue Attributes:           X/Y passing
  - GetQueueAttributes, SetQueueAttributes
- Queue Tags:                 X/Y passing
  - TagQueue, UntagQueue, ListQueueTags
- Queue Permissions:          X/Y passing
  - AddPermission, RemovePermission
- Message Operations:         X/Y passing
  - SendMessage, ReceiveMessage, DeleteMessage
  - ChangeMessageVisibility
- Batch Operations:           X/Y passing
  - SendMessageBatch, DeleteMessageBatch
  - ChangeMessageVisibilityBatch
- FIFO Queues:                X/Y passing
  - Message groups, deduplication, ordering
- Dead Letter Queues:         X/Y passing
  - Redrive policy, ListDeadLetterSourceQueues
- Message Move Tasks:         X/Y passing
  - StartMessageMoveTask, ListMessageMoveTasks, CancelMessageMoveTask
- Error Handling:             X/Y passing
  - Invalid parameters, non-existent queues, access errors
```

---

## Sources

- [ElasticMQ Repository](https://github.com/softwaremill/elasticmq) -- SQS-compatible in-memory message queue (Scala, 2.8k stars)
- [ElasticMQ AmazonJavaSdkTestSuite](https://github.com/softwaremill/elasticmq/blob/master/rest/rest-sqs-testing-amazon-java-sdk/src/test/scala/org/elasticmq/rest/sqs/AmazonJavaSdkTestSuite.scala) -- Main test suite entry point
- [ElasticMQ SqsClient Trait](https://github.com/softwaremill/elasticmq/blob/master/rest/rest-sqs-testing-amazon-java-sdk/src/test/scala/org/elasticmq/rest/sqs/integration/client/SqsClient.scala) -- Abstract SQS client interface
- [ElasticMQ JSON Protocol Issue #903](https://github.com/softwaremill/elasticmq/issues/903) -- JSON error response serialization issues
- [ElasticMQ JSON MD5 Issue #947](https://github.com/softwaremill/elasticmq/issues/947) -- MD5 checksum mismatch with JSON protocol
- [LocalStack Repository](https://github.com/localstack/localstack) -- Full local AWS cloud stack (64k stars)
- [LocalStack SQS Tests](https://github.com/localstack/localstack/tree/main/tests/aws/services/sqs) -- SQS test directory
- [LocalStack SQS Documentation](https://docs.localstack.cloud/user-guide/aws/sqs/) -- SQS service documentation
- [LocalStack JSON Protocol Support Issue #10821](https://github.com/localstack/localstack/issues/10821) -- Adding JSON protocol for SQS
- [Moto Repository](https://github.com/getmoto/moto) -- AWS service mocking library (8.2k stars)
- [Moto test_sqs.py](https://github.com/getmoto/moto/blob/master/tests/test_sqs/test_sqs.py) -- Main SQS test file (127 tests)
- [Moto SQS Implementation Status](http://docs.getmoto.org/en/latest/docs/services/sqs.html) -- 20/23 operations implemented
- [GoAWS Repository](https://github.com/Admiral-Piett/goaws) -- Go SQS/SNS clone (830 stars)
- [GoAWS Smoke Tests](https://github.com/Admiral-Piett/goaws/tree/master/smoke_tests) -- Integration tests using AWS SDK for Go v2
- [Lyft fake_sqs](https://github.com/lyft/fake_sqs) -- Ruby SQS implementation (unmaintained)
- [SQSLite](https://github.com/jennyEckstein/sqslite) -- JavaScript SQS implementation (76 stars)
- [SmoothMQ](https://github.com/poundifdef/SmoothMQ) -- SQS replacement backed by SQLite (2.3k stars)
- [NerveMQ](https://github.com/fortress-build/nervemq) -- Rust SQS-compatible queue backed by SQLite (89 stars)
- [AWS SDK Java V2 SQS Tests](https://github.com/aws/aws-sdk-java-v2/tree/master/services/sqs/src/it/java/software/amazon/awssdk/services/sqs) -- Official SDK integration tests
- [AWS SDK Java V2 SQS Smoke Tests](https://github.com/aws/aws-sdk-java-v2/blob/master/services/sqs/src/test/resources/software/amazon/awssdk/services/sqs/smoketests/sqs.feature) -- Cucumber smoke tests
- [Rusoto SQS Integration Tests](https://github.com/rusoto/rusoto/blob/master/integration_tests/tests/sqs.rs) -- Legacy Rust SDK tests
- [AWS Doc SDK Examples - SQS](https://github.com/awsdocs/aws-doc-sdk-examples) -- Multi-language SQS examples
- [Amazon SQS JSON Protocol Announcement](https://aws.amazon.com/about-aws/whats-new/2023/11/amazon-sqs-support-json-protocol/) -- JSON protocol support
- [Amazon SQS JSON Protocol FAQ](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-json-faqs.html) -- Protocol details
