#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
target="${QUIET_TUI_TEST_TARGET:-x86_64-unknown-linux-gnu}"
runner_temp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
log="$(mktemp "${runner_temp%/}/quiet-tui-tests.XXXXXX.log")"

cleanup() {
  rm -f -- "$log"
}
trap cleanup EXIT

# These tests intentionally exercise the stdout-backed Crossterm terminal. They
# return EAGAIN inside the shared full-suite process on a noninteractive Linux
# runner but pass in a fresh process. Skip them in the main process, then run
# each one exactly once in a fresh test process.
isolated_tests=(
  "app::owned_screen::tests::edge_selection_schedules_frames_and_survives_resize_events"
  "app::tests::parent_scoped_exit_shuts_down_both_panes_without_submitting_op"
)

common=(
  cargo test --locked --target "$target" -p codex-tui --lib
)

print_log_tail() {
  local bytes="$1"
  tail -c "$bytes" "$log" || true
}

run_logged() {
  local stage="$1"
  local status
  shift

  set +e
  "$@" >> "$log" 2>&1
  status=$?
  set -e
  if (( status != 0 )); then
    printf 'TUI test stage failed: %s (exit %d)\n' "$stage" "$status" >&2
    print_log_tail 262144
    exit "$status"
  fi
}

cd "$repo_root/codex-rs"
main_args=("${common[@]}" -- --test-threads=1)
for test in "${isolated_tests[@]}"; do
  main_args+=(--skip "$test")
done
run_logged "main suite" "${main_args[@]}"

for test in "${isolated_tests[@]}"; do
  run_logged "$test" "${common[@]}" "$test" -- --exact --test-threads=1
done

print_log_tail 16384
