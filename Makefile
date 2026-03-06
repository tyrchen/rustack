build:
	@cargo build

check:
	@cargo check --all-targets --all-features

test:
	@cargo nextest run --all-features

fmt:
	@cargo +nightly fmt

clippy:
	@cargo clippy --all-targets --all-features -- -D warnings

audit:
	@cargo audit

deny:
	@cargo deny check

run:
	@cargo run -p ruststack-server

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin master
	@cargo release push --execute

codegen:
	@cd codegen && cargo run
	@cargo +nightly fmt -p ruststack-s3-model

integration:
	@cargo test -p ruststack-integration -- --ignored

mint: mint-start mint-run

mint-build:
	@cargo build --release -p ruststack-server

mint-start: mint-build
	@echo "Starting RustStack server..."
	@ACCESS_KEY=minioadmin SECRET_KEY=minioadmin \
		S3_SKIP_SIGNATURE_VALIDATION=false \
		DYNAMODB_SKIP_SIGNATURE_VALIDATION=false \
		GATEWAY_LISTEN=0.0.0.0:4566 \
		LOG_LEVEL=warn \
		cargo run --release -p ruststack-server &
	@for i in $$(seq 1 30); do \
		if curl -sf http://127.0.0.1:4566/_localstack/health > /dev/null 2>&1; then \
			echo "Server is ready"; \
			break; \
		fi; \
		if [ "$$i" -eq 30 ]; then \
			echo "Server did not start within 30s"; \
			exit 1; \
		fi; \
		sleep 1; \
	done

CONTAINER_CMD := $(shell command -v docker 2>/dev/null || command -v podman 2>/dev/null)
# macOS containers can't use --network host; use host.containers.internal instead.
MINT_SERVER_ENDPOINT := $(shell if [ "$$(uname)" = "Darwin" ]; then echo "host.containers.internal:4566"; else echo "127.0.0.1:4566"; fi)
MINT_NETWORK := $(shell if [ "$$(uname)" = "Darwin" ]; then echo ""; else echo "--network host"; fi)

mint-run:
	@mkdir -p /tmp/mint-logs
	$(CONTAINER_CMD) run --rm $(MINT_NETWORK) \
		-e SERVER_ENDPOINT=$(MINT_SERVER_ENDPOINT) \
		-e ACCESS_KEY=minioadmin \
		-e SECRET_KEY=minioadmin \
		-e ENABLE_HTTPS=0 \
		minio/mint:latest 2>&1 | tee /tmp/mint-logs/mint-output.txt || true
	@echo ""
	@PASS_COUNT=$$(grep -oE 'Executed [0-9]+' /tmp/mint-logs/mint-output.txt | grep -oE '[0-9]+' || echo "0"); \
		FAIL_COUNT=$$(grep -c '"status": "FAIL"' /tmp/mint-logs/mint-output.txt || true); \
		echo "Mint results: $$PASS_COUNT passed, $$FAIL_COUNT failed"

mint-stop:
	@pkill -f "ruststack-server" 2>/dev/null || true
	@echo "Server stopped"

alternator: alternator-setup alternator-run

alternator-setup:
	@bash tests/dynamodb-compat/setup.sh

ALTERNATOR_DIR := tests/dynamodb-compat/vendor
ALTERNATOR_VENV := tests/dynamodb-compat/.venv
ALTERNATOR_URL := http://localhost:4566
# P0 test files matching our implemented operations
# P0 test files matching our implemented operations.
# test_limits.py excluded: imports from test_gsi (GSI = Phase 1).
ALTERNATOR_P0_FILES := test_table.py test_item.py test_batch.py test_query.py test_scan.py \
	test_condition_expression.py test_filter_expression.py test_update_expression.py \
	test_projection_expression.py test_key_condition_expression.py test_number.py \
	test_nested.py test_describe_table.py test_returnvalues.py test_expected.py

alternator-run:
	@echo "Running Alternator DynamoDB compatibility tests..."
	@cd $(ALTERNATOR_DIR) && $(CURDIR)/$(ALTERNATOR_VENV)/bin/pytest -v --url $(ALTERNATOR_URL) \
		$(ALTERNATOR_P0_FILES) \
		-k "not scylla" \
		2>&1 | tee /tmp/alternator-output.txt || true
	@echo ""
	@PASSED=$$(grep -oP '\d+ passed' /tmp/alternator-output.txt || echo "0 passed"); \
		FAILED=$$(grep -oP '\d+ failed' /tmp/alternator-output.txt || echo "0 failed"); \
		ERRORS=$$(grep -oP '\d+ error' /tmp/alternator-output.txt || echo "0 errors"); \
		SKIPPED=$$(grep -oP '\d+ skipped' /tmp/alternator-output.txt || echo "0 skipped"); \
		echo "Alternator results: $$PASSED, $$FAILED, $$ERRORS, $$SKIPPED"

alternator-stop:
	@pkill -f "ruststack-server" 2>/dev/null || true
	@echo "Server stopped"

sqs-compat: sqs-compat-setup sqs-compat-run

sqs-compat-setup:
	@cd tests/sqs-compat && python3 -m venv .venv 2>/dev/null || true
	@tests/sqs-compat/.venv/bin/pip install -q -r tests/sqs-compat/requirements.txt

SQS_COMPAT_VENV := tests/sqs-compat/.venv
SQS_COMPAT_URL := http://localhost:4566

sqs-compat-run:
	@echo "Running SQS compatibility tests..."
	@cd tests/sqs-compat && $(CURDIR)/$(SQS_COMPAT_VENV)/bin/pytest -v --url $(SQS_COMPAT_URL) \
		2>&1 | tee /tmp/sqs-compat-output.txt || true
	@echo ""
	@PASSED=$$(grep -oP '\d+ passed' /tmp/sqs-compat-output.txt || echo "0 passed"); \
		FAILED=$$(grep -oP '\d+ failed' /tmp/sqs-compat-output.txt || echo "0 failed"); \
		ERRORS=$$(grep -oP '\d+ error' /tmp/sqs-compat-output.txt || echo "0 errors"); \
		echo "SQS compat results: $$PASSED, $$FAILED, $$ERRORS"

update-submodule:
	@git submodule update --init --recursive --remote

.PHONY: build check test fmt clippy audit deny run release update-submodule codegen integration \
	mint mint-build mint-start mint-run mint-stop \
	alternator alternator-setup alternator-run alternator-stop \
	sqs-compat sqs-compat-setup sqs-compat-run
