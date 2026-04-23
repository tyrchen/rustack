#!/usr/bin/env bash
# Download ScyllaDB Alternator test files for DynamoDB compatibility testing.
#
# Usage:
#   ./setup.sh [--all]
#
# Without --all, downloads only the P0 (core) test files.
# With --all, downloads all available test files.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENDOR_DIR="${SCRIPT_DIR}/vendor"
SCYLLA_BASE="https://raw.githubusercontent.com/scylladb/scylladb/master/test/alternator"

# P0 test files: core operations we support
P0_FILES=(
    test_table.py
    test_item.py
    test_batch.py
    test_query.py
    test_scan.py
    test_condition_expression.py
    test_filter_expression.py
    test_update_expression.py
    test_projection_expression.py
    test_key_condition_expression.py
    test_number.py
    test_nested.py
    test_describe_table.py
    test_returnvalues.py
    test_limits.py
    test_expected.py
)

# Additional test files (P1+)
EXTRA_FILES=(
    test_gsi.py
    test_gsi_updatetable.py
    test_lsi.py
    test_transact.py
    test_ttl.py
    test_tag.py
    test_batch.py
    test_compressed_request.py
    test_compressed_response.py
    test_describe_endpoints.py
    test_key_conditions.py
    test_manual_requests.py
    test_provisioned_throughput.py
    test_query_filter.py
    test_returnconsumedcapacity.py
)

download_file() {
    local file="$1"
    local url="${SCYLLA_BASE}/${file}"
    local dest="${VENDOR_DIR}/${file}"
    echo "  Downloading ${file}..."
    if ! curl -sfL "${url}" -o "${dest}"; then
        echo "  WARNING: Failed to download ${file}" >&2
        rm -f "${dest}"
    fi
}

# Create vendor directory
mkdir -p "${VENDOR_DIR}"

# Determine which files to download
if [[ "${1:-}" == "--all" ]]; then
    FILES=("${P0_FILES[@]}" "${EXTRA_FILES[@]}")
    echo "Downloading ALL Alternator test files..."
else
    FILES=("${P0_FILES[@]}")
    echo "Downloading P0 (core) Alternator test files..."
fi

# Download test files
for file in "${FILES[@]}"; do
    download_file "${file}"
done

# Copy our conftest.py and util.py into vendor/
echo "  Installing Rustack conftest.py and util.py..."
cp "${SCRIPT_DIR}/conftest.py" "${VENDOR_DIR}/conftest.py"
cp "${SCRIPT_DIR}/util.py" "${VENDOR_DIR}/util.py"

# Create __init__.py so relative imports work
touch "${VENDOR_DIR}/__init__.py"

# Patch relative imports to absolute (the test files use 'from .util import ...'
# which requires a package context; we convert to 'from util import ...')
echo "  Patching relative imports..."
for f in "${VENDOR_DIR}"/test_*.py; do
    # from .util import ... -> from util import ...
    sed -i.bak 's/^from \.util import/from util import/' "$f"
    # from test.alternator.util import ... -> from util import ...
    sed -i.bak 's/^from test\.alternator\.util import/from util import/' "$f"
    # from test.pylib.skip_types import ... -> from util import ...
    # Our util.py provides stand-in implementations of skip_env/skip_bug/etc.
    sed -i.bak 's/^from test\.pylib\.skip_types import/from util import/' "$f"
    rm -f "$f.bak"
done

# Set up Python virtual environment
VENV_DIR="${SCRIPT_DIR}/.venv"
if [ ! -d "${VENV_DIR}" ]; then
    echo "  Creating Python virtual environment..."
    python3 -m venv "${VENV_DIR}"
fi

echo "  Installing Python dependencies..."
"${VENV_DIR}/bin/pip" install -q -r "${SCRIPT_DIR}/requirements.txt"

echo ""
echo "Setup complete. ${#FILES[@]} test files downloaded to ${VENDOR_DIR}/"
echo ""
echo "Run tests with:"
echo "  cd ${VENDOR_DIR} && ${VENV_DIR}/bin/pytest -v --url http://localhost:4566 test_table.py"
echo ""
echo "Or use: make alternator"
