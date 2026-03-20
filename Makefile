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

codegen-s3:
	@cd codegen && cargo run -- --config services/s3.toml --model smithy-model/s3.json --output ../crates/ruststack-s3-model/src
	@cargo +nightly fmt -p ruststack-s3-model

codegen-ssm:
	@cd codegen && cargo run -- --config services/ssm.toml --model smithy-model/ssm.json --output ../crates/ruststack-ssm-model/src
	@cargo +nightly fmt -p ruststack-ssm-model

codegen-events:
	@cd codegen && cargo run -- --config services/events.toml --model smithy-model/events.json --output ../crates/ruststack-events-model/src
	@cargo +nightly fmt -p ruststack-events-model

codegen-dynamodb:
	@cd codegen && cargo run -- --config services/dynamodb.toml --model smithy-model/dynamodb.json --output ../crates/ruststack-dynamodb-model/src
	@cargo +nightly fmt -p ruststack-dynamodb-model

codegen-sqs:
	@cd codegen && cargo run -- --config services/sqs.toml --model smithy-model/sqs.json --output ../crates/ruststack-sqs-model/src
	@cargo +nightly fmt -p ruststack-sqs-model

codegen-sns:
	@cd codegen && cargo run -- --config services/sns.toml --model smithy-model/sns.json --output ../crates/ruststack-sns-model/src
	@cargo +nightly fmt -p ruststack-sns-model

codegen-lambda:
	@cd codegen && cargo run -- --config services/lambda.toml --model smithy-model/lambda.json --output ../crates/ruststack-lambda-model/src
	@cargo +nightly fmt -p ruststack-lambda-model

codegen-kms:
	@cd codegen && cargo run -- --config services/kms.toml --model smithy-model/kms.json --output ../crates/ruststack-kms-model/src
	@cargo +nightly fmt -p ruststack-kms-model

codegen-kinesis:
	@cd codegen && cargo run -- --config services/kinesis.toml --model smithy-model/kinesis.json --output ../crates/ruststack-kinesis-model/src
	@cargo +nightly fmt -p ruststack-kinesis-model

codegen-logs:
	@cd codegen && cargo run -- --config services/logs.toml --model smithy-model/logs.json --output ../crates/ruststack-logs-model/src
	@cargo +nightly fmt -p ruststack-logs-model

codegen-secretsmanager:
	@cd codegen && cargo run -- --config services/secretsmanager.toml --model smithy-model/secretsmanager.json --output ../crates/ruststack-secretsmanager-model/src
	@cargo +nightly fmt -p ruststack-secretsmanager-model

codegen-ses:
	@cd codegen && cargo run -- --config services/ses.toml --model smithy-model/ses.json --output ../crates/ruststack-ses-model/src
	@cargo +nightly fmt -p ruststack-ses-model

codegen-apigatewayv2:
	@cd codegen && cargo run -- --config services/apigatewayv2.toml --model smithy-model/apigatewayv2.json --output ../crates/ruststack-apigatewayv2-model/src
	@cargo +nightly fmt -p ruststack-apigatewayv2-model

codegen-cloudwatch:
	@cd codegen && cargo run -- --config services/cloudwatch.toml --model smithy-model/cloudwatch.json --output ../crates/ruststack-cloudwatch-model/src
	@cargo +nightly fmt -p ruststack-cloudwatch-model

codegen: codegen-s3

SMITHY_MODELS_REPO = https://raw.githubusercontent.com/aws/api-models-aws/main
codegen-download:
	@echo "Downloading Smithy models from aws/api-models-aws..."
	@curl -sL $(SMITHY_MODELS_REPO)/models/ssm/service/2014-11-06/ssm-2014-11-06.json -o codegen/smithy-model/ssm.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/eventbridge/service/2015-10-07/eventbridge-2015-10-07.json -o codegen/smithy-model/events.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/dynamodb/service/2012-08-10/dynamodb-2012-08-10.json -o codegen/smithy-model/dynamodb.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/sqs/service/2012-11-05/sqs-2012-11-05.json -o codegen/smithy-model/sqs.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/sns/service/2010-03-31/sns-2010-03-31.json -o codegen/smithy-model/sns.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/lambda/service/2015-03-31/lambda-2015-03-31.json -o codegen/smithy-model/lambda.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/kms/service/2014-11-01/kms-2014-11-01.json -o codegen/smithy-model/kms.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/kinesis/service/2013-12-02/kinesis-2013-12-02.json -o codegen/smithy-model/kinesis.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/cloudwatch-logs/service/2014-03-28/cloudwatch-logs-2014-03-28.json -o codegen/smithy-model/logs.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/secrets-manager/service/2017-10-17/secrets-manager-2017-10-17.json -o codegen/smithy-model/secretsmanager.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/ses/service/2010-12-01/ses-2010-12-01.json -o codegen/smithy-model/ses.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/apigatewayv2/service/2018-11-29/apigatewayv2-2018-11-29.json -o codegen/smithy-model/apigatewayv2.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/cloudwatch/service/2010-08-01/cloudwatch-2010-08-01.json -o codegen/smithy-model/cloudwatch.json
	@echo "Done."

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

test-events-unit:
	@cargo test -p ruststack-events-model -p ruststack-events-core -p ruststack-events-http

test-events-patterns:
	@cargo test -p ruststack-events-core -- pattern

test-events-integration:
	@cargo test -p ruststack-integration -- events --ignored

test-apigatewayv2-unit:
	@cargo test -p ruststack-apigatewayv2-model -p ruststack-apigatewayv2-core -p ruststack-apigatewayv2-http

test-apigatewayv2-integration:
	@cargo test -p ruststack-integration -- apigatewayv2 --ignored

update-submodule:
	@git submodule update --init --recursive --remote

test-cloudwatch-unit:
	@cargo test -p ruststack-cloudwatch-model -p ruststack-cloudwatch-core -p ruststack-cloudwatch-http

test-cloudwatch-integration:
	@cargo test -p ruststack-integration -- cloudwatch --ignored

.PHONY: build check test fmt clippy audit deny run release update-submodule integration \
	codegen codegen-s3 codegen-ssm codegen-events codegen-dynamodb codegen-sqs codegen-sns codegen-lambda \
	codegen-kms codegen-kinesis codegen-logs codegen-secretsmanager codegen-ses codegen-apigatewayv2 codegen-cloudwatch codegen-download \
	mint mint-build mint-start mint-run mint-stop \
	alternator alternator-setup alternator-run alternator-stop \
	sqs-compat sqs-compat-setup sqs-compat-run \
	test-events-unit test-events-patterns test-events-integration \
	test-apigatewayv2-unit test-apigatewayv2-integration
