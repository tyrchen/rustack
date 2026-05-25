#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE_DIR="$ROOT_DIR/examples/pulumi/rustack-target"
ENDPOINT="${RUSTACK_ENDPOINT:-http://127.0.0.1:4566}"
STACK_NAME="${PULUMI_STACK:-rustack-smoke}"
RUSTACK_LOG="${RUSTACK_LOG:-/tmp/rustack-pulumi-smoke.log}"

SERVER_PID=""
STATE_DIR=""
CREATED_STATE_DIR="0"
STACK_READY="0"
UP_SUCCEEDED="0"

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$STACK_READY" == "1" && "$UP_SUCCEEDED" == "1" ]]; then
    (
      cd "$EXAMPLE_DIR" || exit 0
      echo "Destroying Pulumi smoke resources"
      pulumi destroy --stack "$STACK_NAME" --yes --skip-preview >/dev/null 2>&1
      echo "Removing Pulumi smoke stack"
      pulumi stack rm "$STACK_NAME" --yes >/dev/null 2>&1
    )
  fi

  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1
    wait "$SERVER_PID" >/dev/null 2>&1
  fi

  if [[ "$CREATED_STATE_DIR" == "1" && -n "$STATE_DIR" ]]; then
    rm -rf "$STATE_DIR"
  fi

  exit "$exit_code"
}
trap cleanup EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

health_url() {
  printf '%s/_localstack/health' "${ENDPOINT%/}"
}

wait_for_rustack() {
  for _ in $(seq 1 60); do
    if curl -sf "$(health_url)" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  echo "Rustack did not become healthy at $(health_url)" >&2
  if [[ -f "$RUSTACK_LOG" ]]; then
    tail -n 120 "$RUSTACK_LOG" >&2
  fi
  exit 1
}

start_rustack_if_needed() {
  if curl -sf "$(health_url)" >/dev/null 2>&1; then
    echo "Using existing Rustack at $ENDPOINT"
    return
  fi

  if [[ "${RUSTACK_SKIP_START:-0}" == "1" ]]; then
    echo "Rustack is not healthy at $ENDPOINT and RUSTACK_SKIP_START=1" >&2
    exit 1
  fi

  case "$ENDPOINT" in
    http://127.0.0.1:4566|http://localhost:4566) ;;
    *)
      echo "Cannot auto-start Rustack for non-default endpoint: $ENDPOINT" >&2
      echo "Start Rustack yourself or use RUSTACK_ENDPOINT=http://127.0.0.1:4566." >&2
      exit 1
      ;;
  esac

  echo "Building Rustack release binary"
  cargo build --release -p rustack-cli

  echo "Starting Rustack for Pulumi smoke test"
  GATEWAY_LISTEN=127.0.0.1:4566 \
    LOG_LEVEL=warn \
    cargo run --release -p rustack-cli >"$RUSTACK_LOG" 2>&1 &
  SERVER_PID=$!
  wait_for_rustack
}

need_cmd cargo
need_cmd curl
need_cmd npm
need_cmd pulumi

start_rustack_if_needed

if [[ -z "${PULUMI_STATE_DIR:-}" ]]; then
  STATE_DIR="$(mktemp -d)"
  CREATED_STATE_DIR="1"
else
  STATE_DIR="$PULUMI_STATE_DIR"
  mkdir -p "$STATE_DIR"
fi

export PULUMI_HOME="${PULUMI_HOME:-$STATE_DIR/home}"
export PULUMI_CONFIG_PASSPHRASE="${PULUMI_CONFIG_PASSPHRASE:-}"
export AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID:-AKIAIOSFODNN7EXAMPLE}"
export AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY:-wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY}"
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-us-east-1}"
export RUSTACK_ENDPOINT="$ENDPOINT"

mkdir -p "$PULUMI_HOME" "$STATE_DIR/state"

cd "$EXAMPLE_DIR"

if [[ -f package-lock.json ]]; then
  npm ci
else
  npm install
fi

npm run typecheck

pulumi login "file://$STATE_DIR/state"
pulumi stack init "$STACK_NAME"
STACK_READY="1"

pulumi config set endpoint "$ENDPOINT" --stack "$STACK_NAME"
pulumi config set region "$AWS_DEFAULT_REGION" --stack "$STACK_NAME"
pulumi config set accessKey "$AWS_ACCESS_KEY_ID" --stack "$STACK_NAME"
pulumi config set secretKey "$AWS_SECRET_ACCESS_KEY" --secret --stack "$STACK_NAME"

pulumi up --stack "$STACK_NAME" --yes --skip-preview
UP_SUCCEEDED="1"
pulumi stack output --stack "$STACK_NAME" --json
