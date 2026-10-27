build:
	@cargo build

# P0 evidence tooling is isolated from the production workspace/runtime.
WORKFLOWS_SPIKE_DIR ?= /tmp/rustack-spikes
WORKFLOWS_DOCKER_SOCKET ?= /var/run/docker.sock
WORKFLOWS_ARCH ?= $(if $(filter arm64 aarch64,$(shell uname -m)),arm64,amd64)
WORKFLOWS_IMAGE_BUILDER ?= docker buildx
LAMBDA_RUNTIME_DIR ?= /tmp/rustack-lambda-runtime
LAMBDA_RUNTIME_IMAGE ?= rustack/lambda-runtime:integration-$(WORKFLOWS_ARCH)

lambda-runtime-build:
	@case "$(WORKFLOWS_ARCH)" in arm64|amd64) ;; *) echo "WORKFLOWS_ARCH must be arm64 or amd64" >&2; exit 1;; esac
	@cargo lambda build -p rustack-lambda-runtime --bin rustack-lambda-runtime --target "$(if $(filter arm64,$(WORKFLOWS_ARCH)),aarch64,x86_64)-unknown-linux-gnu" --release --lambda-dir "$(LAMBDA_RUNTIME_DIR)/$(WORKFLOWS_ARCH)"

lambda-runtime-image: lambda-runtime-build
	@DOCKER_HOST="unix://$(WORKFLOWS_DOCKER_SOCKET)" $(WORKFLOWS_IMAGE_BUILDER) build --load --platform "linux/$(WORKFLOWS_ARCH)" --file assets/lambda/runtime/Dockerfile --tag "$(LAMBDA_RUNTIME_IMAGE)" "$(LAMBDA_RUNTIME_DIR)/$(WORKFLOWS_ARCH)/rustack-lambda-runtime"

lambda-runtime-fixture-image:
	@DOCKER_HOST="unix://$(WORKFLOWS_DOCKER_SOCKET)" $(WORKFLOWS_IMAGE_BUILDER) build --load --platform "linux/$(WORKFLOWS_ARCH)" --file tools/workflows-spikes/Handler.Dockerfile --tag "rustack-p0-handler:$(WORKFLOWS_ARCH)" tools/workflows-spikes

test-lambda-runtime-docker: lambda-runtime-image lambda-runtime-fixture-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-runtime --test docker -- --ignored --test-threads=1

test-lambda-docker-wire: lambda-runtime-image lambda-runtime-fixture-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-core --test docker_wire -- --ignored --test-threads=1

.PHONY: lambda-runtime-build lambda-runtime-image lambda-runtime-fixture-image test-lambda-runtime-docker test-lambda-docker-wire

workflows-model-check:
	@python3 -c 'import hashlib,json,pathlib; p=pathlib.Path("codegen/smithy-model"); m=json.loads((p/"stepfunctions.provenance.json").read_text()); raw=(p/"stepfunctions.json").read_bytes(); assert len(raw)==m["bytes"] and hashlib.sha256(raw).hexdigest()==m["sha256"], "SFN source hash mismatch"; s=json.loads(raw)["shapes"][m["service"]]; assert s["version"]=="2016-11-23" and "aws.protocols#awsJson1_0" in s["traits"] and s["traits"]["aws.auth#sigv4"]["name"]=="states"; print("Pinned SFN model verified:",len(s["operations"]),"operations")'

workflows-fixture-check: workflows-model-check
	@python3 -c 'import json,pathlib; p=pathlib.Path("tests/workflows"); f=json.loads((p/"semantic-cases.json").read_text()); m=json.loads((p/"provenance.json").read_text()); source=json.loads(pathlib.Path("codegen/smithy-model/stepfunctions.provenance.json").read_text()); ids=[c["id"] for c in f["cases"]]; assert f["schemaVersion"]==1 and len(ids)==len(set(ids)) and all(c["source"] in f["sources"] for c in f["cases"]); assert m["awsModelCommit"]==source["commit"] and m["awsModelSha256"]==source["sha256"]; print("Fixture catalog structure/provenance verified:",len(ids),"cases; interpreter execution is not claimed")'

workflows-wire-probe:
	@PYTHONDONTWRITEBYTECODE=1 uv run --no-project --python 3.13 --with-requirements tests/workflows/requirements.txt python -m unittest discover -s tests/workflows -p 'test_botocore_wire.py' -v

workflows-codegen-spike: workflows-model-check
	@cargo run --manifest-path codegen/Cargo.toml -- --config codegen/services/stepfunctions.toml --model codegen/smithy-model/stepfunctions.json --output "$(WORKFLOWS_SPIKE_DIR)/stepfunctions-generated/src"

test-workflows-spikes:
	@cargo test --manifest-path tools/workflows-spikes/Cargo.toml
	@cargo clippy --manifest-path tools/workflows-spikes/Cargo.toml --all-targets -- -D warnings

workflows-relay-spike-build:
	@case "$(WORKFLOWS_ARCH)" in arm64|amd64) ;; *) echo "WORKFLOWS_ARCH must be arm64 or amd64" >&2; exit 1;; esac
	@cargo lambda build --manifest-path tools/workflows-spikes/Cargo.toml --bin relay --target "$(if $(filter arm64,$(WORKFLOWS_ARCH)),aarch64,x86_64)-unknown-linux-gnu" --release --lambda-dir "$(WORKFLOWS_SPIKE_DIR)/$(WORKFLOWS_ARCH)"

workflows-relay-spike-images: workflows-relay-spike-build
	@DOCKER_HOST="unix://$(WORKFLOWS_DOCKER_SOCKET)" $(WORKFLOWS_IMAGE_BUILDER) build --load --platform "linux/$(WORKFLOWS_ARCH)" --file tools/workflows-spikes/Relay.Dockerfile --tag "rustack-p0-relay:$(WORKFLOWS_ARCH)" "$(WORKFLOWS_SPIKE_DIR)/$(WORKFLOWS_ARCH)/relay"
	@DOCKER_HOST="unix://$(WORKFLOWS_DOCKER_SOCKET)" $(WORKFLOWS_IMAGE_BUILDER) build --load --platform "linux/$(WORKFLOWS_ARCH)" --file tools/workflows-spikes/Handler.Dockerfile --tag "rustack-p0-handler:$(WORKFLOWS_ARCH)" tools/workflows-spikes

workflows-relay-spike: workflows-relay-spike-images
	@cargo run --manifest-path tools/workflows-spikes/Cargo.toml --bin driver -- "$(WORKFLOWS_DOCKER_SOCKET)" "$(WORKFLOWS_ARCH)"

.PHONY: workflows-model-check workflows-fixture-check workflows-wire-probe workflows-codegen-spike \
	test-workflows-spikes workflows-relay-spike-build workflows-relay-spike-images workflows-relay-spike

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
	@cargo run -p rustack-cli --bin rustack

# Set PUBLISH_PACKAGES to a space-separated release set; defaults to the workspace.
# Package the set together so unpublished internal dependencies are verified locally.
PUBLISH_PACKAGES ?=

publish-dry-run:
	@cargo publish --dry-run --allow-dirty --locked --all-features \
		$(if $(strip $(PUBLISH_PACKAGES)),$(foreach package,$(PUBLISH_PACKAGES),-p $(package)),--workspace)

publish:
	@cargo publish --locked --all-features \
		$(if $(strip $(PUBLISH_PACKAGES)),$(foreach package,$(PUBLISH_PACKAGES),-p $(package)),$(error Set PUBLISH_PACKAGES to the crates to publish))

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

codegen-stepfunctions:
	@cargo run --manifest-path codegen/Cargo.toml -- --config codegen/services/stepfunctions.toml --model codegen/smithy-model/stepfunctions.json --output crates/rustack-stepfunctions-model/src
	@cargo +nightly fmt -p rustack-stepfunctions-model

check-stepfunctions-codegen:
	@set -eu; generated=$$(mktemp -d /tmp/rustack-sfn-codegen.XXXXXX); \
	trap 'rm -rf "$$generated"' EXIT; \
	cargo run --manifest-path codegen/Cargo.toml -- --config codegen/services/stepfunctions.toml --model codegen/smithy-model/stepfunctions.json --output "$$generated"; \
	rustup run nightly rustfmt --edition 2024 --config-path rustfmt.toml "$$generated"/*.rs; \
	diff -r "$$generated" crates/rustack-stepfunctions-model/src

test-stepfunctions-model: check-stepfunctions-codegen
	@cargo test --manifest-path codegen/Cargo.toml
	@cargo clippy --manifest-path codegen/Cargo.toml --all-targets -- -D warnings
	@cargo test -p rustack-stepfunctions-model
	@cargo clippy -p rustack-stepfunctions-model --lib --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: codegen-stepfunctions check-stepfunctions-codegen test-stepfunctions-model

test-stepfunctions-http:
	@cargo test -p rustack-stepfunctions-http
	@cargo clippy -p rustack-stepfunctions-http --lib --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

test-stepfunctions-http-sdk: test-workflows-manifest
	@cargo test -p rustack-stepfunctions-http --test network test_should_exchange_signed_boto3_requests_with_typed_transport -- --ignored --test-threads=1

.PHONY: test-stepfunctions-http test-stepfunctions-http-sdk

test-stepfunctions-core:
	@cargo test -p rustack-stepfunctions-core
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order dataflow::callback_tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order payload::tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::callback
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order compiler::tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::fanout::tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::scheduler::tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::runtime::tests::structured
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::runtime::tests::callback
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order engine::runtime::structured::callback::tests
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order registry::callback
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order registry::tests::nested
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order registry::tests::children
	@cargo test -p rustack-stepfunctions-core --features serde_json/preserve_order registry::actor::clock_tests::children
	@cargo clippy -p rustack-stepfunctions-core --all-targets -- -D warnings
	@cargo clippy -p rustack-stepfunctions-core --lib --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: test-stepfunctions-core

test-stepfunctions-bridge:
	@cargo test -p rustack-cli --lib --features stepfunctions
	@cargo test -p rustack-cli --doc --features stepfunctions
	@cargo clippy -p rustack-cli --all-targets --features stepfunctions -- -D warnings
	@cargo clippy -p rustack-cli --lib --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

test-stepfunctions-bridge-native: test-workflows-manifest
	@cargo build -p rustack-lambda-echo-bootstrap
	@RUSTACK_WORKFLOWS_BOOTSTRAP="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/bootstrap" cargo test -p rustack-cli --lib --features stepfunctions native_e2e -- --ignored --test-threads=1

# The shared binary-support gate covers real API, pagination and Lambda bridge.
test-stepfunctions-api: test-stepfunctions-bridge

.PHONY: test-stepfunctions-bridge test-stepfunctions-bridge-native test-stepfunctions-api

# Actual main composition and both minimal/runtime feature shapes.
test-stepfunctions-runtime: test-stepfunctions-api
	@cargo test -p rustack-cli --bin rustack --features stepfunctions
	@cargo test -p rustack-cli --bin rustack --no-default-features --features stepfunctions
	@cargo clippy -p rustack-cli --all-targets --no-default-features --features stepfunctions -- -D warnings

	@cargo clippy -p rustack-cli --bin rustack --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic
	@cargo clippy -p rustack-cli --bin rustack --no-default-features --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: test-stepfunctions-runtime

# Dedicated build artifact: uv's ephemeral run environment disappears on exit.
workflows-sdk-python:
	@uv venv --python 3.13 --allow-existing "$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/workflows-sdk-python"
	@uv pip install --python "$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/workflows-sdk-python/bin/python" --requirement tests/workflows/requirements.txt

# Actual provided-runtime handler and strict fixture boundary checks.
test-stepfunctions-handler-fixture:
	@cargo build -p rustack-lambda-echo-bootstrap --features sdk-callback --bin callback-bootstrap
	@cargo clippy -p rustack-lambda-echo-bootstrap --features sdk-callback --bin callback-bootstrap --no-deps -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic
	@python3 -c 'import ast,pathlib; ast.parse(pathlib.Path("tests/lambda-fixtures/callback_bootstrap.py").read_text())'

# Both ordinary SDKs directly own their real client and server processes.
test-stepfunctions-sdk: test-workflows-manifest workflows-sdk-python test-stepfunctions-handler-fixture
	@cargo build -p rustack-lambda-echo-bootstrap
	@RUSTACK_WORKFLOWS_PYTHON="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/workflows-sdk-python/bin/python" RUSTACK_CALLBACK_BOOTSTRAP="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/callback-bootstrap" RUSTACK_WORKFLOWS_BOOTSTRAP="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/bootstrap" cargo test -p rustack-cli --test workflows_sdk --features stepfunctions -- --ignored --test-threads=1 --nocapture

.PHONY: workflows-sdk-python test-stepfunctions-handler-fixture test-stepfunctions-sdk

audit-codegen:
	@cargo audit --file codegen/Cargo.lock

# D69: source-preserving loader overlay and the complete patched codec graph.
verify-workflows-compression:
	@python3 -I -c 'import hashlib,json,pathlib; p=pathlib.Path("patches/squib-loader"); v=json.loads((p/"provenance.json").read_text()); names={"src/lib.rs","Cargo.toml.registry","Cargo.toml.orig",".cargo_vcs_info.json","LICENSE.md"}; assert v["schemaVersion"]==1 and v["package"]=="squib-loader" and v["version"]=="0.2.0"; assert set(v["sha256"])==names; assert all(hashlib.sha256((p/n).read_bytes()).hexdigest()==h for n,h in v["sha256"].items()); original=(p/"Cargo.toml.registry").read_text(); before="[dependencies.zstd]\nversion = \"0.13\""; after="[dependencies.zstd]\nversion = \"0.14.3\""; assert original.count(before)==1 and (p/"Cargo.toml").read_text()==original.replace(before,after); f=pathlib.Path("apps/rustack/src/snapshot/fixtures"); a=json.loads((f/"legacy-v2-meta-provenance.json").read_text()); assert hashlib.sha256((f/a["fixture"]).read_bytes()).hexdigest()==a["sha256"]'
	@cargo metadata --locked --format-version 1 | python3 -I -c 'import json,pathlib,sys; data=json.load(sys.stdin); minimum={"zstd":(0,14,3),"zstd-safe":(8,2,0),"zstd-sys":(2,1,2)}; codecs=[p for p in data["packages"] if p["name"] in minimum]; assert len(codecs)==3 and {p["name"] for p in codecs}==set(minimum); assert all(tuple(map(int,p["version"].split("+")[0].split(".")))>=minimum[p["name"]] for p in codecs); loader=[p for p in data["packages"] if p["name"]=="squib-loader"]; assert len(loader)==1 and loader[0]["source"] is None and pathlib.Path(loader[0]["manifest_path"]).resolve()==pathlib.Path("patches/squib-loader/Cargo.toml").resolve()'

test-workflows-compression: verify-workflows-compression
	@cargo test --locked -p squib-loader --lib
	@cargo test --locked -p rustack-cli --bin rustack --features stepfunctions snapshot::

.PHONY: verify-workflows-compression test-workflows-compression

# D70: installed archive parser/encoder boundaries, including minimal binary builds.
test-workflows-archive:
	@cargo test --locked -p rustack-cli --bin rustack --features stepfunctions snapshot::
	@cargo test --locked -p rustack-cli --bin rustack --no-default-features --features stepfunctions snapshot::
	@cargo clippy --locked -p rustack-cli --bin rustack --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic
	@cargo clippy --locked -p rustack-cli --bin rustack --no-default-features --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: test-workflows-archive

# D71: sealed ownership/export, exact history codec and selected runtime consumer.
test-workflows-snapshot-export:
	@cargo test --locked -p rustack-stepfunctions-core snapshot
	@cargo test --locked -p rustack-stepfunctions-core --features serde_json/preserve_order snapshot
	@cargo clippy --locked -p rustack-stepfunctions-core --lib --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: test-workflows-snapshot-export

# D72: startup-only atomic restore, original quotas, complete history and no replay.
test-workflows-snapshot-restore:
	@cargo test --locked -p rustack-stepfunctions-core registry::snapshot::restore
	@cargo test --locked -p rustack-stepfunctions-core --features serde_json/preserve_order registry::snapshot::restore
	@cargo clippy --locked -p rustack-stepfunctions-core --lib --no-deps -- -D warnings -W clippy::pedantic -W clippy::missing_errors_doc -W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

.PHONY: test-workflows-snapshot-restore

# D73: actual service archives and signed Native binary restart without replay.
test-workflows-persistence:
	@cargo test --locked -p rustack-stepfunctions-core registry::snapshot::archive
	@cargo test --locked -p rustack-cli --bin rustack --features stepfunctions snapshot::
	@cargo test --locked -p rustack-cli --bin rustack --no-default-features --features stepfunctions snapshot::
	@cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions persistence::
	@cargo build --locked -p rustack-lambda-echo-bootstrap --bin bootstrap
	@RUSTACK_WORKFLOWS_BOOTSTRAP="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/bootstrap" cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions persistence:: -- --ignored --test-threads=1 --nocapture

.PHONY: test-workflows-persistence

# D74: actual pure-workflow latency, bounded loops and one-slot Stop reuse.
test-workflows-performance:
	@cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions performance::
	@cargo build --locked -p rustack-lambda-echo-bootstrap --bin bootstrap
	@RUSTACK_WORKFLOWS_BOOTSTRAP="$$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/bootstrap" cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions performance:: -- --ignored --test-threads=1 --nocapture
	@cargo clippy --locked -p rustack-cli --test workflows_sdk --features stepfunctions --no-deps -- -D warnings -W clippy::pedantic

.PHONY: test-workflows-performance

# D75: exact same-feature resource inventories, before any resource execution.
test-workflows-manifest:
	@python3 -I -B -m unittest discover -s tests/workflows -p test_required_suites.py -v
	@set -eu; \
		workflow_manifest_dir=$$(mktemp -d); \
		trap 'rm -rf "$$workflow_manifest_dir"' EXIT; \
		cargo test --locked -p rustack-cli --lib --features stepfunctions -- --ignored --list --color never >"$$workflow_manifest_dir/native-bridge.txt"; \
		cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions -- --ignored --list --color never >"$$workflow_manifest_dir/registered-sdk.txt"; \
		cargo test --locked -p rustack-stepfunctions-http --test network -- --ignored --list --color never >"$$workflow_manifest_dir/typed-http-sdk.txt"; \
		python3 -I -B tests/workflows/required_suites.py "$$workflow_manifest_dir"

# This owned pure-service smoke needs no Native bootstrap, uv, boto3 or Docker.
smoke-workflows: test-workflows-manifest
	@cargo test --locked -p rustack-cli --test workflows_sdk --features stepfunctions test_should_execute_documented_pure_workflow_in_owned_strict_service -- --ignored --exact --nocapture

# Explicit current-host workflow scope; the full image/platform release gate remains open.
verify-workflows-native: test-workflows-manifest
	@$(MAKE) --no-print-directory -j1 test-stepfunctions-model test-stepfunctions-http test-stepfunctions-http-sdk test-stepfunctions-core test-stepfunctions-runtime test-stepfunctions-bridge-native test-stepfunctions-sdk test-workflows-persistence test-workflows-performance test-workflows-archive test-workflows-compression
	@echo 'Verified native-workflow-only suites; image/ZIP/IaC/reference-platform release acceptance remains required.'

.PHONY: test-workflows-manifest smoke-workflows verify-workflows-native

deny-codegen:
	@cargo deny --manifest-path codegen/Cargo.toml check

.PHONY: audit-codegen deny-codegen

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

# Functional smoke uses exact existing SDK integration tests against a running server.
# Keep the manifest explicit: a removed/renamed test must fail, not silently pass.
SMOKE_ENDPOINT ?= http://127.0.0.1:4566
SERVICE_SMOKE_TESTS := \
	test_object::tests::test_should_put_and_get_object \
	test_object::tests::test_should_delete_object \
	test_list::tests::test_should_list_objects_v2 \
	test_ssm::tests::test_should_put_and_get_parameter \
	test_ssm::tests::test_should_overwrite_parameter \
	test_dynamodb::tests::test_should_put_and_get_item \
	test_dynamodb::tests::test_should_query_by_partition_and_sort_key \
	test_dynamodbstreams::tests::test_should_capture_put_item_as_insert_event \
	test_sqs::tests::test_should_send_and_receive_message \
	test_sqs::tests::test_should_delete_message \
	test_sns::tests::test_should_fan_out_to_sqs \
	test_kinesis::tests::test_should_put_and_get_record \
	test_kms::tests::test_should_encrypt_decrypt_symmetric \
	test_kms::tests::test_should_sign_secp256k1_and_reject_wrong_length_digest \
	test_secretsmanager::tests::test_should_create_and_get_secret_string \
	test_secretsmanager::tests::test_should_put_new_secret_version \
	test_ses::tests::test_should_send_email \
	test_cloudwatch::tests::test_should_put_and_get_metric_statistics \
	test_iam::test_should_create_role_and_get_role \
	test_sts::tests::test_should_get_caller_identity_root \
	test_lambda::tests::test_should_create_and_get_function \
	test_lambda::tests::test_should_invoke_dry_run \
	test_lambda::tests::test_should_enforce_revision_preconditions_through_the_sdk_wire \
	test_lambda::tests::test_should_dry_run_code_without_mutation_and_reject_unsupported_deployment_fields \
	test_lambda::tests::test_should_create_function_with_s3_code \
	test_lambda::tests::test_should_update_function_code_from_s3_and_read_new_metadata \
	test_lambda::tests::test_should_fail_create_function_with_missing_s3_bucket \
	test_apigatewayv2::tests::test_should_execute_mock_integration \
	test_events::tests::test_events_should_deliver_to_sqs_target \
	test_logs::tests::test_logs_should_put_and_get_log_events \
	test_cloudfront_dataplane::test_dp_should_serve_s3_object_via_path_based_url \
	test_cloudfront::test_should_roundtrip_cloudfront_geo_restrictions_and_disable_distribution

smoke-services:
	@curl --fail --silent --show-error --max-time 5 "$(SMOKE_ENDPOINT)/_localstack/health" >/dev/null
	@bash -euo pipefail -c '\
		manifest=$$(mktemp); \
		trap "rm -f \"$$manifest\"" EXIT; \
		cargo test -p rustack-integration --lib -- --ignored --list >"$$manifest"; \
		for test in $(SERVICE_SMOKE_TESTS); do \
			if ! grep -Fxq "$$test: test" "$$manifest"; then \
				echo "Missing smoke test: $$test" >&2; exit 1; \
			fi; \
		done; \
		for test in $(SERVICE_SMOKE_TESTS); do \
			echo "Service smoke: $$test"; \
			S3_ENDPOINT_URL="$(SMOKE_ENDPOINT)" cargo test -p rustack-integration --lib "$$test" -- --ignored --exact --test-threads=1; \
		done'

# Recent security/data/lifecycle fixes have regression tests beyond the live API smoke.
smoke-regressions:
	@cargo test --workspace
	@$(MAKE) test-lambda-invoke-native

# Run regression and functional assertions, then provisioning against the same server.
smoke:
	@$(MAKE) smoke-regressions
	@$(MAKE) smoke-services
	@RUSTACK_ENDPOINT="$(SMOKE_ENDPOINT)" RUSTACK_SKIP_START=1 $(MAKE) pulumi-smoke

integration:
	@cargo test -p rustack-integration -- --ignored

# Shared runtime protocol tests and strict production boundary checks.
test-json-boundaries:
	@cargo test -p rustack-core
	@cargo clippy -p rustack-core --lib --no-deps -- \
		-D warnings -W clippy::pedantic -W clippy::missing_errors_doc \
		-W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used \
		-W clippy::indexing_slicing -W clippy::panic

# Requires a running Lambda HTTP service at SMOKE_ENDPOINT; executes actual AWS SDK IO.
test-lambda-revision-wire:
	@S3_ENDPOINT_URL="$(SMOKE_ENDPOINT)" cargo test -p rustack-integration --lib test_lambda::tests::test_should_enforce_revision_preconditions_through_the_sdk_wire -- --ignored --exact --test-threads=1

.PHONY: test-lambda-revision-wire

test-lambda-validation-wire:
	@S3_ENDPOINT_URL="$(SMOKE_ENDPOINT)" cargo test -p rustack-integration --lib test_lambda::tests::test_should_dry_run_code_without_mutation_and_reject_unsupported_deployment_fields -- --ignored --exact --test-threads=1

.PHONY: test-lambda-validation-wire

# Deployment state, retained image preparation ownership and provider control-plane tests.
test-lambda-deployments:
	@cargo test -p rustack-lambda-core --lib deployment

.PHONY: test-lambda-deployments

test-lambda-runtime:
	@cargo test -p rustack-lambda-runtime
	@cargo clippy -p rustack-lambda-runtime --lib --no-deps -- \
		-D warnings -W clippy::pedantic -W clippy::missing_errors_doc \
		-W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used \
		-W clippy::indexing_slicing -W clippy::panic

.PHONY: test-json-boundaries test-lambda-runtime

# Strict production-library boundary gate; unit, integration and doctest regressions.
test-lambda-boundaries:
	@cargo test -p rustack-auth -p rustack-core -p rustack-lambda-model -p rustack-lambda-http -p rustack-lambda-core -p rustack-lambda-runtime
	@cargo clippy -p rustack-auth -p rustack-core -p rustack-lambda-model -p rustack-lambda-http -p rustack-lambda-core -p rustack-lambda-runtime --lib --no-deps -- \
		-D warnings -W clippy::pedantic -W clippy::missing_errors_doc \
		-W clippy::doc_markdown -W clippy::unwrap_used -W clippy::expect_used \
		-W clippy::indexing_slicing -W clippy::panic

.PHONY: test-lambda-boundaries

# Descriptor lifetime, persisted namespace identity and cleanup ownership.
test-lambda-ownership:
	@cargo test -p rustack-lambda-core --lib docker::ownership::tests
	@cargo test -p rustack-lambda-core --lib test_should_retain_the_directory_lock_through_abandoned_image_owner_cleanup

.PHONY: test-lambda-ownership

# Bounded function output, snapshot/read fairness and joined attachment cleanup.
test-lambda-logs:
	@cargo test -p rustack-lambda-core --lib docker::logs::tests

.PHONY: test-lambda-logs

test-lambda-environment-plans:
	@cargo test -p rustack-lambda-core --lib docker::plan::tests

.PHONY: test-lambda-environment-plans

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

.PHONY: build check test fmt clippy audit deny run publish publish-dry-run release update-submodule integration \
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

.PHONY: smoke smoke-services smoke-regressions

# Explicit real two-container environment lifecycle qualification.
test-lambda-environments-docker: lambda-runtime-image lambda-runtime-fixture-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-core --test docker_environment -- --ignored --test-threads=1

.PHONY: test-lambda-environments-docker

# Pure ownership/failure regressions plus transport failure precedence.
test-lambda-environments:
	@cargo test -p rustack-lambda-core --lib docker::environment::tests
	@cargo test -p rustack-lambda-runtime --lib test_should_preserve_transport_worker_failure_after_a_prior_scope_error

.PHONY: test-lambda-environments

# Warm checkout ownership, bounded FIFO, LRU and result-consumption regressions.
test-lambda-pool:
	@cargo test -p rustack-lambda-core --lib docker::pool::tests

# Includes the real warm-counter/revision-eviction case and owner regressions.
test-lambda-pool-docker: test-lambda-environments-docker

.PHONY: test-lambda-pool test-lambda-pool-docker

# Actual Docker Executor/provider/HTTP composition via pinned ordinary boto3.
test-lambda-executor:
	@cargo test -p rustack-lambda-core --lib docker::executor::tests
	@cargo test -p rustack-lambda-core --lib handler::tests

test-lambda-executor-docker: lambda-runtime-image lambda-runtime-fixture-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-core --test docker_executor -- --ignored --test-threads=1

.PHONY: test-lambda-executor test-lambda-executor-docker

# The bridge lane requires an explicit pre-provisioned operator network.
test-lambda-probes:
	@cargo test -p rustack-lambda-runtime --lib probe::
	@cargo test -p rustack-lambda-core --lib docker::network::tests
	@cargo test -p rustack-lambda-core --lib docker::probe::tests

test-lambda-probes-docker: lambda-runtime-image lambda-runtime-fixture-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" RUSTACK_RUNTIME_NETWORK="$(WORKFLOWS_NETWORK)" cargo test -p rustack-lambda-core --test docker_probe -- --ignored --test-threads=1

.PHONY: test-lambda-probes test-lambda-probes-docker

# Durable container identities, uncertain absence and actual restart recovery.
test-lambda-recovery:
	@cargo test -p rustack-lambda-core --lib docker::ownership::journal::tests
	@cargo test -p rustack-lambda-core --lib docker::recovery::tests
	@cargo test -p rustack-lambda-core --lib docker::environment::tests::test_should_publish_intent_before_create_and_retain_reservation_until_durable_erase

test-lambda-recovery-docker: lambda-runtime-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-core --lib docker::recovery::tests::test_should_reconcile_real_previous_run_containers_from_durable_intents -- --ignored --test-threads=1

.PHONY: test-lambda-recovery test-lambda-recovery-docker

# Durable private tag intents and container-ordered historical alias recovery.
test-lambda-image-recovery:
	@cargo test -p rustack-lambda-core --lib docker::ownership::images::tests
	@cargo test -p rustack-lambda-core --lib docker::recovery::images::tests
	@cargo test -p rustack-lambda-core --lib docker::lease::tests

test-lambda-image-recovery-docker: lambda-runtime-image
	@RUSTACK_RUNTIME_DOCKER_SOCKET="$(WORKFLOWS_DOCKER_SOCKET)" RUSTACK_RUNTIME_ARCH="$(WORKFLOWS_ARCH)" RUSTACK_RUNTIME_IMAGE="$(LAMBDA_RUNTIME_IMAGE)" cargo test -p rustack-lambda-core --lib docker::recovery::images::tests::test_should_recover_real_private_alias_across_directory_reacquisition -- --ignored --test-threads=1

.PHONY: test-lambda-image-recovery test-lambda-image-recovery-docker

# Delivery clocks, protocol-v2 correlation and actual Native process retirement.
test-lambda-timeouts:
	@cargo test -p rustack-lambda-runtime --lib
	@cargo test -p rustack-lambda-echo-bootstrap --test hardening

# Captured handler-timeout/retirement/replacement through the real image pool.
test-lambda-timeouts-docker: test-lambda-environments-docker

.PHONY: test-lambda-timeouts test-lambda-timeouts-docker

# Invocation log capabilities/receipts and independent diagnostic capture.
test-lambda-log-windows:
	@cargo test -p rustack-lambda-core --lib docker::logs::tests

# Includes requested/silent/uncaptured warm-call log isolation on actual images.
test-lambda-log-windows-docker: test-lambda-environments-docker

.PHONY: test-lambda-log-windows test-lambda-log-windows-docker
