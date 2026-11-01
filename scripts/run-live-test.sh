#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 <test-target|lib> <exact-test-name>" >&2
  echo "  TON_LIVE_FEATURES  cargo features (default: liteclient,mempool,overlay-config)" >&2
  echo "  TON_LIVE_LOG_DIR   evidence log directory (default: target/live-test-logs)" >&2
}

if [ "$#" -ne 2 ]; then
  usage
  exit 2
fi

target="$1"
test_name="$2"
features="${TON_LIVE_FEATURES:-liteclient,mempool,overlay-config}"
log_dir="${TON_LIVE_LOG_DIR:-target/live-test-logs}"
log_name="${target//\//_}-${test_name//:/_}.log"
log_path="$log_dir/$log_name"

mkdir -p "$log_dir"
if [ "$target" = "lib" ]; then
  target_args=(--lib)
else
  target_args=(--test "$target")
fi

echo "live-test target=$target test=$test_name features=$features log=$log_path"
set +e
cargo test -p tonutils --features "$features" "${target_args[@]}" "$test_name" -- \
  --exact --ignored --nocapture 2>&1 | tee "$log_path"
cargo_status=${PIPESTATUS[0]}
set -e

if [ "$cargo_status" -ne 0 ]; then
  echo "live-test failed: cargo exited $cargo_status; log=$log_path" >&2
  exit "$cargo_status"
fi
if grep -Fxq "running 0 tests" "$log_path"; then
  echo "live-test failed: exact filter matched zero tests; log=$log_path" >&2
  exit 1
fi
if ! grep -Fxq "running 1 test" "$log_path"; then
  echo "live-test failed: expected exactly one executed test; log=$log_path" >&2
  exit 1
fi
if ! grep -Fq "test $test_name ... ok" "$log_path"; then
  echo "live-test failed: no successful exact test result; log=$log_path" >&2
  exit 1
fi

echo "live-test passed: test=$test_name log=$log_path"
