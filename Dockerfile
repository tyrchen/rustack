# syntax=docker/dockerfile:1

# Match the repository toolchain pin and add musl targets to that toolchain.
FROM rust:1.99.1-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
COPY Cargo.toml Cargo.lock ./
COPY patches/ patches/
COPY crates/ crates/
COPY apps/ apps/
COPY tests/ tests/

ARG TARGETARCH
ARG CARGO_BUILD_JOBS=2
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    case "$CARGO_BUILD_JOBS" in 1|2|3|4|5|6|7|8) ;; \
      *) echo "CARGO_BUILD_JOBS must be between 1 and 8" >&2; exit 1 ;; esac && \
    case "$TARGETARCH" in \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      amd64) RUST_TARGET=x86_64-unknown-linux-musl ;; \
      *) echo "unsupported target architecture" >&2; exit 1 ;; \
    esac && \
    rustc --version && cargo --version && \
    cargo build --locked --release --target "$RUST_TARGET" -p rustack-cli --bin rustack --features stepfunctions && \
    cp "/src/target/$RUST_TARGET/release/rustack" /rustack
RUN mkdir -p /runtime/tmp /runtime/data /runtime/run/rustack \
    && chmod 1777 /runtime/tmp && chmod 0700 /runtime/data \
    && chown 65532:65532 /runtime/tmp /runtime/data \
    && touch /runtime/run/rustack/docker.sock \
    && chmod 0555 /runtime/run /runtime/run/rustack \
    && chmod 0444 /runtime/run/rustack/docker.sock

FROM scratch
COPY --from=builder /rustack /rustack
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /runtime/ /
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
