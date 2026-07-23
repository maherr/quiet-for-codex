# Changelog

## Unreleased

## 0.145.0-beta.2 - 2026-07-23

### Fixed

- Reduced long-session memory growth by adopting OpenAI Codex's request
  serialization fix, which avoids cloning the full prior request and current
  input prefix at every WebSocket tool step.
- Bounded Quiet's compact tool-output classifier to 256 KiB and stopped
  materializing full successful outputs when a cell can be rejected before
  classification.

## 0.145.0-beta.1 - 2026-07-22

### Added

- App-owned alternate-screen transcript with a fixed bottom composer, retained
  scrolling, mouse selection, clipboard copy, resize reflow, and replay.
- Failure-safe, outcome-first `Work` groups with per-group click expansion,
  `Alt+I` inspection, temporary `Alt+O` show-all, and compact live progress.
  Failed, streamed, and action-required operations remain fully visible.
- Source-backed lifecycle cards for background terminals and collaborator fleets.
- A side-by-side `codex-quiet` command and Quiet-specific version identity
  across the CLI, TUI, package metadata, and diagnostics.
- Checksum-verifying installers and native release packages for Linux, macOS,
  and Windows on x86_64 and arm64.
- Fork-owned CI, platform smoke tests, release checks, dependency notices,
  issue templates, and support documentation.

### Changed

- Upstream update actions, remote announcements, Desktop app promotion, the
  `app` subcommand, `/app` handoff, and login-success Desktop redirects are
  disabled in Quiet builds.
- Daemon-managed app-server and daemon-backed remote-control routes fail closed
  because their upstream implementation installs and updates stock Codex.
- Feedback and log uploads to OpenAI's upstream Sentry endpoint fail closed;
  Quiet support is routed through this repository's issue tracker.
- User-facing command examples, completions, diagnostics, and resume hints use
  the side-by-side `codex-quiet` executable, including embedded skill assets.
- Binary packages omit the experimental patched-zsh payload and fall back to
  the normal user shell when that shared feature flag is enabled.

### Preserved

- `--no-alt-screen` and `tui.alternate_screen = "never"` as immediate inline
  fallbacks.
- Full raw source history through `Ctrl+T` and raw-output mode.

The upstream Codex changelog is published on the
[OpenAI Codex releases page](https://github.com/openai/codex/releases).
