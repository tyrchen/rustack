# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Builder stage: compile a fully static binary with musl
# ---------------------------------------------------------------------------
FROM rust:1.94-slim AS builder

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the toolchain file first so rustup installs the correct version,
# then add musl targets to the resolved toolchain.
COPY rust-toolchain.toml ./
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

# Copy manifests to cache dependency compilation.
COPY Cargo.toml Cargo.lock ./
COPY apps/rustack/Cargo.toml apps/rustack/Cargo.toml
COPY crates/rustack-auth/Cargo.toml crates/rustack-auth/Cargo.toml
COPY crates/rustack-core/Cargo.toml crates/rustack-core/Cargo.toml
COPY crates/rustack-s3-core/Cargo.toml crates/rustack-s3-core/Cargo.toml
COPY crates/rustack-s3-http/Cargo.toml crates/rustack-s3-http/Cargo.toml
COPY crates/rustack-s3-model/Cargo.toml crates/rustack-s3-model/Cargo.toml
COPY crates/rustack-s3-xml/Cargo.toml crates/rustack-s3-xml/Cargo.toml
COPY crates/rustack-dynamodb-model/Cargo.toml crates/rustack-dynamodb-model/Cargo.toml
COPY crates/rustack-dynamodb-http/Cargo.toml crates/rustack-dynamodb-http/Cargo.toml
COPY crates/rustack-dynamodb-core/Cargo.toml crates/rustack-dynamodb-core/Cargo.toml
COPY crates/rustack-dynamodbstreams-model/Cargo.toml crates/rustack-dynamodbstreams-model/Cargo.toml
COPY crates/rustack-dynamodbstreams-http/Cargo.toml crates/rustack-dynamodbstreams-http/Cargo.toml
COPY crates/rustack-dynamodbstreams-core/Cargo.toml crates/rustack-dynamodbstreams-core/Cargo.toml
COPY crates/rustack-sqs-model/Cargo.toml crates/rustack-sqs-model/Cargo.toml
COPY crates/rustack-sqs-http/Cargo.toml crates/rustack-sqs-http/Cargo.toml
COPY crates/rustack-sqs-core/Cargo.toml crates/rustack-sqs-core/Cargo.toml
COPY crates/rustack-ssm-model/Cargo.toml crates/rustack-ssm-model/Cargo.toml
COPY crates/rustack-ssm-http/Cargo.toml crates/rustack-ssm-http/Cargo.toml
COPY crates/rustack-ssm-core/Cargo.toml crates/rustack-ssm-core/Cargo.toml
COPY crates/rustack-sns-model/Cargo.toml crates/rustack-sns-model/Cargo.toml
COPY crates/rustack-sns-http/Cargo.toml crates/rustack-sns-http/Cargo.toml
COPY crates/rustack-sns-core/Cargo.toml crates/rustack-sns-core/Cargo.toml
COPY crates/rustack-lambda-model/Cargo.toml crates/rustack-lambda-model/Cargo.toml
COPY crates/rustack-lambda-http/Cargo.toml crates/rustack-lambda-http/Cargo.toml
COPY crates/rustack-lambda-core/Cargo.toml crates/rustack-lambda-core/Cargo.toml
COPY crates/rustack-events-model/Cargo.toml crates/rustack-events-model/Cargo.toml
COPY crates/rustack-events-http/Cargo.toml crates/rustack-events-http/Cargo.toml
COPY crates/rustack-events-core/Cargo.toml crates/rustack-events-core/Cargo.toml
COPY crates/rustack-logs-model/Cargo.toml crates/rustack-logs-model/Cargo.toml
COPY crates/rustack-logs-http/Cargo.toml crates/rustack-logs-http/Cargo.toml
COPY crates/rustack-logs-core/Cargo.toml crates/rustack-logs-core/Cargo.toml
COPY crates/rustack-kms-model/Cargo.toml crates/rustack-kms-model/Cargo.toml
COPY crates/rustack-kms-http/Cargo.toml crates/rustack-kms-http/Cargo.toml
COPY crates/rustack-kms-core/Cargo.toml crates/rustack-kms-core/Cargo.toml
COPY crates/rustack-kinesis-model/Cargo.toml crates/rustack-kinesis-model/Cargo.toml
COPY crates/rustack-kinesis-http/Cargo.toml crates/rustack-kinesis-http/Cargo.toml
COPY crates/rustack-kinesis-core/Cargo.toml crates/rustack-kinesis-core/Cargo.toml
COPY crates/rustack-secretsmanager-model/Cargo.toml crates/rustack-secretsmanager-model/Cargo.toml
COPY crates/rustack-secretsmanager-http/Cargo.toml crates/rustack-secretsmanager-http/Cargo.toml
COPY crates/rustack-secretsmanager-core/Cargo.toml crates/rustack-secretsmanager-core/Cargo.toml
COPY crates/rustack-ses-model/Cargo.toml crates/rustack-ses-model/Cargo.toml
COPY crates/rustack-ses-http/Cargo.toml crates/rustack-ses-http/Cargo.toml
COPY crates/rustack-ses-core/Cargo.toml crates/rustack-ses-core/Cargo.toml
COPY crates/rustack-apigatewayv2-model/Cargo.toml crates/rustack-apigatewayv2-model/Cargo.toml
COPY crates/rustack-apigatewayv2-http/Cargo.toml crates/rustack-apigatewayv2-http/Cargo.toml
COPY crates/rustack-apigatewayv2-core/Cargo.toml crates/rustack-apigatewayv2-core/Cargo.toml
COPY crates/rustack-cloudwatch-model/Cargo.toml crates/rustack-cloudwatch-model/Cargo.toml
COPY crates/rustack-cloudwatch-http/Cargo.toml crates/rustack-cloudwatch-http/Cargo.toml
COPY crates/rustack-cloudwatch-core/Cargo.toml crates/rustack-cloudwatch-core/Cargo.toml
COPY crates/rustack-iam-model/Cargo.toml crates/rustack-iam-model/Cargo.toml
COPY crates/rustack-iam-http/Cargo.toml crates/rustack-iam-http/Cargo.toml
COPY crates/rustack-iam-core/Cargo.toml crates/rustack-iam-core/Cargo.toml
COPY crates/rustack-sts-model/Cargo.toml crates/rustack-sts-model/Cargo.toml
COPY crates/rustack-sts-http/Cargo.toml crates/rustack-sts-http/Cargo.toml
COPY crates/rustack-sts-core/Cargo.toml crates/rustack-sts-core/Cargo.toml
COPY tests/integration/Cargo.toml tests/integration/Cargo.toml

# Create stub sources so cargo can resolve the workspace.
RUN mkdir -p apps/rustack/src && echo 'fn main() {}' > apps/rustack/src/main.rs \
    && mkdir -p crates/rustack-auth/src && echo '//! stub' > crates/rustack-auth/src/lib.rs \
    && mkdir -p crates/rustack-core/src && echo '//! stub' > crates/rustack-core/src/lib.rs \
    && mkdir -p crates/rustack-s3-core/src && echo '//! stub' > crates/rustack-s3-core/src/lib.rs \
    && mkdir -p crates/rustack-s3-http/src && echo '//! stub' > crates/rustack-s3-http/src/lib.rs \
    && mkdir -p crates/rustack-s3-model/src && echo '//! stub' > crates/rustack-s3-model/src/lib.rs \
    && mkdir -p crates/rustack-s3-xml/src && echo '//! stub' > crates/rustack-s3-xml/src/lib.rs \
    && mkdir -p crates/rustack-dynamodb-model/src && echo '//! stub' > crates/rustack-dynamodb-model/src/lib.rs \
    && mkdir -p crates/rustack-dynamodb-http/src && echo '//! stub' > crates/rustack-dynamodb-http/src/lib.rs \
    && mkdir -p crates/rustack-dynamodb-core/src && echo '//! stub' > crates/rustack-dynamodb-core/src/lib.rs \
    && mkdir -p crates/rustack-dynamodbstreams-model/src && echo '//! stub' > crates/rustack-dynamodbstreams-model/src/lib.rs \
    && mkdir -p crates/rustack-dynamodbstreams-http/src && echo '//! stub' > crates/rustack-dynamodbstreams-http/src/lib.rs \
    && mkdir -p crates/rustack-dynamodbstreams-core/src && echo '//! stub' > crates/rustack-dynamodbstreams-core/src/lib.rs \
    && mkdir -p crates/rustack-sqs-model/src && echo '//! stub' > crates/rustack-sqs-model/src/lib.rs \
    && mkdir -p crates/rustack-sqs-http/src && echo '//! stub' > crates/rustack-sqs-http/src/lib.rs \
    && mkdir -p crates/rustack-sqs-core/src && echo '//! stub' > crates/rustack-sqs-core/src/lib.rs \
    && mkdir -p crates/rustack-ssm-model/src && echo '//! stub' > crates/rustack-ssm-model/src/lib.rs \
    && mkdir -p crates/rustack-ssm-http/src && echo '//! stub' > crates/rustack-ssm-http/src/lib.rs \
    && mkdir -p crates/rustack-ssm-core/src && echo '//! stub' > crates/rustack-ssm-core/src/lib.rs \
    && mkdir -p crates/rustack-sns-model/src && echo '//! stub' > crates/rustack-sns-model/src/lib.rs \
    && mkdir -p crates/rustack-sns-http/src && echo '//! stub' > crates/rustack-sns-http/src/lib.rs \
    && mkdir -p crates/rustack-sns-core/src && echo '//! stub' > crates/rustack-sns-core/src/lib.rs \
    && mkdir -p crates/rustack-lambda-model/src && echo '//! stub' > crates/rustack-lambda-model/src/lib.rs \
    && mkdir -p crates/rustack-lambda-http/src && echo '//! stub' > crates/rustack-lambda-http/src/lib.rs \
    && mkdir -p crates/rustack-lambda-core/src && echo '//! stub' > crates/rustack-lambda-core/src/lib.rs \
    && mkdir -p crates/rustack-events-model/src && echo '//! stub' > crates/rustack-events-model/src/lib.rs \
    && mkdir -p crates/rustack-events-http/src && echo '//! stub' > crates/rustack-events-http/src/lib.rs \
    && mkdir -p crates/rustack-events-core/src && echo '//! stub' > crates/rustack-events-core/src/lib.rs \
    && mkdir -p crates/rustack-logs-model/src && echo '//! stub' > crates/rustack-logs-model/src/lib.rs \
    && mkdir -p crates/rustack-logs-http/src && echo '//! stub' > crates/rustack-logs-http/src/lib.rs \
    && mkdir -p crates/rustack-logs-core/src && echo '//! stub' > crates/rustack-logs-core/src/lib.rs \
    && mkdir -p crates/rustack-kms-model/src && echo '//! stub' > crates/rustack-kms-model/src/lib.rs \
    && mkdir -p crates/rustack-kms-http/src && echo '//! stub' > crates/rustack-kms-http/src/lib.rs \
    && mkdir -p crates/rustack-kms-core/src && echo '//! stub' > crates/rustack-kms-core/src/lib.rs \
    && mkdir -p crates/rustack-kinesis-model/src && echo '//! stub' > crates/rustack-kinesis-model/src/lib.rs \
    && mkdir -p crates/rustack-kinesis-http/src && echo '//! stub' > crates/rustack-kinesis-http/src/lib.rs \
    && mkdir -p crates/rustack-kinesis-core/src && echo '//! stub' > crates/rustack-kinesis-core/src/lib.rs \
    && mkdir -p crates/rustack-secretsmanager-model/src && echo '//! stub' > crates/rustack-secretsmanager-model/src/lib.rs \
    && mkdir -p crates/rustack-secretsmanager-http/src && echo '//! stub' > crates/rustack-secretsmanager-http/src/lib.rs \
    && mkdir -p crates/rustack-secretsmanager-core/src && echo '//! stub' > crates/rustack-secretsmanager-core/src/lib.rs \
    && mkdir -p crates/rustack-ses-model/src && echo '//! stub' > crates/rustack-ses-model/src/lib.rs \
    && mkdir -p crates/rustack-ses-http/src && echo '//! stub' > crates/rustack-ses-http/src/lib.rs \
    && mkdir -p crates/rustack-ses-core/src && echo '//! stub' > crates/rustack-ses-core/src/lib.rs \
    && mkdir -p crates/rustack-apigatewayv2-model/src && echo '//! stub' > crates/rustack-apigatewayv2-model/src/lib.rs \
    && mkdir -p crates/rustack-apigatewayv2-http/src && echo '//! stub' > crates/rustack-apigatewayv2-http/src/lib.rs \
    && mkdir -p crates/rustack-apigatewayv2-core/src && echo '//! stub' > crates/rustack-apigatewayv2-core/src/lib.rs \
    && mkdir -p crates/rustack-cloudwatch-model/src && echo '//! stub' > crates/rustack-cloudwatch-model/src/lib.rs \
    && mkdir -p crates/rustack-cloudwatch-http/src && echo '//! stub' > crates/rustack-cloudwatch-http/src/lib.rs \
    && mkdir -p crates/rustack-cloudwatch-core/src && echo '//! stub' > crates/rustack-cloudwatch-core/src/lib.rs \
    && mkdir -p crates/rustack-iam-model/src && echo '//! stub' > crates/rustack-iam-model/src/lib.rs \
    && mkdir -p crates/rustack-iam-http/src && echo '//! stub' > crates/rustack-iam-http/src/lib.rs \
    && mkdir -p crates/rustack-iam-core/src && echo '//! stub' > crates/rustack-iam-core/src/lib.rs \
    && mkdir -p crates/rustack-sts-model/src && echo '//! stub' > crates/rustack-sts-model/src/lib.rs \
    && mkdir -p crates/rustack-sts-http/src && echo '//! stub' > crates/rustack-sts-http/src/lib.rs \
    && mkdir -p crates/rustack-sts-core/src && echo '//! stub' > crates/rustack-sts-core/src/lib.rs \
    && mkdir -p tests/integration/src && echo '//! stub' > tests/integration/src/lib.rs

# Pre-build dependencies (cached layer).
ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    if [ "$TARGETARCH" = "arm64" ]; then \
      RUST_TARGET=aarch64-unknown-linux-musl; \
    else \
      RUST_TARGET=x86_64-unknown-linux-musl; \
    fi && \
    cargo build --release --target "$RUST_TARGET" -p rustack-cli 2>/dev/null || true

# Copy real source code.
COPY crates/ crates/
COPY apps/ apps/
COPY tests/ tests/

# Touch source files so cargo knows they changed.
RUN find crates/ apps/ tests/ -name '*.rs' -exec touch {} +

# Build the actual binary.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    if [ "$TARGETARCH" = "arm64" ]; then \
      RUST_TARGET=aarch64-unknown-linux-musl; \
    else \
      RUST_TARGET=x86_64-unknown-linux-musl; \
    fi && \
    cargo build --release --target "$RUST_TARGET" -p rustack-cli && \
    cp "/src/target/$RUST_TARGET/release/rustack" /rustack

# ---------------------------------------------------------------------------
# Runtime stage: scratch image with just the binary
# ---------------------------------------------------------------------------
FROM scratch

COPY --from=builder /rustack /rustack
COPY --from=builder /tmp /tmp

ENV GATEWAY_LISTEN=0.0.0.0:4566
ENV LOG_LEVEL=info
ENV SERVICES=

EXPOSE 4566

HEALTHCHECK --interval=2s --timeout=3s --start-period=1s --retries=3 \
    CMD ["/rustack", "--health-check"]

ENTRYPOINT ["/rustack"]
