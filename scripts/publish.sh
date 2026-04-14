#!/bin/bash
# Publish unpublished crates in dependency order with delay to avoid rate limiting
# Already published: rustack-core, rustack-auth, rustack-apigatewayv2-model,
#   rustack-cloudwatch-model, rustack-dynamodb-model, rustack-integration

set -e

DELAY=5  # seconds between publishes to avoid 429 rate limit

publish_crate() {
    local crate_name="$1"
    echo "=== Publishing $crate_name ==="
    output=$(cargo publish -p "$crate_name" 2>&1) && {
        echo "  ✓ $crate_name published successfully"
        echo "  Waiting ${DELAY}s before next publish..."
        sleep $DELAY
    } || {
        if echo "$output" | grep -q "already exists"; then
            echo "  ⏭ $crate_name already published, skipping"
        else
            echo "$output"
            echo "  ✗ $crate_name failed"
            exit 1
        fi
    }
}

# Layer 1: Model crates (no internal deps, or deps already published)
# dynamodbstreams-model depends on dynamodb-model (already published)
MODEL_CRATES=(
    rustack-events-model
    rustack-iam-model
    rustack-kinesis-model
    rustack-kms-model
    rustack-lambda-model
    rustack-logs-model
    rustack-s3-model
    rustack-secretsmanager-model
    rustack-ses-model
    rustack-sns-model
    rustack-sqs-model
    rustack-ssm-model
    rustack-sts-model
    rustack-dynamodbstreams-model
)

# Layer 2: S3 XML (depends on s3-model)
XML_CRATES=(
    rustack-s3-xml
)

# Layer 3: HTTP crates (depend on model + auth)
# s3-http also depends on s3-xml
HTTP_CRATES=(
    rustack-apigatewayv2-http
    rustack-cloudwatch-http
    rustack-dynamodb-http
    rustack-events-http
    rustack-iam-http
    rustack-kinesis-http
    rustack-kms-http
    rustack-lambda-http
    rustack-logs-http
    rustack-s3-http
    rustack-secretsmanager-http
    rustack-ses-http
    rustack-sns-http
    rustack-sqs-http
    rustack-ssm-http
    rustack-sts-http
    rustack-dynamodbstreams-http
)

# Layer 4: Core crates (depend on model + http + rustack-core)
CORE_CRATES=(
    rustack-apigatewayv2-core
    rustack-cloudwatch-core
    rustack-dynamodb-core
    rustack-events-core
    rustack-iam-core
    rustack-kinesis-core
    rustack-kms-core
    rustack-lambda-core
    rustack-logs-core
    rustack-s3-core
    rustack-secretsmanager-core
    rustack-ses-core
    rustack-sns-core
    rustack-sqs-core
    rustack-ssm-core
    rustack-sts-core
    rustack-dynamodbstreams-core
)

# Layer 5: App
APP_CRATES=(
    rustack-cli
)

echo "Publishing model crates..."
for crate in "${MODEL_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing XML crates..."
for crate in "${XML_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing HTTP crates..."
for crate in "${HTTP_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing core crates..."
for crate in "${CORE_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing app crates..."
for crate in "${APP_CRATES[@]}"; do
    publish_crate "$crate"
done

echo ""
echo "=== All crates published successfully! ==="
