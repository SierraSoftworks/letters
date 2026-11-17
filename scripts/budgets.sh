#!/usr/bin/env sh
# Checks the launch and keystroke budgets on a release build, on a document of 500 pages.
#
#   ./scripts/budgets.sh            the budgets of the reference machine, which the app holds itself
#   ./scripts/budgets.sh RUNNER     also that RUNNER's own limits in perf-budget.toml, as CI does
#   ./scripts/budgets.sh --window   the launch in a window, as the budget is worded, where there is a display
#
# Keystrokes are always measured without a window.
# Every measurement is made and printed before the script fails for any of them. A RUNNER that is over a
# limit is measured again, up to three rounds, and passes with the first round that is within them all.
set -eu
# No check reports its use as a person's (see apps/letters/src/telemetry.rs).
export LETTERS_TELEMETRY=0
cd "$(dirname "$0")/.."
headless=--headless
if [ "${1:-}" = --window ]; then
  headless=
  shift
fi
runner=${1:-}
target=${CARGO_TARGET_DIR:-target}

cargo build --release -p letters
letters=$target/release/letters

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The document is imported from Markdown, so it is made by the build it measures.
awk 'BEGIN {
  for (i = 0; i < 5000; i++) {
    if (i % 12 == 0) printf "## Section %d\n\n", i / 12 + 1
    printf "Rates change on **1 January**. "
    for (n = 0; n < 3; n++) {
      printf "The quick brown fox jumps over the lazy dog, and then writes a rather long paragraph about "
      printf "it so that lines wrap several times across the page column. "
    }
    printf "\n\n"
  }
}' >"$work/Rates.md"
"$letters" import "$work/Rates.md" >/dev/null

# The limit of `$runner` under `$1` in perf-budget.toml. Empty when the runner has none.
limit() {
  awk -v section="[$runner]" -v key="$1" '
    /^\[/ { inside = ($0 == section) }
    inside && $1 == key && $2 == "=" { print $3 }
  ' perf-budget.toml
}

# What was measured: the number before " ms" that follows `$2` in the report `$1`, on its first line that
# has `$3`.
measured() {
  sed -n "/${3:-^}/s/.*$2 \([0-9.]*\) ms.*/\1/p" "$work/$1" | head -n 1
}

# Fails when `$2`, measured as `$1` (and said to be `$3`), is over the runner's limit.
check() {
  allowed=$(limit "$1")
  if [ -z "$2" ] || [ -z "$allowed" ]; then
    echo "budgets.sh: no measurement or no limit for $1 on $runner" >&2
    failed=1
  elif awk -v measured="$2" -v allowed="$allowed" 'BEGIN { exit !(measured > allowed) }'; then
    echo "$1: $2 ms${3:+ $3} is over the $allowed ms that $runner is held to"
    failed=1
  else
    echo "$1: $2 ms${3:+ $3} is within the $allowed ms that $runner is held to"
  fi
}

# One round: both measurements, printed, with the runner's limits held to them. Sets `failed`.
round() {
  failed=0
  "$letters" --bench-launch $headless "$work/Rates.let" >"$work/launch" || failed=1
  cat "$work/launch"
  "$letters" --typing-report=1000 --headless "$work/Rates.let" >"$work/typing" || failed=1
  cat "$work/typing"
  if [ -n "$runner" ]; then
    # A shared machine is busy in fits, and that only ever adds time. So the fastest launches say what
    # Letters costs there, where the medians say how busy the machine was.
    launch=$(awk -v letters="$(measured launch fastest '^Letters ')" \
      -v empty="$(measured launch fastest '^empty window')" \
      'BEGIN { if (letters != "" && empty != "") printf "%.1f", letters - empty }')
    check launch_ms "$launch" "between the fastest launches"
    check keystroke_median_ms "$(measured typing median)"
    check keystroke_p99_ms "$(measured typing p99)"
  fi
}

round
# A runner shares its machine, and a while when that was busy is not Letters made slower. A slower Letters
# is over a limit in every round.
rounds=1
while [ "$failed" = 1 ] && [ -n "$runner" ] && [ "$rounds" -lt 3 ]; do
  rounds=$((rounds + 1))
  echo
  echo "Over a limit, so everything is measured again after a pause. Round $rounds of 3:"
  sleep 10
  round
done
exit $failed
