#!/bin/sh

set -eu

REPOSITORY="maherr/quiet-for-codex"
RELEASE="${CODEX_QUIET_RELEASE:-latest}"
INSTALL_ROOT="${CODEX_QUIET_INSTALL_ROOT:-$HOME/.local/share/codex-quiet}"
BIN_DIR="${CODEX_QUIET_BIN_DIR:-$HOME/.local/bin}"
TEMP_DIR=""

step() {
  printf '==> %s\n' "$1"
}

fail() {
  printf 'ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if [ -n "$TEMP_DIR" ] && [ -d "$TEMP_DIR" ]; then
    rm -rf "$TEMP_DIR"
  fi
}

trap cleanup EXIT HUP INT TERM

case "$INSTALL_ROOT" in
  "" | "/") fail "CODEX_QUIET_INSTALL_ROOT must be a non-root directory." ;;
esac
case "$BIN_DIR" in
  "" | "/") fail "CODEX_QUIET_BIN_DIR must be a non-root directory." ;;
esac

usage() {
  printf '%s\n' 'Usage: install.sh [--release VERSION]'
  printf '%s\n' ''
  printf '%s\n' 'Environment:'
  printf '%s\n' '  CODEX_QUIET_RELEASE       Release version or latest.'
  printf '%s\n' '  CODEX_QUIET_INSTALL_ROOT  Versioned package root.'
  printf '%s\n' '  CODEX_QUIET_BIN_DIR       Directory for the codex-quiet symlink.'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --release)
      [ "$#" -ge 2 ] || fail "--release requires a value."
      RELEASE="$2"
      shift
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    *)
      fail "Unknown argument: $1"
      ;;
  esac
  shift
done

download_file() {
  url="$1"
  output="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$output" "$url"
  else
    fail "curl or wget is required."
  fi
}

download_text() {
  url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O - "$url"
  else
    fail "curl or wget is required."
  fi
}

normalize_version() {
  case "$1" in
    quiet-v*) printf '%s\n' "${1#quiet-v}" ;;
    v*) printf '%s\n' "${1#v}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

validate_version() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)(\.[0-9]+)?)?$' \
    || fail "Invalid release version: $1"
}

resolve_latest_version() {
  api_url="https://api.github.com/repos/$REPOSITORY/releases?per_page=20"
  version="$(download_text "$api_url" | awk -F '"' '
    /"tag_name":[[:space:]]*"quiet-v[0-9]/ {
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^quiet-v[0-9]/) {
          sub(/^quiet-v/, "", $i)
          print $i
          exit
        }
      }
    }
  ')"
  [ -n "$version" ] || fail "No Quiet for Codex release was found."
  printf '%s\n' "$version"
}

detect_target() {
  system="$(uname -s)"
  machine="$(uname -m)"
  case "$system:$machine" in
    Linux:x86_64 | Linux:amd64) printf '%s\n' 'x86_64-unknown-linux-musl' ;;
    Linux:aarch64 | Linux:arm64) printf '%s\n' 'aarch64-unknown-linux-musl' ;;
    Darwin:x86_64 | Darwin:amd64) printf '%s\n' 'x86_64-apple-darwin' ;;
    Darwin:arm64 | Darwin:aarch64) printf '%s\n' 'aarch64-apple-darwin' ;;
    *) fail "Unsupported platform: $system $machine" ;;
  esac
}

file_sha256() {
  path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    fail "sha256sum or shasum is required."
  fi
}

if [ "$RELEASE" = "latest" ] || [ -z "$RELEASE" ]; then
  VERSION="$(resolve_latest_version)"
else
  VERSION="$(normalize_version "$RELEASE")"
fi
validate_version "$VERSION"
TARGET="$(detect_target)"
TAG="quiet-v$VERSION"
ASSET="codex-quiet-$VERSION-$TARGET.tar.gz"
BASE_URL="https://github.com/$REPOSITORY/releases/download/$TAG"

TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/codex-quiet-install.XXXXXX")"
ARCHIVE_PATH="$TEMP_DIR/$ASSET"
SUMS_PATH="$TEMP_DIR/SHA256SUMS"

step "Downloading Quiet for Codex $VERSION for $TARGET"
download_file "$BASE_URL/$ASSET" "$ARCHIVE_PATH"
download_file "$BASE_URL/SHA256SUMS" "$SUMS_PATH"

EXPECTED_SHA="$(awk -v asset="$ASSET" '$2 == asset && $1 ~ /^[0-9a-fA-F]{64}$/ { print tolower($1); exit }' "$SUMS_PATH")"
[ -n "$EXPECTED_SHA" ] || fail "SHA256SUMS has no digest for $ASSET."
ACTUAL_SHA="$(file_sha256 "$ARCHIVE_PATH")"
[ "$EXPECTED_SHA" = "$ACTUAL_SHA" ] || fail "Archive checksum mismatch."

RELEASES_DIR="$INSTALL_ROOT/releases"
TARGET_DIR="$RELEASES_DIR/$VERSION-$TARGET"
STAGING_DIR="$RELEASES_DIR/.staging.$VERSION.$$"
CURRENT_LINK="$INSTALL_ROOT/current"
VISIBLE_BIN="$BIN_DIR/codex-quiet"

mkdir -p "$RELEASES_DIR" "$BIN_DIR" "$STAGING_DIR"
tar -xzf "$ARCHIVE_PATH" -C "$STAGING_DIR"
[ -x "$STAGING_DIR/bin/codex-quiet" ] || fail "Archive is missing bin/codex-quiet."
[ -x "$STAGING_DIR/bin/codex-code-mode-host" ] || fail "Archive is missing bin/codex-code-mode-host."

if [ -e "$TARGET_DIR" ]; then
  rm -rf "$TARGET_DIR"
fi
mv "$STAGING_DIR" "$TARGET_DIR"

NEXT_CURRENT="$INSTALL_ROOT/.current.$$"
ln -s "$TARGET_DIR" "$NEXT_CURRENT"
if [ -L "$CURRENT_LINK" ]; then
  rm -f "$CURRENT_LINK"
elif [ -e "$CURRENT_LINK" ]; then
  fail "$CURRENT_LINK exists and is not a symlink."
fi
mv "$NEXT_CURRENT" "$CURRENT_LINK"

if [ -e "$VISIBLE_BIN" ] && [ ! -L "$VISIBLE_BIN" ]; then
  fail "$VISIBLE_BIN exists and is not a symlink. Set CODEX_QUIET_BIN_DIR to another directory."
fi
NEXT_BIN="$BIN_DIR/.codex-quiet.$$"
ln -s "$CURRENT_LINK/bin/codex-quiet" "$NEXT_BIN"
if [ -L "$VISIBLE_BIN" ]; then
  rm -f "$VISIBLE_BIN"
fi
mv "$NEXT_BIN" "$VISIBLE_BIN"

step "Installed $VISIBLE_BIN"
case ":${PATH:-}:" in
  *":$BIN_DIR:"*) ;;
  *) printf 'Add %s to PATH, then run codex-quiet.\n' "$BIN_DIR" ;;
esac
