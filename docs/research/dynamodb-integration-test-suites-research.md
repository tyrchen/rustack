# DynamoDB Integration Test Suites Research

**Date:** 2026-03-01
**Purpose:** Survey of available DynamoDB compatibility/conformance test suites and recommendations for adoption in Rustack, analogous to how MinIO Mint is used for S3 testing.

---

## Table of Contents

1. [Key Finding: No "Mint for DynamoDB" Exists](#1-key-finding-no-mint-for-dynamodb-exists)
2. [ScyllaDB Alternator Test Suite](#2-scylladb-alternator-test-suite)
3. [Dynalite Test Suite](#3-dynalite-test-suite)
4. [Moto DynamoDB Tests](#4-moto-dynamodb-tests)
5. [LocalStack Internal DynamoDB Tests](#5-localstack-internal-dynamodb-tests)
6. [AWS SDK Examples & DynamoDB Local](#6-aws-sdk-examples--dynamodb-local)
7. [pytest + boto3 Approach](#7-pytest--boto3-approach)
8. [Rust aws-sdk-dynamodb Approach](#8-rust-aws-sdk-dynamodb-approach)
9. [Comparison Matrix](#9-comparison-matrix)
10. [How S3 Testing Is Set Up in Rustack (Current State)](#10-how-s3-testing-is-set-up-in-rustack-current-state)
11. [Recommendations for Rustack DynamoDB](#11-recommendations-for-rustack-dynamodb)

---

## 1. Key Finding: No "Mint for DynamoDB" Exists

Unlike S3, where MinIO Mint provides a self-contained, Docker-based, multi-SDK conformance test suite, **no equivalent exists for DynamoDB**. The DynamoDB ecosystem lacks:

- A single, community-standard conformance test runner (like Mint or Ceph s3-tests)
- A Docker-packaged, point-and-shoot test suite
- A multi-SDK compatibility test framework

This gap exists because:
- DynamoDB's API is simpler to route (single POST endpoint, header-based dispatch) compared to S3's complex REST routing
- Fewer open-source DynamoDB-compatible implementations exist compared to S3
- Most DynamoDB testing is done against DynamoDB Local (AWS's official Java emulator), which discourages third-party conformance suites
- DynamoDB testing in the ecosystem is heavily fragmented across mocking libraries (moto), local emulators (DynamoDB Local, Dynalite), and per-project test suites

**The closest equivalent to "Mint for DynamoDB" is the ScyllaDB Alternator test suite**, which is the most comprehensive, publicly available, and reusable DynamoDB conformance test suite.

---

## 2. ScyllaDB Alternator Test Suite

**Repository:** https://github.com/scylladb/scylladb/tree/master/test/alternator
**License:** AGPL-3.0 (ScyllaDB), but tests themselves are standalone Python

### Overview

The ScyllaDB Alternator project includes an extensive pytest + boto3 test suite specifically designed to validate DynamoDB API compatibility. This is the **gold standard** for DynamoDB conformance testing.

### Key Stats

| Property | Value |
|----------|-------|
| Language | Python (pytest + boto3) |
| Test functions | **700+** |
| Lines of test code | **17,000+** |
| Test files | **~51** (named `test_*.py`) |
| Docker support | No (native pytest) |
| Can run against any endpoint | **Yes** (`--url` flag) |
| Can run against real AWS DynamoDB | **Yes** (`--aws` flag) |
| Active maintenance | Yes (actively developed by ScyllaDB) |

### DynamoDB Features Covered

- **Table operations**: CreateTable, DeleteTable, DescribeTable, ListTables, UpdateTable
- **Item operations**: PutItem, GetItem, UpdateItem, DeleteItem
- **Query and Scan**: Full query/scan with filters, pagination, projections
- **Batch operations**: BatchGetItem, BatchWriteItem
- **Transactions**: TransactGetItems, TransactWriteItems
- **Indexes**: GSI and LSI (creation, querying, projections)
- **Expressions**: Condition, filter, key condition, projection, and update expressions
- **TTL**: Time-to-live configuration and behavior
- **Streams**: DynamoDB Streams operations
- **Tags**: Resource tagging
- **Authorization**: Basic auth checks
- **Consumed capacity**: Tracking and validation
- **CORS**: Cross-origin request handling
- **Error handling**: Comprehensive error code and message validation

### How to Run Against Rustack

```bash
# Clone ScyllaDB repo (only need the test/alternator directory)
git clone --depth 1 --filter=blob:none --sparse https://github.com/scylladb/scylladb.git
cd scylladb
git sparse-checkout set test/alternator

# Install dependencies
cd test/alternator
pip install boto3 pytest

# Run against Rustack
pytest --url http://localhost:4566 -v

# Run specific test files
pytest --url http://localhost:4566 test_query.py -v

# Run specific test functions
pytest --url http://localhost:4566 test_put_item.py::test_put_item_basic -v
```

### conftest.py Architecture

The `conftest.py` is highly reusable:
- Creates a boto3 DynamoDB client/resource connected to the specified endpoint
- Disables client-side parameter validation (allows testing error conditions)
- Provides fixtures for pre-created tables with various key schemas
- Provides a `filled_test_table` fixture with 328 pre-populated items
- Has `scylla_only` marker to skip ScyllaDB-specific tests
- Has `dynamodb_bug` marker for known DynamoDB bugs

### Pros
- Most comprehensive DynamoDB conformance test suite available (700+ tests)
- Designed to run against any DynamoDB-compatible endpoint
- Validates behavior against real AWS DynamoDB
- Covers edge cases, error conditions, and limits
- Well-organized by feature area
- Actively maintained

### Cons
- Part of the larger ScyllaDB monorepo (need sparse checkout)
- Some tests are ScyllaDB-specific (marked with `scylla_only`)
- AGPL-3.0 license for the repository (though tests are independently useful)
- Requires Python + pip setup
- No Docker packaging

---

## 3. Dynalite Test Suite

**Repository:** https://github.com/architect/dynalite/tree/main/test
**License:** MIT

### Overview

Dynalite is a Node.js DynamoDB emulator built on LevelDB. Its test suite is designed to match DynamoDB behavior exactly, including error messages and limits.

### Key Stats

| Property | Value |
|----------|-------|
| Language | JavaScript (Mocha) |
| Test files | **20** |
| Docker support | No |
| Tested against real AWS | Yes (in multiple regions) |
| Active maintenance | Moderate (OpenJS Foundation project) |

### DynamoDB Operations Covered

- CreateTable, DeleteTable, DescribeTable, ListTables, UpdateTable
- PutItem, GetItem, DeleteItem, UpdateItem
- BatchGetItem, BatchWriteItem
- Query, Scan
- TagResource, UntagResource, ListTagsOfResource
- DescribeTimeToLive
- Connection handling

### Pros
- MIT license (very permissive)
- Tests verified against real AWS DynamoDB across multiple regions
- Thorough validation of error messages and limits
- Lightweight (no large dependencies)

### Cons
- Smaller test count (~20 files) compared to Alternator
- JavaScript/Mocha framework (different ecosystem from Rust)
- No expression language testing depth visible from file listing
- Uses DynamoDB SDK v2 (older)
- Less actively maintained than Alternator tests

---

## 4. Moto DynamoDB Tests

**Repository:** https://github.com/getmoto/moto/tree/master/tests/test_dynamodb
**License:** Apache-2.0

### Overview

Moto is a Python library for mocking AWS services. Its DynamoDB test suite validates its own mock implementation against expected DynamoDB behavior.

### Key Stats

| Property | Value |
|----------|-------|
| Language | Python (pytest + boto3) |
| Test files | **31** |
| Docker support | Yes (moto server mode) |
| Can run against external endpoint | No (tests mock internally) |
| Active maintenance | Very active |

### Coverage Areas

- Core CRUD operations
- Batch read/write
- Query and Scan
- GSI operations
- Transactional writes and reads
- Condition and update expressions
- CloudFormation integration
- IAM permissions
- Resource policies
- Import/export functionality

### Pros
- Apache-2.0 license
- Very comprehensive (31 test files)
- Actively maintained
- Covers advanced features (transactions, expressions, CloudFormation)

### Cons
- **Cannot be run against an external endpoint** -- tests are tightly coupled to moto's mock decorator pattern (`@mock_aws`)
- Designed to test moto's internal mock, not a real DynamoDB server
- Would require significant rewriting to adapt for external endpoint testing
- Not suitable as a conformance test suite without modification

---

## 5. LocalStack Internal DynamoDB Tests

**Repository:** https://github.com/localstack/localstack/tree/master/tests/aws/services/dynamodb
**License:** Apache-2.0 (community edition)

### Overview

LocalStack's DynamoDB tests validate their DynamoDB emulation (which internally wraps DynamoDB Local).

### Key Stats

| Property | Value |
|----------|-------|
| Language | Python (pytest + boto3) |
| Test files | ~4 (test_dynamodb.py + snapshot/validation files) |
| Can run against external endpoint | Partially (designed for LocalStack) |
| Test coverage | ~84% of DynamoDB listener code |

### Pros
- Tests the DynamoDB API surface we also want to implement
- Snapshot-based testing approach is interesting
- Apache-2.0 license

### Cons
- Relatively small test suite
- Tightly coupled to LocalStack's infrastructure
- Not designed as a standalone conformance suite
- Depends on LocalStack-specific test fixtures

---

## 6. AWS SDK Examples & DynamoDB Local

### AWS DynamoDB Examples (aws-samples)

**Repository:** https://github.com/aws-samples/aws-dynamodb-examples

Contains example code in multiple languages (Python, Java, Node.js, Rust, .NET, Go) demonstrating DynamoDB operations. The Rust examples cover:

- Working with Items (batch processing, conditional updates)
- Working with Queries (sorting, filtering, projections)
- Working with Indexes (secondary indexes)
- Working with Scans
- Working with Streams
- Working with Tables (creation, deletion, global tables)

These are **examples, not tests**, but could serve as a reference for writing integration tests.

### DynamoDB Local

**Docker image:** `amazon/dynamodb-local`

AWS's official Java-based DynamoDB emulator. Commonly used for local development and testing. See section 12 of `dynamodb-api-research.md` for its known limitations (no parallel scan, case-insensitive table names, missing PITR/tags support, etc.).

---

## 7. pytest + boto3 Approach

### Why This Is the Dominant Pattern

For DynamoDB conformance testing, **pytest + boto3 is overwhelmingly the most common approach** across the ecosystem:

- ScyllaDB Alternator uses it (700+ tests)
- Moto uses it (31 test files)
- LocalStack uses it
- Most community DynamoDB testing guides recommend it

### Key Libraries

| Library | Purpose |
|---------|---------|
| `boto3` | AWS SDK for Python, DynamoDB client |
| `pytest` | Test framework |
| `pytest-dynamodb` | Pytest plugin for DynamoDB fixture management |
| `moto` | AWS service mocking (for unit tests, not conformance) |

### Standard Client Setup

```python
import boto3

dynamodb = boto3.resource(
    "dynamodb",
    endpoint_url="http://localhost:4566",
    region_name="us-east-1",
    aws_access_key_id="test",
    aws_secret_access_key="test",
)
```

---

## 8. Rust aws-sdk-dynamodb Approach

### Overview

Using the official AWS SDK for Rust to write DynamoDB integration tests natively.

### Setup

```rust
use aws_sdk_dynamodb::Client;
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
```

### Pros
- Native to our Rust codebase
- Compile-time type safety
- Runs as part of `cargo test`
- Fastest feedback loop
- Tests the exact SDK our users would use

### Cons
- Only tests one SDK (Rust)
- Would need to write all tests from scratch
- Smaller community of DynamoDB Rust testing examples

---

## 9. Comparison Matrix

| Criteria | Alternator Tests | Dynalite Tests | Moto Tests | Custom pytest+boto3 | Custom Rust SDK |
|----------|:---------------:|:--------------:|:----------:|:-------------------:|:--------------:|
| **Test count** | 700+ | ~20 files | 31 files | Varies | Varies |
| **Language** | Python/pytest | JS/Mocha | Python/pytest | Python/pytest | Rust |
| **Run against any endpoint** | Yes | No (internal) | No (mock) | Yes | Yes |
| **Expression coverage** | Excellent | Moderate | Good | Varies | Varies |
| **Transaction coverage** | Yes | No | Yes | Varies | Varies |
| **Error condition testing** | Excellent | Good | Good | Varies | Varies |
| **Docker support** | No | No | Yes (server mode) | N/A | N/A |
| **License** | AGPL-3.0 | MIT | Apache-2.0 | N/A | N/A |
| **Active maintenance** | Very active | Moderate | Very active | N/A | N/A |
| **Setup complexity** | Low (pip install) | Medium (npm) | N/A | Low | Low |
| **Verified against real AWS** | Yes | Yes | No | N/A | N/A |
| **Rust ecosystem native** | No | No | No | No | Yes |

---

## 10. How S3 Testing Is Set Up in Rustack (Current State)

For context, the existing S3 testing approach uses:

### Makefile Targets
- `mint`: Orchestrates full test cycle (build, start, run)
- `mint-build`: Builds the server in release mode
- `mint-start`: Starts the server with test credentials and waits for health check
- `mint-run`: Runs the MinIO Mint Docker container against the server
- `mint-stop`: Kills the server process

### Configuration
- Server endpoint: `0.0.0.0:4566`
- Credentials: `minioadmin/minioadmin`
- Health check: `http://127.0.0.1:4566/_localstack/health`
- Container networking: macOS uses `host.containers.internal`, Linux uses `--network host`
- Results: Output to `/tmp/mint-logs/mint-output.txt` with pass/fail counting

### Key Pattern
The Mint approach works because:
1. Build the server
2. Start it as a background process
3. Wait for health check
4. Run containerized tests against it
5. Parse results
6. Stop the server

This pattern can be replicated for DynamoDB testing using the Alternator test suite.

---

## 11. Recommendations for Rustack DynamoDB

### Recommended Multi-Layer Testing Strategy

#### Tier 1: Rust Native Tests (Must Have -- Day 1)

**Approach**: Write integration tests using `aws-sdk-dynamodb` in Rust.

```rust
// tests/integration/src/dynamodb.rs
#[tokio::test]
#[ignore] // Run with: cargo test -- --ignored
async fn test_should_create_table_and_put_get_item() {
    let client = create_dynamodb_client().await;
    // CreateTable, PutItem, GetItem, DeleteTable
}
```

**Coverage target**: Core CRUD, batch operations, query/scan with expressions, error responses.

**Rationale**: Fastest feedback loop, compile-time type safety, runs as part of normal Rust development workflow.

#### Tier 2: ScyllaDB Alternator Test Suite (Must Have -- Early Development)

**Approach**: Clone the Alternator test suite and run it against our server.

**Recommended Makefile targets**:

```makefile
ALTERNATOR_TEST_DIR := vendors/alternator-tests

alternator-tests-setup:
	@if [ ! -d "$(ALTERNATOR_TEST_DIR)" ]; then \
		git clone --depth 1 --filter=blob:none --sparse \
			https://github.com/scylladb/scylladb.git $(ALTERNATOR_TEST_DIR); \
		cd $(ALTERNATOR_TEST_DIR) && git sparse-checkout set test/alternator; \
	fi
	@pip install -q boto3 pytest

dynamodb-test: dynamodb-test-start dynamodb-test-run

dynamodb-test-start: mint-build
	@echo "Starting Rustack server..."
	@ACCESS_KEY=test SECRET_KEY=test \
		DYNAMODB_SKIP_SIGNATURE_VALIDATION=false \
		GATEWAY_LISTEN=0.0.0.0:4566 \
		LOG_LEVEL=warn \
		cargo run --release -p rustack &
	@# Wait for server...

dynamodb-test-run: alternator-tests-setup
	@cd $(ALTERNATOR_TEST_DIR)/test/alternator && \
		pytest --url http://localhost:4566 -v \
		--ignore=test_scylla.py \
		--ignore=test_cql.py \
		-k "not scylla_only" \
		2>&1 | tee /tmp/dynamodb-test-output.txt || true
	@echo ""
	@# Parse results...
```

**Rationale**: 700+ tests is vastly more comprehensive than anything we could write ourselves in a reasonable time. The tests are verified against real AWS DynamoDB, cover edge cases and error conditions, and are actively maintained. This is the "Mint equivalent" for DynamoDB.

**When to add**: As soon as CreateTable + PutItem + GetItem are working. Run selective subsets first:

```bash
# Start with table operations
pytest --url http://localhost:4566 test_table.py -v

# Then item operations
pytest --url http://localhost:4566 test_item.py test_put_item.py test_get_item.py -v

# Then expressions
pytest --url http://localhost:4566 test_condition_expression.py test_update_expression.py -v

# Then query/scan
pytest --url http://localhost:4566 test_query.py test_scan.py -v
```

#### Tier 3: Dynalite Test Suite (Nice to Have -- Later)

**Approach**: Port or adapt Dynalite's JavaScript tests as a secondary validation. Lower priority because the Alternator tests already cover more ground.

**Rationale**: MIT license, different SDK perspective (JavaScript), but significantly fewer tests than Alternator.

### What NOT to Adopt

| Option | Reason to Skip |
|--------|---------------|
| **Moto tests** | Cannot run against external endpoints; tightly coupled to mock decorator pattern |
| **LocalStack internal tests** | Too few tests, tightly coupled to LocalStack infrastructure |
| **DynamoDB Local** | Not a test suite; it's a competing implementation |
| **Writing 700+ tests from scratch** | Alternator tests already exist and are verified against real AWS |

### CI Pipeline Design

```
PR Checks (fast feedback):
  - cargo build
  - cargo test (Tier 1 Rust integration tests with --ignored)
  - cargo clippy, cargo fmt

Merge / Nightly (thorough validation):
  - Start rustack
  - Run Alternator tests (Tier 2) -- selective subset initially
  - Upload test artifacts (pytest output, pass/fail counts)
```

### Implementation Priority

1. **Immediate**: Rust integration tests in `tests/integration/` using `aws-sdk-dynamodb`
2. **Week 2-3**: Alternator test suite integration (sparse checkout + Makefile targets)
3. **Ongoing**: Gradually expand which Alternator test files pass as features are implemented
4. **Later**: Consider Docker-packaging the Alternator tests for easier CI

### Tracking Progress with Alternator Tests

Create a tracking matrix similar to how Mint results are tracked for S3:

```
Alternator Test Results:
- test_table.py:              X/Y passing
- test_item.py:               X/Y passing
- test_put_item.py:           X/Y passing
- test_get_item.py:           X/Y passing
- test_update_item.py:        X/Y passing
- test_delete_item.py:        X/Y passing
- test_query.py:              X/Y passing
- test_scan.py:               X/Y passing
- test_batch.py:              X/Y passing
- test_condition_expression.py: X/Y passing
- test_update_expression.py:  X/Y passing
- test_filter_expression.py:  X/Y passing
- test_projection_expression.py: X/Y passing
- test_key_condition_expression.py: X/Y passing
- test_gsi.py:                X/Y passing
- test_lsi.py:                X/Y passing
- test_transactions.py:       X/Y passing
- test_ttl.py:                X/Y passing
- test_streams.py:            X/Y passing
- test_tag.py:                X/Y passing
```

---

## Sources

- [ScyllaDB Alternator Test Suite](https://github.com/scylladb/scylladb/tree/master/test/alternator) -- 700+ pytest+boto3 tests for DynamoDB compatibility
- [ScyllaDB Alternator README](https://github.com/scylladb/scylladb/blob/master/test/alternator/README.md) -- Test suite documentation and usage
- [ScyllaDB Alternator conftest.py](https://github.com/scylladb/scylladb/blob/master/test/alternator/conftest.py) -- Pytest fixtures and endpoint configuration
- [ScyllaDB Alternator Compatibility](https://github.com/scylladb/scylladb/blob/master/docs/alternator/compatibility.md) -- DynamoDB API compatibility matrix
- [ScyllaDB Alternator Project Overview](https://github.com/scylladb/scylladb/wiki/Alternator-Project-Overview) -- Project wiki
- [Dynalite](https://github.com/architect/dynalite) -- Node.js DynamoDB emulator with test suite
- [Moto DynamoDB Tests](https://github.com/getmoto/moto/tree/master/tests/test_dynamodb) -- Moto's DynamoDB mock test suite
- [LocalStack DynamoDB Tests](https://github.com/localstack/localstack/tree/master/tests/aws/services/dynamodb) -- LocalStack's DynamoDB tests
- [LocalStack DynamoDB Coverage](https://docs.localstack.cloud/references/coverage/coverage_dynamodb/) -- DynamoDB API coverage in LocalStack
- [AWS DynamoDB Examples (Rust)](https://github.com/aws-samples/aws-dynamodb-examples/tree/master/examples/SDK/rust) -- Official Rust SDK examples
- [aws-sdk-dynamodb crate](https://crates.io/crates/aws-sdk-dynamodb) -- Official AWS SDK for Rust DynamoDB client
- [DynamoDB Local Docker Image](https://hub.docker.com/r/amazon/dynamodb-local/) -- AWS's official DynamoDB emulator
- [Testcontainers DynamoDB Module](https://testcontainers.com/modules/dynamodb/) -- Testcontainers support for DynamoDB
- [pytest-dynamodb](https://github.com/ClearcodeHQ/pytest-dynamodb) -- Pytest plugin for DynamoDB fixture management
