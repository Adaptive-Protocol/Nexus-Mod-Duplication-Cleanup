#!/usr/bin/env bash
# Runs the Python engine and the Rust binary on the same folder and fails if their reports differ.
set -euo pipefail
bin=${1:-target/release/NexusModCleanup}; dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
n=0; while IFS= read -r f; do n=$((n + 1)); head -c $((n * 1500)) /dev/zero > "$dir/$f"; done < tests/fixtures.txt
for keep in 1 2; do
  python nexus_dedupe.py --folder "$dir" --keep "$keep" > /dev/null
  diff <(sed 2d "$dir"/dedupe_log_2*.txt; echo) <("$bin" --report "$dir" "$keep" | sed 2d)
  rm "$dir"/dedupe_log_2*.txt
done
echo "parity ok"
