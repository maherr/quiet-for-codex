# Installing and building Quiet for Codex

Quiet for Codex is installed as `codex-quiet`, beside any official `codex` command.
Both commands can use the same `~/.codex` configuration and login state, but
their binaries and update channels remain separate.

## Install the latest release

### macOS or Linux

```shell
curl -fsSL https://raw.githubusercontent.com/maherr/quiet-for-codex/quiet-v0.145.0-beta.1/scripts/release/install.sh | sh
```

### Windows PowerShell

```powershell
& ([scriptblock]::Create((irm -UseBasicParsing https://raw.githubusercontent.com/maherr/quiet-for-codex/quiet-v0.145.0-beta.1/scripts/release/install.ps1)))
```

The installers detect the host target, download the matching archive from the
latest GitHub release, verify its published SHA-256 checksum, and install it in
a user-local versioned directory. They do not modify an official Codex install.

On macOS and Linux, packages live under
`~/.local/share/codex-quiet/releases/`. The installer updates
`~/.local/share/codex-quiet/current` and exposes
`~/.local/bin/codex-quiet`. Override those roots with
`CODEX_QUIET_INSTALL_ROOT` and `CODEX_QUIET_BIN_DIR`.

On Windows, packages live under
`%LOCALAPPDATA%\CodexQuiet\releases\`. The installer creates
`%LOCALAPPDATA%\CodexQuiet\bin\codex-quiet.cmd`, records the current package in
`current.txt`, and adds that bin directory to the user `PATH`. Override the
root with `CODEX_QUIET_INSTALL_ROOT`.

To install a specific release instead of the latest beta, set
`CODEX_QUIET_RELEASE` to a version such as `0.145.0-beta.1`. The Unix installer
also accepts `--release 0.145.0-beta.1` when downloaded and run as a file.

Run the installed command:

```shell
codex-quiet --version
codex-quiet
```

## Manual installation

1. Open the newest Quiet for Codex beta on the
   [releases page](https://github.com/maherr/quiet-for-codex/releases).
2. Select the archive matching your Rust target triple.
3. Download the archive and the release checksum file.
4. Verify the archive's SHA-256 digest.
5. Extract the complete package. Keep `bin`, `codex-resources`, `codex-path`,
   and `codex-package.json` together.
6. Add the extracted `bin` directory to `PATH`, or create a launcher that runs
   `bin/codex-quiet` from the package root.

Unix archives use this pattern:

```text
codex-quiet-<version>-<target>.tar.gz
```

Windows archives use this pattern:

```text
codex-quiet-<version>-<target>.zip
```

Put one downloaded archive and `SHA256SUMS` in the same directory. On macOS or
Linux, run:

```shell
(
  set -eu
  set -- codex-quiet-*.tar.gz
  [ "$#" -eq 1 ] && [ -f "$1" ] || {
    echo "expected exactly one codex-quiet tar.gz archive" >&2
    exit 1
  }
  archive=$1
  expected=$(awk -v file="$archive" '$2 == file { print $1 }' SHA256SUMS)
  [ "${#expected}" -eq 64 ] || {
    echo "no unique SHA256SUMS entry for $archive" >&2
    exit 1
  }
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{ print $1 }')
  else
    actual=$(shasum -a 256 "$archive" | awk '{ print $1 }')
  fi
  [ "$actual" = "$expected" ] || {
    echo "SHA-256 mismatch for $archive" >&2
    exit 1
  }
  echo "verified $archive"
)
```

In Windows PowerShell, run:

```powershell
$Archives = @(Get-ChildItem -File codex-quiet-*.zip)
if ($Archives.Count -ne 1) {
    throw "Expected exactly one codex-quiet zip archive."
}
$Archive = $Archives[0]
$Matches = @(Get-Content .\SHA256SUMS | Where-Object {
    $_.EndsWith("  $($Archive.Name)")
})
if ($Matches.Count -ne 1) {
    throw "No unique SHA256SUMS entry for $($Archive.Name)."
}
$Expected = ($Matches[0] -split '\s+', 2)[0]
$Actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive.FullName).Hash
if ($Actual -ne $Expected) {
    throw "SHA-256 mismatch for $($Archive.Name)."
}
Write-Host "Verified $($Archive.Name)"
```

This verifies that the archive exactly matches the release's published
checksum. A checksum is not a publisher signature. The current macOS and
Windows beta binaries remain unsigned, so also review the
[unsigned binary notes](../SUPPORT.md#unsigned-beta-binaries).

The package includes `codex-code-mode-host` beside the main executable and
platform resources used by sandboxing, shell integration, and file search. Do
not copy only the main executable if you need those features. Beta packages do
not include upstream's experimental patched-zsh payload; an enabled zsh-fork
flag falls back to the normal user shell.

## Supported targets

Published targets and validation levels are listed in [SUPPORT.md](../SUPPORT.md).
There are no 32-bit release archives.

The beta artifacts for macOS are not Apple-notarized, and Windows artifacts are
not Authenticode-signed. Review the
[unsigned binary notes](../SUPPORT.md#unsigned-beta-binaries) before using them.

## Build from source

Install Git and a current stable Rust toolchain, then:

```shell
git clone https://github.com/maherr/quiet-for-codex.git
cd quiet-for-codex/codex-rs
cargo build --release --bin codex --bin codex-code-mode-host
```

The resulting files are:

```text
target/release/codex
target/release/codex-code-mode-host
```

On Windows they have an `.exe` suffix. Keep the two files together. Rename only
the main executable from `codex` to `codex-quiet` if you copy them into a
directory on `PATH`.

A two-binary source build does not include all helper resources in a packaged
release. It is sufficient for ordinary TUI use, but some platform sandbox,
shell, and bundled search behavior may depend on resources from the full
package.

## Development checks

The repository uses `just` helpers for formatting, linting, and tests. Read
`AGENTS.md` before changing Rust code.

```shell
cargo install --locked just cargo-nextest
just fmt
just fix -p codex-tui
just test -p codex-tui
```

## Terminal fallback

If your terminal does not handle the retained alternate screen correctly:

```shell
codex-quiet --no-alt-screen
```

Or set:

```toml
[tui]
alternate_screen = "never"
```
