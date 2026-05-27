#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE_DIR="${PULUMI_EXAMPLE_DIR:-$ROOT_DIR/examples/pulumi/rustack-target}"
ENDPOINT="${RUSTACK_ENDPOINT:-http://127.0.0.1:4566}"
STACK_NAME="${PULUMI_STACK:-rustack-smoke}"
RUSTACK_LOG="${RUSTACK_LOG:-/tmp/rustack-pulumi-smoke.log}"

SERVER_PID=""
STATE_DIR=""
CREATED_STATE_DIR="0"
STACK_READY="0"
UP_SUCCEEDED="0"

is_running() {
  kill -0 "$1" >/dev/null 2>&1
}

wait_for_exit() {
  local pid="$1"
  local attempts="$2"
  for _ in $(seq 1 "$attempts"); do
    if ! is_running "$pid"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_rustack() {
  local pid="$1"
  if ! is_running "$pid"; then
    return 0
  fi

  kill -INT "$pid" >/dev/null 2>&1
  if wait_for_exit "$pid" 50; then
    wait "$pid" >/dev/null 2>&1
    return 0
  fi

  kill -TERM "$pid" >/dev/null 2>&1
  if wait_for_exit "$pid" 20; then
    wait "$pid" >/dev/null 2>&1
    return 0
  fi

  kill -KILL "$pid" >/dev/null 2>&1
  wait "$pid" >/dev/null 2>&1
}

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$STACK_READY" == "1" && "${PULUMI_KEEP_STACK:-0}" != "1" ]]; then
    (
      cd "$EXAMPLE_DIR" || exit 0
      if [[ "$UP_SUCCEEDED" == "1" && "${PULUMI_SKIP_DESTROY:-0}" != "1" ]]; then
        echo "Destroying Pulumi smoke resources (SQS delete confirmation can take around 2 minutes)"
        pulumi destroy --stack "$STACK_NAME" --yes --skip-preview >/dev/null 2>&1
      fi
      echo "Removing Pulumi smoke stack"
      pulumi stack rm "$STACK_NAME" --yes >/dev/null 2>&1
      rm -f "Pulumi.$STACK_NAME.yaml"
    )
  fi

  if [[ -n "$SERVER_PID" ]]; then
    if [[ "${RUSTACK_KEEP_RUNNING:-0}" == "1" ]]; then
      if [[ -n "${RUSTACK_PID_FILE:-}" ]]; then
        printf '%s\n' "$SERVER_PID" >"$RUSTACK_PID_FILE"
      fi
    else
      stop_rustack "$SERVER_PID"
    fi
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

now_ms() {
  node -e 'process.stdout.write(String(Date.now()))'
}

rustack_binary_path() {
  local target_dir
  target_dir="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
  if [[ -z "$target_dir" ]]; then
    target_dir="$ROOT_DIR/target"
  fi
  printf '%s/release/rustack' "$target_dir"
}

wait_for_rustack() {
  local started_ms="$1"
  for _ in $(seq 1 1200); do
    if curl -sf --max-time 1 "$(health_url)" >/dev/null 2>&1; then
      if [[ -n "${RUSTACK_READY_MS_FILE:-}" ]]; then
        local ready_ms
        ready_ms="$(now_ms)"
        printf '%s\n' "$((ready_ms - started_ms))" >"$RUSTACK_READY_MS_FILE"
      fi
      return 0
    fi
    sleep 0.05
  done

  echo "Rustack did not become healthy at $(health_url)" >&2
  if [[ -f "$RUSTACK_LOG" ]]; then
    tail -n 120 "$RUSTACK_LOG" >&2
  fi
  exit 1
}

start_rustack_if_needed() {
  if curl -sf --max-time 1 "$(health_url)" >/dev/null 2>&1; then
    echo "Using existing Rustack at $ENDPOINT"
    if [[ -n "${RUSTACK_READY_MS_FILE:-}" ]]; then
      printf '0\n' >"$RUSTACK_READY_MS_FILE"
    fi
    return
  fi

  if [[ "${RUSTACK_SKIP_START:-0}" == "1" ]]; then
    echo "Rustack is not healthy at $ENDPOINT and RUSTACK_SKIP_START=1" >&2
    exit 1
  fi

  local listen_addr
  if [[ "$ENDPOINT" =~ ^http://(127\.0\.0\.1|localhost):([0-9]+)$ ]]; then
    listen_addr="${BASH_REMATCH[1]}:${BASH_REMATCH[2]}"
  else
    echo "Cannot auto-start Rustack for endpoint: $ENDPOINT" >&2
    echo "Start Rustack yourself or use a local http://127.0.0.1:<port> endpoint." >&2
    exit 1
  fi

  echo "Building Rustack release binary"
  cargo build --release -p rustack-cli

  echo "Starting Rustack for Pulumi smoke test"
  local rustack_bin
  rustack_bin="$(rustack_binary_path)"
  if [[ ! -x "$rustack_bin" ]]; then
    echo "Rustack binary was not built at $rustack_bin" >&2
    exit 1
  fi
  local rustack_args=()
  if [[ -n "${RUSTACK_EXTRA_ARGS:-}" ]]; then
    # shellcheck disable=SC2206
    rustack_args=(${RUSTACK_EXTRA_ARGS})
  fi
  local started_ms
  started_ms="$(now_ms)"
  : >"$RUSTACK_LOG"
  if [[ "${#rustack_args[@]}" -eq 0 ]]; then
    GATEWAY_LISTEN="$listen_addr" \
      LOG_LEVEL=warn \
      "$rustack_bin" >"$RUSTACK_LOG" 2>&1 &
  else
    GATEWAY_LISTEN="$listen_addr" \
      LOG_LEVEL=warn \
      "$rustack_bin" "${rustack_args[@]}" >"$RUSTACK_LOG" 2>&1 &
  fi
  SERVER_PID=$!
  if [[ -n "${RUSTACK_PID_FILE:-}" ]]; then
    printf '%s\n' "$SERVER_PID" >"$RUSTACK_PID_FILE"
  fi
  wait_for_rustack "$started_ms"
}

need_cmd cargo
need_cmd curl
need_cmd node
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
if ! pulumi stack select "$STACK_NAME" >/dev/null 2>&1; then
  pulumi stack init "$STACK_NAME"
fi
STACK_READY="1"

pulumi config set endpoint "$ENDPOINT" --stack "$STACK_NAME"
pulumi config set region "$AWS_DEFAULT_REGION" --stack "$STACK_NAME"
pulumi config set accessKey "$AWS_ACCESS_KEY_ID" --stack "$STACK_NAME"
pulumi config set secretKey "$AWS_SECRET_ACCESS_KEY" --secret --stack "$STACK_NAME"

case "${PULUMI_OPERATION:-up}" in
  up)
    pulumi up --stack "$STACK_NAME" --yes --skip-preview
    UP_SUCCEEDED="1"
    ;;
  refresh)
    pulumi refresh --stack "$STACK_NAME" --yes --skip-preview
    UP_SUCCEEDED="1"
    ;;
  *)
    echo "unsupported PULUMI_OPERATION: ${PULUMI_OPERATION:-up}" >&2
    exit 1
    ;;
esac
pulumi stack output --stack "$STACK_NAME" --json
