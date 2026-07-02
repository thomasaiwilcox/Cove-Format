#!/usr/bin/env sh
set -eu

fail() {
    echo "COVE boundary check failed: $*" >&2
    exit 1
}

if grep -nE '^[[:space:]]*(arrow-(array|buffer|schema)|parquet|datafusion|cove-arrow)[[:space:]]*=' crates/cove-core/Cargo.toml; then
    fail "cove-core must not depend on Arrow, Parquet, DataFusion, or cove-arrow"
fi

if grep -RInE 'arrow_array|arrow_buffer|arrow_schema|parquet::|datafusion::|use[[:space:]]+datafusion' crates/cove-core/src; then
    fail "cove-core source must not import Arrow, Parquet, or DataFusion crates"
fi

if [ -e crates/cove-core/src/interop/arrow.rs ] || [ -e crates/cove-core/src/interop/parquet.rs ]; then
    fail "Arrow and Parquet interop source must live in cove-arrow, not cove-core"
fi

[ -f crates/cove-arrow/src/arrow.rs ] || fail "missing cove-arrow Arrow interop module"
[ -f crates/cove-arrow/src/parquet.rs ] || fail "missing cove-arrow Parquet interop module"

if ! grep -q '^name = "cove"$' crates/cove/Cargo.toml; then
    fail "crates/cove must publish the canonical app-facing package as cove"
fi

if grep -nE '^[[:space:]]*(cove-cli|datafusion|datafusion-datasource)[[:space:]]*=' crates/cove/Cargo.toml; then
    fail "the cove facade must not depend on cove-cli or DataFusion directly"
fi

if grep -RInE '(^|[^[:alnum:]_])datafusion::|use[[:space:]]+datafusion(::|[[:space:]])' crates/cove/src; then
    fail "the cove facade must reach DataFusion through cove-engine/cove-datafusion APIs, not direct imports"
fi

if grep -RInE 'println!|eprintln!' crates/cove/src; then
    fail "the cove facade must expose typed results, not terminal output"
fi

if grep -RInE '(arrow-(array|buffer|schema)[^"]*"54"|parquet[^"]*"54")' Cargo.toml crates/*/Cargo.toml; then
    fail "Arrow and Parquet consumers must be on the Arrow 58 line"
fi

if grep -RInE '^[[:space:]]*datafusion[[:space:]]*=' Cargo.toml crates/*/Cargo.toml | grep -v '^crates/cove-datafusion/Cargo.toml:'; then
    fail "DataFusion dependency must be isolated to cove-datafusion"
fi

if grep -RInE '(^|[^[:alnum:]_])datafusion::|use[[:space:]]+datafusion(::|[[:space:]])' crates/*/src | grep -vE '^crates/cove-datafusion/src/(adapter_v53/|projection_provider(/|\.rs:)|register\.rs:)'; then
    fail "DataFusion imports must stay in cove-datafusion adapter_v53, projection_provider, or register"
fi
