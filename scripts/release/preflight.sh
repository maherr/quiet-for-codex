#!/usr/bin/env bash
# Run the release workflow's gates locally, before tagging.
#
# A tag is immutable and a failed release burns it, so the expensive failure is
# not a slow pipeline, it is discovering a broken gate after the tag exists.
# Every check here mirrors one the hosted release runs. The one that matters
# most is the identity test in the RELEASE environment: the release injects
# CODEX_QUIET_DISPLAY_VERSION, so a test that passes in a plain source build can
# still fail the release, which is exactly how a stable tag was burned once.
#
# Usage: scripts/release/preflight.sh [version]
#   version defaults to the contents of QUIET_VERSION.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
REPO_ROOT="$PWD"

VERSION="${1:-$(tr -d '[:space:]' < QUIET_VERSION)}"
CODEX_BASE="$(python3 -c 'import tomllib; print(tomllib.load(open("codex-rs/Cargo.toml","rb"))["workspace"]["package"]["version"])')"
DISPLAY="codex-quiet ${VERSION} (codex ${CODEX_BASE})"

FAILED=()
PASSED=0

run() {
  local name="$1"; shift
  printf '%-58s' "$name"
  if "$@" > "/tmp/preflight-${name//[^A-Za-z0-9]/_}.log" 2>&1; then
    printf 'PASS\n'; PASSED=$((PASSED + 1))
  else
    printf 'FAIL  (log: /tmp/preflight-%s.log)\n' "${name//[^A-Za-z0-9]/_}"
    FAILED+=("$name")
  fi
}

echo "Quiet release preflight"
echo "  version:    $VERSION"
echo "  codex base: $CODEX_BASE"
echo "  display:    $DISPLAY"
echo

# --- Identity agreement, the cheap checks that gate the hosted validate step ---
run "version shape accepted by the packager" python3 -c "
import sys; sys.path.insert(0, 'scripts')
from release.finalize_quiet_package import RELEASE_VERSION_RE
raise SystemExit(0 if RELEASE_VERSION_RE.fullmatch('$VERSION') else 'version $VERSION rejected by RELEASE_VERSION_RE')
"
run "QUIET_VERSION matches first CHANGELOG heading" python3 -c "
import re, pathlib
want = '$VERSION'
heads = [l for l in pathlib.Path('CHANGELOG.md').read_text().splitlines()
         if l.startswith('## ') and l != '## Unreleased']
m = re.fullmatch(r'## ([0-9]+\.[0-9]+\.[0-9]+(?:-beta\.[1-9][0-9]*)?) - [0-9]{4}-[0-9]{2}-[0-9]{2}', heads[0])
raise SystemExit(0 if m and m.group(1) == want else f'first CHANGELOG heading {heads[0]!r} does not match {want}')
"
run "FORK_CHANGES documents the Cargo codex base" python3 -c "
import re, pathlib
t = pathlib.Path('FORK_CHANGES.md').read_text()
rel = re.findall(r'^- Base release: \`(rust-v[0-9]+\.[0-9]+\.[0-9]+)\`\$', t, re.M)
raise SystemExit(0 if rel == ['rust-v$CODEX_BASE'] else f'FORK_CHANGES base {rel} != rust-v$CODEX_BASE')
"
run "no em or en dash in public text" bash -c '
! grep -rnP '\''\x{2014}|\x{2013}'\'' CHANGELOG.md README.md docs/install.md SUPPORT.md 2>/dev/null'

# --- Source-only gates, mirroring the release verify job ---
run "rustfmt" python3 scripts/ci/check_quiet_rustfmt.py
run "embedded skill identity" python3 scripts/ci/check_embedded_skill_identity.py
for t in test_generate_v8_notices test_prepare_v8_artifacts test_archive_release_symbols \
         test_finalize_quiet_package test_smoke_quiet_package test_watch_workflow_run \
         test_install_sh test_release_workflow; do
  run "$t" python3 "scripts/release/$t.py"
done
run "release Python syntax" python3 -m py_compile \
  scripts/release/finalize_quiet_package.py \
  scripts/release/generate_v8_notices.py \
  scripts/release/prepare_v8_artifacts.py \
  scripts/release/smoke_quiet_package.py \
  scripts/release/watch_workflow_run.py
run "ripgrep packaging" python3 scripts/codex_package/test_ripgrep.py
run "install.sh syntax" sh -n scripts/release/install.sh
run "release workflow parses" python3 -c "import yaml; yaml.safe_load(open('.github/workflows/quiet-release.yml'))"
run "ci workflow parses" python3 -c "import yaml; yaml.safe_load(open('.github/workflows/quiet-ci.yml'))"

# --- Rust gates ---
cd "$REPO_ROOT/codex-rs"
run "full TUI library suite" cargo test --locked -p codex-tui --lib -- --test-threads=1
run "feedback uploads disabled" cargo test --locked -p codex-feedback --lib quiet_feedback_uploads_are_disabled
run "app-server feedback API disabled" cargo test --locked -p codex-app-server --lib quiet_feedback_upload_api_stays_disabled
run "feedback command reports disabled" cargo test --locked -p codex-tui --lib feedback_command_reports_uploads_disabled
run "quiet commands hidden" cargo test --locked -p codex-tui --lib quiet_commands_are_hidden_from_command_popup
run "desktop subcommand absent" cargo test --locked -p codex-cli --bin codex quiet_build_does_not_register_desktop_app_subcommand
run "release channel list" cargo test --locked -p codex-cli --bin codex release_list_includes_prerelease_channel
run "cli update" cargo test --locked -p codex-cli --test update

# The release configuration. A plain source build cannot exercise this path, so
# without it a display-string change looks green locally and fails the release.
run "identity test IN RELEASE CONFIG" env \
  CODEX_QUIET_VERSION="$VERSION" \
  CODEX_QUIET_DISPLAY_VERSION="$DISPLAY" \
  cargo test --locked -p codex-exec --lib exec_summary_uses_quiet_identity_and_exact_build_version
run "tui version tests IN RELEASE CONFIG" env \
  CODEX_QUIET_VERSION="$VERSION" \
  CODEX_QUIET_DISPLAY_VERSION="$DISPLAY" \
  cargo test --locked -p codex-tui version::tests

echo
if [[ "${#FAILED[@]}" -gt 0 ]]; then
  echo "PREFLIGHT FAILED: ${#FAILED[@]} of $((PASSED + ${#FAILED[@]})) gates"
  printf '  - %s\n' "${FAILED[@]}"
  echo "Do NOT tag. A failed release burns the tag."
  exit 1
fi
echo "PREFLIGHT PASSED: $PASSED gates. Safe to tag quiet-v$VERSION."
