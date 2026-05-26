#!/bin/bash
# Publish crates in dependency order with delay to avoid rate limiting.
# Re-running the script is safe: already-published crate versions are skipped.

set -e

DELAY=${PUBLISH_DELAY:-10}  # seconds between publishes to avoid 429 rate limit
MAX_ATTEMPTS=${PUBLISH_MAX_ATTEMPTS:-5}
RETRY_DELAY=${PUBLISH_RETRY_DELAY:-30}

publish_crate() {
    local crate_name="$1"
    local attempt=1

    while true; do
        echo "=== Publishing $crate_name (attempt ${attempt}/${MAX_ATTEMPTS}) ==="
        output=$(cargo publish -p "$crate_name" 2>&1) && {
            echo "  ✓ $crate_name published successfully"
            echo "  Waiting ${DELAY}s before next publish..."
            sleep $DELAY
            return
        }

        if echo "$output" | grep -q "already exists"; then
            echo "  ⏭ $crate_name already published, skipping"
            return
        fi

        if (( attempt < MAX_ATTEMPTS )) && echo "$output" | grep -Eq "no matching package named|failed to select a version|status code 429|rate limit"; then
            echo "$output"
            echo "  Retryable publish failure; waiting ${RETRY_DELAY}s before retry..."
            sleep $RETRY_DELAY
            attempt=$((attempt + 1))
            continue
        fi

        echo "$output"
        echo "  ✗ $crate_name failed"
        exit 1
    done
}

# Layer 0: Shared base crates.
BASE_CRATES=(
    rustack-core
    rustack-auth
)

# Layer 1: Model crates (no internal deps, except dynamodbstreams-model).
MODEL_CRATES=(
    rustack-apigatewayv2-model
    rustack-cloudfront-model
    rustack-cloudwatch-model
    rustack-dynamodb-model
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

# Layer 2: CloudFront core (the CloudFront HTTP layer depends on it).
CLOUDFRONT_CORE_CRATES=(
    rustack-cloudfront-core
)

# Layer 3: S3 XML (depends on s3-model)
XML_CRATES=(
    rustack-s3-xml
)

# Layer 4: HTTP crates (depend on model + auth)
# s3-http also depends on s3-xml
# cloudfront-http also depends on cloudfront-core
HTTP_CRATES=(
    rustack-apigatewayv2-http
    rustack-cloudfront-http
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

# Layer 5: Core crates (depend on model + http + rustack-core)
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

# Layer 6: Data plane crates (depend on core crates)
DATAPLANE_CRATES=(
    rustack-cloudfront-dataplane
)

# Layer 7: App
APP_CRATES=(
    rustack-cli
)

echo "Publishing base crates..."
for crate in "${BASE_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing model crates..."
for crate in "${MODEL_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing CloudFront core crates..."
for crate in "${CLOUDFRONT_CORE_CRATES[@]}"; do
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

echo "Publishing data plane crates..."
for crate in "${DATAPLANE_CRATES[@]}"; do
    publish_crate "$crate"
done

echo "Publishing app crates..."
for crate in "${APP_CRATES[@]}"; do
    publish_crate "$crate"
done

echo ""
echo "=== All crates published successfully! ==="
