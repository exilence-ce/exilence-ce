#!/usr/bin/env bash
# Stress-runs the core tests to find and name flaky ones: builds the core test
# binary once, then runs <copies> of it at once for <rounds> rounds and prints
# every failing test with its failure output. Exits 1 if any run failed.
#
#   tools/stress.sh [filter] [copies] [rounds]    (defaults: all tests, 16, 20)
#
# `filter` is the libtest name filter (e.g. `profiles::f2_`). Linux, macOS and
# Windows (Git Bash).
set -euo pipefail

filter=()
[[ -n ${1:-} ]] && filter=("$1")
copies=${2:-16}
rounds=${3:-20}

bin=$(cargo test -p exilence-ce-ng --test core --locked --no-run --color never 2>&1 |
  sed -n 's/.*Executable .*(\(.*\))$/\1/p' | tail -n 1)
bin=${bin//\\//}
if [[ -z $bin ]]; then
  echo "stress: cannot find the core test binary" >&2
  exit 2
fi

logs=$(mktemp -d)
trap 'rm -rf "$logs"' EXIT

failed=0
for ((round = 1; round <= rounds; round++)); do
  for ((copy = 1; copy <= copies; copy++)); do
    "$bin" ${filter[@]+"${filter[@]}"} >"$logs/$round-$copy.log" 2>&1 &
  done
  wait || true
  for log in "$logs/$round"-*.log; do
    if ! grep -q '^test result: ok\.' "$log"; then
      failed=$((failed + 1))
      echo "=== round $round, copy $(basename "$log" .log | cut -d- -f2): FAILED"
      grep -E '^test .* \.\.\. FAILED$' "$log" || echo "(no test line failed: the binary itself failed)"
      sed -n '/^failures:$/,/^test result:/p' "$log" | head -n 60
    fi
  done
  echo "round $round/$rounds done, failed runs so far: $failed"
done

runs=$((rounds * copies))
echo "stress: $failed of $runs runs failed"
[[ $failed -eq 0 ]]
