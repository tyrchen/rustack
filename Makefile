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
	@cargo run -p rustack

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin master
	@cargo release push --execute

codegen-s3:
	@cd codegen && cargo run -- --config services/s3.toml --model smithy-model/s3.json --output ../crates/rustack-s3-model/src
	@cargo +nightly fmt -p rustack-s3-model

codegen-ssm:
	@cd codegen && cargo run -- --config services/ssm.toml --model smithy-model/ssm.json --output ../crates/rustack-ssm-model/src
	@cargo +nightly fmt -p rustack-ssm-model

codegen-events:
	@cd codegen && cargo run -- --config services/events.toml --model smithy-model/events.json --output ../crates/rustack-events-model/src
	@cargo +nightly fmt -p rustack-events-model

codegen-dynamodb:
	@cd codegen && cargo run -- --config services/dynamodb.toml --model smithy-model/dynamodb.json --output ../crates/rustack-dynamodb-model/src
	@cargo +nightly fmt -p rustack-dynamodb-model

codegen-sqs:
	@cd codegen && cargo run -- --config services/sqs.toml --model smithy-model/sqs.json --output ../crates/rustack-sqs-model/src
	@cargo +nightly fmt -p rustack-sqs-model

codegen-sns:
	@cd codegen && cargo run -- --config services/sns.toml --model smithy-model/sns.json --output ../crates/rustack-sns-model/src
	@cargo +nightly fmt -p rustack-sns-model

codegen-lambda:
	@cd codegen && cargo run -- --config services/lambda.toml --model smithy-model/lambda.json --output ../crates/rustack-lambda-model/src
	@cargo +nightly fmt -p rustack-lambda-model

codegen-kms:
	@cd codegen && cargo run -- --config services/kms.toml --model smithy-model/kms.json --output ../crates/rustack-kms-model/src
	@cargo +nightly fmt -p rustack-kms-model

codegen-kinesis:
	@cd codegen && cargo run -- --config services/kinesis.toml --model smithy-model/kinesis.json --output ../crates/rustack-kinesis-model/src
	@cargo +nightly fmt -p rustack-kinesis-model

codegen-logs:
	@cd codegen && cargo run -- --config services/logs.toml --model smithy-model/logs.json --output ../crates/rustack-logs-model/src
	@cargo +nightly fmt -p rustack-logs-model

codegen-secretsmanager:
	@cd codegen && cargo run -- --config services/secretsmanager.toml --model smithy-model/secretsmanager.json --output ../crates/rustack-secretsmanager-model/src
	@cargo +nightly fmt -p rustack-secretsmanager-model

codegen-ses:
	@cd codegen && cargo run -- --config services/ses.toml --model smithy-model/ses.json --output ../crates/rustack-ses-model/src
	@cargo +nightly fmt -p rustack-ses-model

codegen-apigatewayv2:
	@cd codegen && cargo run -- --config services/apigatewayv2.toml --model smithy-model/apigatewayv2.json --output ../crates/rustack-apigatewayv2-model/src
	@cargo +nightly fmt -p rustack-apigatewayv2-model

codegen-cloudwatch:
	@cd codegen && cargo run -- --config services/cloudwatch.toml --model smithy-model/cloudwatch.json --output ../crates/rustack-cloudwatch-model/src
	@cargo +nightly fmt -p rustack-cloudwatch-model

codegen-dynamodbstreams:
	@cd codegen && cargo run -- --config services/dynamodbstreams.toml --model smithy-model/dynamodbstreams.json --output ../crates/rustack-dynamodbstreams-model/src
	@cargo +nightly fmt -p rustack-dynamodbstreams-model

codegen-iam:
	@cd codegen && cargo run -- --config services/iam.toml --model smithy-model/iam.json --output ../crates/rustack-iam-model/src
	@cargo +nightly fmt -p rustack-iam-model

codegen-sts:
	@cd codegen && cargo run -- --config services/sts.toml --model smithy-model/sts.json --output ../crates/rustack-sts-model/src
	@cargo +nightly fmt -p rustack-sts-model

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
	@curl -sL $(SMITHY_MODELS_REPO)/models/dynamodb-streams/service/2012-08-10/dynamodb-streams-2012-08-10.json -o codegen/smithy-model/dynamodbstreams.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/iam/service/2010-05-08/iam-2010-05-08.json -o codegen/smithy-model/iam.json
	@curl -sL $(SMITHY_MODELS_REPO)/models/sts/service/2011-06-15/sts-2011-06-15.json -o codegen/smithy-model/sts.json
	@echo "Done."

integration:
	@cargo test -p rustack-integration -- --ignored

# Real-execution Lambda invoke tests (native backend).
# Builds a Rust bootstrap fixture and invokes it through the rustack provider.
test-lambda-invoke-native:
	@RUSTACK_LAMBDA_NATIVE_TESTS=1 cargo test -p rustack-integration test_lambda_invoke -- --ignored --test-threads=1

SQUIB_LAMBDA_BUILD := $(CURDIR)/target/rustack-lambda-squib
SQUIB_LAMBDA_KERNEL_VERSION ?= 6.1.141
SQUIB_LAMBDA_KERNEL_URL ?= https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.13/aarch64/vmlinux-$(SQUIB_LAMBDA_KERNEL_VERSION)
SQUIB_LAMBDA_AL2023_INDEX ?= https://cdn.amazonlinux.com/al2023/os-images/latest/container-minimal-arm64
SQUIB_LAMBDA_BUSYBOX_ALPINE_VERSION ?= 3.20.5
SQUIB_LAMBDA_BUSYBOX_ALPINE_MAJOR := $(shell echo $(SQUIB_LAMBDA_BUSYBOX_ALPINE_VERSION) | cut -d. -f1,2)
SQUIB_LAMBDA_BUSYBOX_ALPINE_URL ?= https://dl-cdn.alpinelinux.org/alpine/v$(SQUIB_LAMBDA_BUSYBOX_ALPINE_MAJOR)/releases/aarch64/alpine-minirootfs-$(SQUIB_LAMBDA_BUSYBOX_ALPINE_VERSION)-aarch64.tar.gz
SQUIB_LAMBDA_ENTITLEMENTS := assets/lambda/squib/hypervisor.entitlements

lambda-squib-agent:
	@rustup target add aarch64-unknown-linux-musl
	@cargo zigbuild --manifest-path tools/lambda-squib-agent/Cargo.toml --target-dir target --release --target aarch64-unknown-linux-musl

lambda-squib-image: lambda-squib-agent
	@mkdir -p $(SQUIB_LAMBDA_BUILD)
	@if [ ! -f "$(SQUIB_LAMBDA_BUILD)/Image" ]; then \
		echo "Downloading Squib Lambda kernel: $(SQUIB_LAMBDA_KERNEL_URL)"; \
		curl -fsSL -o "$(SQUIB_LAMBDA_BUILD)/Image" "$(SQUIB_LAMBDA_KERNEL_URL)"; \
	else \
		echo "Reusing $(SQUIB_LAMBDA_BUILD)/Image"; \
	fi
	@if [ ! -f "$(SQUIB_LAMBDA_BUILD)/al2023-minimal-rootfs.tar.xz" ]; then \
		echo "Resolving AL2023 minimal arm64 rootfs from $(SQUIB_LAMBDA_AL2023_INDEX)"; \
		curl -fsSL -o "$(SQUIB_LAMBDA_BUILD)/al2023-index.html" "$(SQUIB_LAMBDA_AL2023_INDEX)/"; \
		rootfs_file=$$(sed -nE 's/.*(al2023-container-minimal-[^"< ]+-arm64\.tar\.xz).*/\1/p' "$(SQUIB_LAMBDA_BUILD)/al2023-index.html" | head -n 1); \
		test -n "$$rootfs_file"; \
		echo "Downloading AL2023 minimal arm64 rootfs: $$rootfs_file"; \
		curl -fsSL -o "$(SQUIB_LAMBDA_BUILD)/$$rootfs_file" "$(SQUIB_LAMBDA_AL2023_INDEX)/$$rootfs_file"; \
		curl -fsSL -o "$(SQUIB_LAMBDA_BUILD)/SHA256SUMS" "$(SQUIB_LAMBDA_AL2023_INDEX)/SHA256SUMS"; \
		(cd "$(SQUIB_LAMBDA_BUILD)" && shasum -a 256 -c SHA256SUMS); \
		cp "$(SQUIB_LAMBDA_BUILD)/$$rootfs_file" "$(SQUIB_LAMBDA_BUILD)/al2023-minimal-rootfs.tar.xz"; \
	fi
	@if [ ! -f "$(SQUIB_LAMBDA_BUILD)/alpine-busybox-rootfs.tar.gz" ]; then \
		echo "Downloading Alpine busybox helper rootfs: $(SQUIB_LAMBDA_BUSYBOX_ALPINE_URL)"; \
		curl -fsSL -o "$(SQUIB_LAMBDA_BUILD)/alpine-busybox-rootfs.tar.gz" "$(SQUIB_LAMBDA_BUSYBOX_ALPINE_URL)"; \
	fi
	@chmod -R u+w "$(SQUIB_LAMBDA_BUILD)/busybox-root" "$(SQUIB_LAMBDA_BUILD)/initramfs-root" 2>/dev/null || true
	@rm -rf "$(SQUIB_LAMBDA_BUILD)/busybox-root" "$(SQUIB_LAMBDA_BUILD)/initramfs-root"
	@mkdir -p "$(SQUIB_LAMBDA_BUILD)/initramfs-root"
	@tar -xJf "$(SQUIB_LAMBDA_BUILD)/al2023-minimal-rootfs.tar.xz" \
		-C "$(SQUIB_LAMBDA_BUILD)/initramfs-root" \
		--exclude './dev/*' \
		--exclude 'dev/*' \
		--exclude './etc/shadow*' \
		--exclude 'etc/shadow*' \
		--exclude './etc/gshadow*' \
		--exclude 'etc/gshadow*'
	@chmod -R u+w "$(SQUIB_LAMBDA_BUILD)/initramfs-root"
	@mkdir -p "$(SQUIB_LAMBDA_BUILD)/initramfs-root/proc" \
		"$(SQUIB_LAMBDA_BUILD)/initramfs-root/sys" \
		"$(SQUIB_LAMBDA_BUILD)/initramfs-root/dev" \
		"$(SQUIB_LAMBDA_BUILD)/initramfs-root/tmp"
	@mkdir -p "$(SQUIB_LAMBDA_BUILD)/busybox-root"
	@tar -xzf "$(SQUIB_LAMBDA_BUILD)/alpine-busybox-rootfs.tar.gz" \
		-C "$(SQUIB_LAMBDA_BUILD)/busybox-root" \
		./bin/busybox \
		./lib/ld-musl-aarch64.so.1 \
		./lib/libc.musl-aarch64.so.1
	@cp "$(SQUIB_LAMBDA_BUILD)/busybox-root/bin/busybox" "$(SQUIB_LAMBDA_BUILD)/initramfs-root/usr/bin/busybox"
	@chmod +x "$(SQUIB_LAMBDA_BUILD)/initramfs-root/usr/bin/busybox"
	@cp "$(SQUIB_LAMBDA_BUILD)/busybox-root/lib/ld-musl-aarch64.so.1" "$(SQUIB_LAMBDA_BUILD)/initramfs-root/lib/"
	@cp "$(SQUIB_LAMBDA_BUILD)/busybox-root/lib/libc.musl-aarch64.so.1" "$(SQUIB_LAMBDA_BUILD)/initramfs-root/lib/"
	@cp assets/lambda/squib/init "$(SQUIB_LAMBDA_BUILD)/initramfs-root/init"
	@chmod +x "$(SQUIB_LAMBDA_BUILD)/initramfs-root/init"
	@cp target/aarch64-unknown-linux-musl/release/rustack-lambda-squib-agent "$(SQUIB_LAMBDA_BUILD)/initramfs-root/sbin/rustack-lambda-squib-agent"
	@chmod +x "$(SQUIB_LAMBDA_BUILD)/initramfs-root/sbin/rustack-lambda-squib-agent"
	@(cd "$(SQUIB_LAMBDA_BUILD)/initramfs-root" && find . -print0 | cpio --null -o -H newc 2>/dev/null) | gzip -9n > "$(SQUIB_LAMBDA_BUILD)/initramfs.cpio.gz"
	@printf '%s\n' '{"boot-source":{"kernel_image_path":"$(SQUIB_LAMBDA_BUILD)/Image","initrd_path":"$(SQUIB_LAMBDA_BUILD)/initramfs.cpio.gz","boot_args":"console=ttyAMA0 earlycon=pl011,mmio32,0x0e0a0000 panic=1 reboot=k root=/dev/ram0 rdinit=/init"},"machine-config":{"vcpu_count":1,"mem_size_mib":512,"smt":false},"vsock":{"guest_cid":3,"uds_path":"$(SQUIB_LAMBDA_BUILD)/vsock.sock"}}' > "$(SQUIB_LAMBDA_BUILD)/config.json"
	@echo "Squib Lambda image ready:"
	@echo "  $(SQUIB_LAMBDA_BUILD)/config.json"

test-lambda-invoke-squib: lambda-squib-image
	@cargo test -p rustack-integration test_should_invoke_cargo_lambda_arm64_zip_through_auto_squib --no-run --quiet
	@for bin in $$(cargo test -p rustack-integration test_should_invoke_cargo_lambda_arm64_zip_through_auto_squib --no-run --message-format=json 2>/dev/null \
		| jq -r 'select(.profile.test == true) | .filenames[]'); do \
		echo "signing $$bin"; \
		codesign --sign - --entitlements $(SQUIB_LAMBDA_ENTITLEMENTS) --deep --force $$bin; \
	done
	@RUSTACK_LAMBDA_SQUIB_E2E=1 cargo test -p rustack-integration test_should_invoke_cargo_lambda_arm64_zip_through_auto_squib -- --ignored --nocapture

mint: mint-start mint-run

mint-build:
	@cargo build --release -p rustack

mint-start: mint-build
	@echo "Starting Rustack server..."
	@ACCESS_KEY=minioadmin SECRET_KEY=minioadmin \
		S3_SKIP_SIGNATURE_VALIDATION=false \
		DYNAMODB_SKIP_SIGNATURE_VALIDATION=false \
		GATEWAY_LISTEN=0.0.0.0:4566 \
		LOG_LEVEL=warn \
		cargo run --release -p rustack-cli &
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
	@pkill -f "rustack" 2>/dev/null || true
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
	@pkill -f "rustack" 2>/dev/null || true
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

pulumi-smoke:
	@bash scripts/pulumi-rustack-smoke.sh

pulumi-hackathon-smoke:
	@PULUMI_EXAMPLE_DIR="$(CURDIR)/examples/pulumi/hackathon-app" \
		PULUMI_STACK="$${PULUMI_STACK:-rustack-hackathon}" \
		bash scripts/pulumi-rustack-smoke.sh

pulumi-commerce-smoke:
	@PULUMI_EXAMPLE_DIR="$(CURDIR)/examples/pulumi/commerce-platform-app" \
		PULUMI_STACK="$${PULUMI_STACK:-rustack-commerce}" \
		bash scripts/pulumi-rustack-smoke.sh

pulumi-hackathon-snapshot-smoke:
	@bash -euo pipefail -c '\
		ROOT="$(CURDIR)"; \
		ENDPOINT="$${RUSTACK_SNAPSHOT_ENDPOINT:-http://127.0.0.1:4577}"; \
		STACK="$${PULUMI_STACK:-rustack-hackathon-snapshot}"; \
		SNAPSHOT_NAME="$${RUSTACK_SNAPSHOT_NAME:-hackathon-snapshot}"; \
		TMP_DIR=$$(mktemp -d); \
		STATE_DIR="$$TMP_DIR/pulumi"; \
		SNAPSHOT_DIR="$$TMP_DIR/snapshots"; \
		PID1="$$TMP_DIR/rustack-save.pid"; \
		PID2="$$TMP_DIR/rustack-load.pid"; \
		LOAD_MS_FILE="$$TMP_DIR/rustack-load.ms"; \
		PERF_FILE="$$TMP_DIR/snapshot-perf.txt"; \
		SAVE_BUDGET_MS="$${RUSTACK_SNAPSHOT_SAVE_BUDGET_MS:-500}"; \
		LOAD_BUDGET_MS="$${RUSTACK_SNAPSHOT_LOAD_BUDGET_MS:-200}"; \
		now_ms() { node -e "process.stdout.write(String(Date.now()))"; }; \
		metric_value() { grep "^$$1=" "$$PERF_FILE" | tail -n 1 | cut -d= -f2; }; \
		stop_pid_file() { \
			local pid_file="$$1"; \
			if [[ -f "$$pid_file" ]]; then \
				local pid; \
				pid=$$(cat "$$pid_file"); \
				if kill -0 "$$pid" >/dev/null 2>&1; then \
					kill -INT "$$pid" >/dev/null 2>&1 || true; \
					for _ in $$(seq 1 1200); do \
						if ! kill -0 "$$pid" >/dev/null 2>&1; then return 0; fi; \
						sleep 0.05; \
					done; \
					echo "Rustack process $$pid did not stop after SIGINT" >&2; \
					return 1; \
				fi; \
			fi; \
		}; \
		sum_named_files() { \
			local root="$$1"; \
			local name="$$2"; \
			local total=0; \
			local file; \
			local size; \
			while IFS= read -r -d "" file; do \
				size=$$(wc -c <"$$file" | tr -d " "); \
				total=$$((total + size)); \
			done < <(find "$$root" -name "$$name" -type f -print0); \
			printf "%s\n" "$$total"; \
		}; \
		cleanup() { \
			stop_pid_file "$$PID1" >/dev/null 2>&1 || true; \
			stop_pid_file "$$PID2" >/dev/null 2>&1 || true; \
			rm -rf "$$TMP_DIR"; \
		}; \
		trap cleanup EXIT; \
		echo "Creating hackathon snapshot $$SNAPSHOT_NAME at $$SNAPSHOT_DIR"; \
		PULUMI_EXAMPLE_DIR="$$ROOT/examples/pulumi/hackathon-app" \
			PULUMI_STATE_DIR="$$STATE_DIR" \
			PULUMI_STACK="$$STACK" \
			RUSTACK_ENDPOINT="$$ENDPOINT" \
			RUSTACK_SNAPSHOT_DIR="$$SNAPSHOT_DIR" \
			RUSTACK_SNAPSHOT_PERF_FILE="$$PERF_FILE" \
			RUSTACK_EXTRA_ARGS="--snapshot $$SNAPSHOT_NAME" \
			RUSTACK_KEEP_RUNNING=1 \
			RUSTACK_PID_FILE="$$PID1" \
			PULUMI_KEEP_STACK=1 \
			PULUMI_SKIP_DESTROY=1 \
			bash "$$ROOT/scripts/pulumi-rustack-smoke.sh"; \
		test -f "$$PID1"; \
		export PULUMI_HOME="$$STATE_DIR/home"; \
		export PULUMI_CONFIG_PASSPHRASE=""; \
		export AWS_ACCESS_KEY_ID="$${AWS_ACCESS_KEY_ID:-AKIAIOSFODNN7EXAMPLE}"; \
		export AWS_SECRET_ACCESS_KEY="$${AWS_SECRET_ACCESS_KEY:-wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY}"; \
		export AWS_DEFAULT_REGION="$${AWS_DEFAULT_REGION:-us-east-1}"; \
		cd "$$ROOT/examples/pulumi/hackathon-app"; \
		pulumi login "file://$$STATE_DIR/state" >/dev/null; \
		FRONTEND_BUCKET=$$(pulumi stack output frontendBucketName --stack "$$STACK"); \
		PROJECTS_TABLE=$$(pulumi stack output projectsTableName --stack "$$STACK"); \
		curl -sf "$${ENDPOINT%/}/$$FRONTEND_BUCKET/index.html" | grep -q "Hackathon app deployed by Rustack"; \
		curl -sf -X POST "$${ENDPOINT%/}/" \
			-H "x-amz-target: DynamoDB_20120810.PutItem" \
			-H "content-type: application/x-amz-json-1.0" \
			--data "{\"TableName\":\"$$PROJECTS_TABLE\",\"Item\":{\"pk\":{\"S\":\"PROJECT\"},\"sk\":{\"S\":\"snapshot-smoke\"},\"title\":{\"S\":\"Snapshot Smoke\"}}}" >/dev/null; \
		SAVE_STARTED_MS=$$(now_ms); \
		stop_pid_file "$$PID1"; \
		SAVE_WALL_MS=$$(( $$(now_ms) - SAVE_STARTED_MS )); \
		SNAPSHOT_PATH="$$SNAPSHOT_DIR/$$SNAPSHOT_NAME"; \
		test -f "$$SNAPSHOT_PATH/manifest.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/s3/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/s3/data.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/dynamodb/meta.ss.zst"; \
		if find "$$SNAPSHOT_PATH" \( -path "*/objects/*.bin" -o -path "*/parts/*.bin" \) -type f | grep -q .; then \
			echo "snapshot contains unpacked S3 payload files" >&2; \
			exit 1; \
		fi; \
		echo "Reloading snapshot and refreshing Pulumi state"; \
		PULUMI_EXAMPLE_DIR="$$ROOT/examples/pulumi/hackathon-app" \
			PULUMI_STATE_DIR="$$STATE_DIR" \
			PULUMI_STACK="$$STACK" \
			PULUMI_OPERATION=refresh \
			RUSTACK_ENDPOINT="$$ENDPOINT" \
			RUSTACK_SNAPSHOT_DIR="$$SNAPSHOT_DIR" \
			RUSTACK_SNAPSHOT_PERF_FILE="$$PERF_FILE" \
			RUSTACK_EXTRA_ARGS="--snapshot $$SNAPSHOT_NAME" \
			RUSTACK_KEEP_RUNNING=1 \
			RUSTACK_PID_FILE="$$PID2" \
			RUSTACK_READY_MS_FILE="$$LOAD_MS_FILE" \
			PULUMI_KEEP_STACK=1 \
			PULUMI_SKIP_DESTROY=1 \
			bash "$$ROOT/scripts/pulumi-rustack-smoke.sh"; \
		test -f "$$PID2"; \
		curl -sf "$${ENDPOINT%/}/$$FRONTEND_BUCKET/index.html" | grep -q "Hackathon app deployed by Rustack"; \
		DDB_ITEM=$$(curl -sf -X POST "$${ENDPOINT%/}/" \
			-H "x-amz-target: DynamoDB_20120810.GetItem" \
			-H "content-type: application/x-amz-json-1.0" \
			--data "{\"TableName\":\"$$PROJECTS_TABLE\",\"Key\":{\"pk\":{\"S\":\"PROJECT\"},\"sk\":{\"S\":\"snapshot-smoke\"}}}"); \
		printf "%s" "$$DDB_ITEM" | grep -q "Snapshot Smoke"; \
		stop_pid_file "$$PID2"; \
		SAVE_MS=$$(metric_value save_ms); \
		LOAD_MS=$$(metric_value load_ms); \
		LOAD_READY_MS=$$(cat "$$LOAD_MS_FILE"); \
		MANIFEST_BYTES=$$(wc -c <"$$SNAPSHOT_PATH/manifest.ss.zst" | tr -d " "); \
		META_BYTES=$$(sum_named_files "$$SNAPSHOT_PATH/services" "meta.ss.zst"); \
		DATA_BYTES=$$(sum_named_files "$$SNAPSHOT_PATH/services" "data.ss.zst"); \
		echo "Hackathon snapshot perf: save_ms=$$SAVE_MS save_wall_ms=$$SAVE_WALL_MS load_ms=$$LOAD_MS load_ready_ms=$$LOAD_READY_MS manifest_bytes=$$MANIFEST_BYTES meta_bytes=$$META_BYTES data_bytes=$$DATA_BYTES"; \
		if (( SAVE_MS > SAVE_BUDGET_MS )); then \
			echo "snapshot save exceeded budget: $${SAVE_MS}ms > $${SAVE_BUDGET_MS}ms" >&2; \
			exit 1; \
		fi; \
		if (( LOAD_MS > LOAD_BUDGET_MS )); then \
			echo "snapshot load exceeded budget: $${LOAD_MS}ms > $${LOAD_BUDGET_MS}ms" >&2; \
			exit 1; \
		fi; \
		echo "Hackathon snapshot smoke passed"; \
	'

pulumi-commerce-snapshot-smoke:
	@bash -euo pipefail -c '\
		ROOT="$(CURDIR)"; \
		ENDPOINT="$${RUSTACK_SNAPSHOT_ENDPOINT:-http://127.0.0.1:4578}"; \
		STACK="$${PULUMI_STACK:-rustack-commerce-snapshot}"; \
		SNAPSHOT_NAME="$${RUSTACK_SNAPSHOT_NAME:-commerce-snapshot}"; \
		S3_RUNTIME_OBJECTS="$${RUSTACK_COMMERCE_S3_RUNTIME_OBJECTS:-320}"; \
		DDB_RUNTIME_ITEMS="$${RUSTACK_COMMERCE_DDB_RUNTIME_ITEMS:-240}"; \
		CDN_WARM_OBJECTS="$${RUSTACK_COMMERCE_CDN_WARM_OBJECTS:-80}"; \
		TMP_DIR=$$(mktemp -d); \
		STATE_DIR="$$TMP_DIR/pulumi"; \
		SNAPSHOT_DIR="$$TMP_DIR/snapshots"; \
		PID1="$$TMP_DIR/rustack-save.pid"; \
		PID2="$$TMP_DIR/rustack-load.pid"; \
		LOAD_MS_FILE="$$TMP_DIR/rustack-load.ms"; \
		PERF_FILE="$$TMP_DIR/snapshot-perf.txt"; \
		SAVE_BUDGET_MS="$${RUSTACK_COMMERCE_SNAPSHOT_SAVE_BUDGET_MS:-1000}"; \
		LOAD_BUDGET_MS="$${RUSTACK_COMMERCE_SNAPSHOT_LOAD_BUDGET_MS:-500}"; \
		now_ms() { node -e "process.stdout.write(String(Date.now()))"; }; \
		metric_value() { grep "^$$1=" "$$PERF_FILE" | tail -n 1 | cut -d= -f2; }; \
		stop_pid_file() { \
			local pid_file="$$1"; \
			if [[ -f "$$pid_file" ]]; then \
				local pid; \
				pid=$$(cat "$$pid_file"); \
				if kill -0 "$$pid" >/dev/null 2>&1; then \
					kill -INT "$$pid" >/dev/null 2>&1 || true; \
					for _ in $$(seq 1 1200); do \
						if ! kill -0 "$$pid" >/dev/null 2>&1; then return 0; fi; \
						sleep 0.05; \
					done; \
					echo "Rustack process $$pid did not stop after SIGINT" >&2; \
					return 1; \
				fi; \
			fi; \
		}; \
		sum_named_files() { \
			local root="$$1"; \
			local name="$$2"; \
			local total=0; \
			local file; \
			local size; \
			while IFS= read -r -d "" file; do \
				size=$$(wc -c <"$$file" | tr -d " "); \
				total=$$((total + size)); \
			done < <(find "$$root" -name "$$name" -type f -print0); \
			printf "%s\n" "$$total"; \
		}; \
		put_s3_runtime_data() { \
			local bucket="$$1"; \
			local count="$$2"; \
			local idx; \
			local padded; \
			for idx in $$(seq 0 $$((count - 1))); do \
				padded=$$(printf "%04d" "$$idx"); \
				curl -sf -X PUT "$${ENDPOINT%/}/$$bucket/runtime/blob-$$padded.json" \
					-H "content-type: application/json" \
					--data "{\"id\":\"blob-$$padded\",\"payload\":\"commerce-runtime-payload-$$padded-$$(printf "%064d" "$$idx")\"}" >/dev/null; \
			done; \
		}; \
		put_ddb_runtime_data() { \
			local table="$$1"; \
			local entity="$$2"; \
			local count="$$3"; \
			local idx; \
			local padded; \
			for idx in $$(seq 0 $$((count - 1))); do \
				padded=$$(printf "%04d" "$$idx"); \
				curl -sf -X POST "$${ENDPOINT%/}/" \
					-H "x-amz-target: DynamoDB_20120810.PutItem" \
					-H "content-type: application/x-amz-json-1.0" \
					--data "{\"TableName\":\"$$table\",\"Item\":{\"pk\":{\"S\":\"$$entity\"},\"sk\":{\"S\":\"$$entity#$$padded\"},\"status\":{\"S\":\"ACTIVE\"},\"payload\":{\"S\":\"commerce-ddb-payload-$$padded-$$(printf "%064d" "$$idx")\"}}}" >/dev/null; \
			done; \
		}; \
		warm_cdn_cache() { \
			local distribution_id="$$1"; \
			local count="$$2"; \
			local idx; \
			local page; \
			for idx in $$(seq 0 $$((count - 1))); do \
				page=$$(printf "%03d" $$((idx % 48))); \
				curl -sf "$${ENDPOINT%/}/_aws/cloudfront/$$distribution_id/static/page-$$page.html" >/dev/null; \
			done; \
			curl -sD "$$TMP_DIR/cdn-hit.headers" -o "$$TMP_DIR/cdn-hit.body" \
				"$${ENDPOINT%/}/_aws/cloudfront/$$distribution_id/static/page-000.html" >/dev/null; \
			grep -qi "x-cache: Hit from rustack-cloudfront" "$$TMP_DIR/cdn-hit.headers"; \
			grep -q "Commerce Platform Rustack fixture page 0" "$$TMP_DIR/cdn-hit.body"; \
		}; \
		cleanup() { \
			stop_pid_file "$$PID1" >/dev/null 2>&1 || true; \
			stop_pid_file "$$PID2" >/dev/null 2>&1 || true; \
			rm -rf "$$TMP_DIR"; \
		}; \
		trap cleanup EXIT; \
		echo "Creating commerce snapshot $$SNAPSHOT_NAME at $$SNAPSHOT_DIR"; \
		PULUMI_EXAMPLE_DIR="$$ROOT/examples/pulumi/commerce-platform-app" \
			PULUMI_STATE_DIR="$$STATE_DIR" \
			PULUMI_STACK="$$STACK" \
			RUSTACK_ENDPOINT="$$ENDPOINT" \
			RUSTACK_SNAPSHOT_DIR="$$SNAPSHOT_DIR" \
			RUSTACK_SNAPSHOT_PERF_FILE="$$PERF_FILE" \
			RUSTACK_EXTRA_ARGS="--snapshot $$SNAPSHOT_NAME" \
			RUSTACK_KEEP_RUNNING=1 \
			RUSTACK_PID_FILE="$$PID1" \
			PULUMI_KEEP_STACK=1 \
			PULUMI_SKIP_DESTROY=1 \
			bash "$$ROOT/scripts/pulumi-rustack-smoke.sh"; \
		test -f "$$PID1"; \
		export PULUMI_HOME="$$STATE_DIR/home"; \
		export PULUMI_CONFIG_PASSPHRASE=""; \
		export AWS_ACCESS_KEY_ID="$${AWS_ACCESS_KEY_ID:-AKIAIOSFODNN7EXAMPLE}"; \
		export AWS_SECRET_ACCESS_KEY="$${AWS_SECRET_ACCESS_KEY:-wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY}"; \
		export AWS_DEFAULT_REGION="$${AWS_DEFAULT_REGION:-us-east-1}"; \
		cd "$$ROOT/examples/pulumi/commerce-platform-app"; \
		pulumi login "file://$$STATE_DIR/state" >/dev/null; \
		PUBLIC_BUCKET=$$(pulumi stack output publicBucketName --stack "$$STACK"); \
		MEDIA_BUCKET=$$(pulumi stack output mediaBucketName --stack "$$STACK"); \
		CATALOG_TABLE=$$(pulumi stack output catalogTableName --stack "$$STACK"); \
		ORDERS_TABLE=$$(pulumi stack output ordersTableName --stack "$$STACK"); \
		DISTRIBUTION_ID=$$(pulumi stack output cloudfrontDistributionId --stack "$$STACK"); \
		curl -sf "$${ENDPOINT%/}/$$PUBLIC_BUCKET/static/page-000.html" | grep -q "Commerce Platform Rustack fixture page 0"; \
		put_s3_runtime_data "$$MEDIA_BUCKET" "$$S3_RUNTIME_OBJECTS"; \
		put_ddb_runtime_data "$$CATALOG_TABLE" "PRODUCT_RT" "$$DDB_RUNTIME_ITEMS"; \
		put_ddb_runtime_data "$$ORDERS_TABLE" "ORDER_RT" "$$DDB_RUNTIME_ITEMS"; \
		warm_cdn_cache "$$DISTRIBUTION_ID" "$$CDN_WARM_OBJECTS"; \
		SAVE_STARTED_MS=$$(now_ms); \
		stop_pid_file "$$PID1"; \
		SAVE_WALL_MS=$$(( $$(now_ms) - SAVE_STARTED_MS )); \
		SNAPSHOT_PATH="$$SNAPSHOT_DIR/$$SNAPSHOT_NAME"; \
		test -f "$$SNAPSHOT_PATH/manifest.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/s3/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/s3/data.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/dynamodb/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/lambda/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/cloudfront/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/cloudfront-cache/meta.ss.zst"; \
		test -f "$$SNAPSHOT_PATH/services/cloudfront-cache/data.ss.zst"; \
		if find "$$SNAPSHOT_PATH" \( -path "*/objects/*.bin" -o -path "*/parts/*.bin" -o -path "*/bodies/*.bin" \) -type f | grep -q .; then \
			echo "snapshot contains unpacked payload files" >&2; \
			exit 1; \
		fi; \
		echo "Reloading commerce snapshot and refreshing Pulumi state"; \
		PULUMI_EXAMPLE_DIR="$$ROOT/examples/pulumi/commerce-platform-app" \
			PULUMI_STATE_DIR="$$STATE_DIR" \
			PULUMI_STACK="$$STACK" \
			PULUMI_OPERATION=refresh \
			RUSTACK_ENDPOINT="$$ENDPOINT" \
			RUSTACK_SNAPSHOT_DIR="$$SNAPSHOT_DIR" \
			RUSTACK_SNAPSHOT_PERF_FILE="$$PERF_FILE" \
			RUSTACK_EXTRA_ARGS="--snapshot $$SNAPSHOT_NAME" \
			RUSTACK_KEEP_RUNNING=1 \
			RUSTACK_PID_FILE="$$PID2" \
			RUSTACK_READY_MS_FILE="$$LOAD_MS_FILE" \
			PULUMI_KEEP_STACK=1 \
			PULUMI_SKIP_DESTROY=1 \
			bash "$$ROOT/scripts/pulumi-rustack-smoke.sh"; \
		test -f "$$PID2"; \
		curl -sf "$${ENDPOINT%/}/$$MEDIA_BUCKET/runtime/blob-0000.json" | grep -q "commerce-runtime-payload-0000"; \
		DDB_ITEM=$$(curl -sf -X POST "$${ENDPOINT%/}/" \
			-H "x-amz-target: DynamoDB_20120810.GetItem" \
			-H "content-type: application/x-amz-json-1.0" \
			--data "{\"TableName\":\"$$ORDERS_TABLE\",\"Key\":{\"pk\":{\"S\":\"ORDER_RT\"},\"sk\":{\"S\":\"ORDER_RT#0000\"}}}"); \
		printf "%s" "$$DDB_ITEM" | grep -q "commerce-ddb-payload-0000"; \
		curl -sD "$$TMP_DIR/cdn-load.headers" -o "$$TMP_DIR/cdn-load.body" \
			"$${ENDPOINT%/}/_aws/cloudfront/$$DISTRIBUTION_ID/static/page-000.html" >/dev/null; \
		grep -qi "x-cache: Hit from rustack-cloudfront" "$$TMP_DIR/cdn-load.headers"; \
		grep -q "Commerce Platform Rustack fixture page 0" "$$TMP_DIR/cdn-load.body"; \
		stop_pid_file "$$PID2"; \
		SAVE_MS=$$(metric_value save_ms); \
		LOAD_MS=$$(metric_value load_ms); \
		LOAD_READY_MS=$$(cat "$$LOAD_MS_FILE"); \
		MANIFEST_BYTES=$$(wc -c <"$$SNAPSHOT_PATH/manifest.ss.zst" | tr -d " "); \
		META_BYTES=$$(sum_named_files "$$SNAPSHOT_PATH/services" "meta.ss.zst"); \
		DATA_BYTES=$$(sum_named_files "$$SNAPSHOT_PATH/services" "data.ss.zst"); \
		RESOURCE_COUNT=$$(pulumi stack --stack "$$STACK" --show-urns | grep -c "urn:pulumi:"); \
		echo "Commerce snapshot perf: resources=$$RESOURCE_COUNT s3_runtime_objects=$$S3_RUNTIME_OBJECTS ddb_runtime_items=$$((DDB_RUNTIME_ITEMS * 2)) cdn_warm_requests=$$CDN_WARM_OBJECTS save_ms=$$SAVE_MS save_wall_ms=$$SAVE_WALL_MS load_ms=$$LOAD_MS load_ready_ms=$$LOAD_READY_MS manifest_bytes=$$MANIFEST_BYTES meta_bytes=$$META_BYTES data_bytes=$$DATA_BYTES"; \
		if (( SAVE_MS > SAVE_BUDGET_MS )); then \
			echo "commerce snapshot save exceeded budget: $${SAVE_MS}ms > $${SAVE_BUDGET_MS}ms" >&2; \
			exit 1; \
		fi; \
		if (( LOAD_MS > LOAD_BUDGET_MS )); then \
			echo "commerce snapshot load exceeded budget: $${LOAD_MS}ms > $${LOAD_BUDGET_MS}ms" >&2; \
			exit 1; \
		fi; \
		echo "Commerce snapshot smoke passed"; \
	'

test-events-unit:
	@cargo test -p rustack-events-model -p rustack-events-core -p rustack-events-http

test-events-patterns:
	@cargo test -p rustack-events-core -- pattern

test-events-integration:
	@cargo test -p rustack-integration -- events --ignored

test-apigatewayv2-unit:
	@cargo test -p rustack-apigatewayv2-model -p rustack-apigatewayv2-core -p rustack-apigatewayv2-http

test-apigatewayv2-integration:
	@cargo test -p rustack-integration -- apigatewayv2 --ignored

update-submodule:
	@git submodule update --init --recursive --remote

test-cloudwatch-unit:
	@cargo test -p rustack-cloudwatch-model -p rustack-cloudwatch-core -p rustack-cloudwatch-http

test-cloudwatch-integration:
	@cargo test -p rustack-integration -- cloudwatch --ignored

test-iam-unit:
	@cargo test -p rustack-iam-model -p rustack-iam-core -p rustack-iam-http

test-iam-integration:
	@cargo test -p rustack-integration -- iam --ignored

.PHONY: build check test fmt clippy audit deny run release update-submodule integration \
	codegen codegen-s3 codegen-ssm codegen-events codegen-dynamodb codegen-dynamodbstreams codegen-sqs codegen-sns codegen-lambda \
	codegen-kms codegen-kinesis codegen-logs codegen-secretsmanager codegen-ses codegen-apigatewayv2 codegen-cloudwatch codegen-iam codegen-download \
	mint mint-build mint-start mint-run mint-stop \
	alternator alternator-setup alternator-run alternator-stop \
	sqs-compat sqs-compat-setup sqs-compat-run \
	pulumi-smoke pulumi-hackathon-smoke pulumi-hackathon-snapshot-smoke \
	pulumi-commerce-smoke pulumi-commerce-snapshot-smoke \
	test-events-unit test-events-patterns test-events-integration \
	test-apigatewayv2-unit test-apigatewayv2-integration \
	test-iam-unit test-iam-integration
