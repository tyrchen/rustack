# syntax=docker/dockerfile:1

# Retain the repository-selected toolchain without depending on a local
# rust-toolchain.toml (maintainers may delete it). rust:1.95 matches the
# repository pin; rustup adds the musl targets to that toolchain.
FROM rust:1.95-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY apps/ apps/
COPY tests/ tests/

ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    case "$TARGETARCH" in \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      amd64) RUST_TARGET=x86_64-unknown-linux-musl ;; \
      *) echo "unsupported target architecture" >&2; exit 1 ;; \
    esac && \
    cargo build --locked --release --target "$RUST_TARGET" -p rustack-cli --bin rustack && \
    cp "/src/target/$RUST_TARGET/release/rustack" /rustack
RUN mkdir -p /runtime/tmp /runtime/data && chmod 1777 /runtime/tmp

FROM scratch
COPY --from=builder /rustack /rustack
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder --chown=65532:65532 /runtime/tmp /tmp
COPY --from=builder --chown=65532:65532 /runtime/data /data
WORKDIR /data
USER 65532:65532
# Containers opt in to all-interface binding; bare binaries default to loopback.
ENV GATEWAY_LISTEN=0.0.0.0:4566
ENV LOG_LEVEL=info
ENV SERVICES=
EXPOSE 4566
HEALTHCHECK --interval=2s --timeout=4s --start-period=1s --retries=3 \
    CMD ["/rustack", "--health-check"]
ENTRYPOINT ["/rustack"]
