#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

# Provision first with run.py --setup. This gate never installs tools or fetches
# Cargo packages; CI also runs it inside an OS network sandbox.
python="${WISP_TEST_PYTHON:-python3}"
tools_dir="${WISP_CHAIN_TOOLS_DIR:-$PWD/.tools/component-library-chain}"
output_dir="${WISP_CHAIN_OUTPUT_DIR:-$PWD/out/component-library-chain}"
evidence_dir="${WISP_CHAIN_EVIDENCE_DIR:-$PWD/evidence/component-library-chain}"
export RUSTUP_TOOLCHAIN=1.93.0
export CARGO_NET_OFFLINE=true
mkdir -p "$evidence_dir"
rm -f "$evidence_dir/report.json" "$evidence_dir/negative-controls.json"

"$python" tests/component-chain/run.py \
  --tools-dir "$tools_dir" --output-dir "$output_dir" \
  2>&1 | tee "$evidence_dir/run.log"
cp "$output_dir/report.json" "$evidence_dir/report.json"
"$python" tests/component-chain/test_run.py \
  --tools-dir "$tools_dir" --output-dir "$output_dir" \
  --evidence "$evidence_dir/negative-controls.json" \
  2>&1 | tee "$evidence_dir/negative-controls.log"
