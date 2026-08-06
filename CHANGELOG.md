# Changelog

## Unreleased

### Added

- A live Forge dashboard for long workspace builds and test runs. `just
  watch-test` launches a run inside it, while `just watch-build` attaches to an
  existing run. It shows the active phase and crate, test progress, system
  load, and an ETA learned only from successful comparable runs.

### Changed

- Long-session scrolling, text selection, hover handling, and resizing now
  reuse retained viewport and render data, resolve active Markdown selection
  lazily, cache completed tool classifications, and debounce resize reflow.

## 1.0.1 - 2026-08-05

First stable Quiet release, based on upstream Codex `rust-v0.146.0`. It promotes
the 0.146.0 beta line unchanged in behavior; everything below shipped and was
used daily across beta.1 through beta.5.

Quiet now carries its own version, independent of the Codex version it is built
on. `codex-quiet --version` reports both, as `codex-quiet 1.0.1 (codex
0.146.0)`.

### Added

- A release channel for stable tags. `quiet-vX.Y.Z` now publishes as a stable
  release and becomes the latest pointer, while `quiet-vX.Y.Z-beta.N` continues
  to publish as a prerelease that never claims latest.
- `QUIET_VERSION` at the repository root, the single source of truth for the
  Quiet release version. The release workflow refuses any tag that disagrees
  with it.

### Changed

- Quiet versions independently of Codex. Releases were previously numbered with
  the upstream Codex version, which left nowhere to record a change that Quiet
  made on an unchanged base, and no way to ship a second release on that base.
  Appending to the upstream number cannot solve it: semver ignores build
  metadata when ordering versions, so `+quiet.2` would compare equal to
  `+quiet.1`, and a prerelease suffix sorts below the version it is built from,
  so every Quiet release would look older than the Codex release it shipped
  from. The Codex base is now recorded separately, in the Cargo workspace
  version, in `FORK_CHANGES.md`, and in the version string.
- Source builds now render as `codex-quiet <codex base>+local`, so a local build
  is never mistaken for a release. Release builds are unaffected, and the
  version used for update comparison is unchanged.
- Release notes are generated from this file's entry for the version being
  published, rather than from a fixed string in the release workflow that
  described one specific beta.

### Known limitations

- The macOS and Windows binaries are unsigned and not notarized. macOS
  Gatekeeper will warn on first launch. Each archive carries GitHub Actions
  build provenance, which is verifiable with `gh attestation verify`, but
  provenance is not a substitute for platform code signing.

No release assets were published for 1.0.0. All six platform archives built and
verified, but the publish step generates release notes from this file and its
job had no checkout, so it could not read it. 1.0.1 supersedes that failed
attempt.

## 0.146.0-beta.5 - 2026-08-01

### Changed

- Rebuilt legacy local-session tool history from compact call metadata only.
  Historical tool output bodies remain unloaded, and remote or paginated
  history behavior is unchanged.

### Fixed

- Restored tool-call rows when reopening legacy local sessions instead of
  showing only user and assistant messages.
- Indexed retained pager heights and rendered only the visible range, avoiding
  full-transcript measurement and traversal on every redraw in long sessions.

## 0.146.0-beta.4 - 2026-07-30

### Fixed

- Kept completed hook notifications lossless and prevented saturated live
  channels from delivering transient hook starts after their completions,
  eliminating stale `Running PreToolUse hook` and
  `Running PostToolUse hook` rows during ongoing turns.

## 0.146.0-beta.3 - 2026-07-29

### Changed

- Added a compact before/after text comparison with explicit timeline length
  and a concise decision table, making the terminal-interface tradeoffs visible
  before installation.

### Fixed

- Cached retained `Work` summaries and updated trailing groups incrementally,
  preventing long tool-heavy sessions from repeatedly rescanning prior command
  output during redraws and tool completion.

## 0.146.0-beta.2 - 2026-07-29

### Fixed

- Resolved the documented OpenAI Codex base tag from the official upstream
  repository during release validation, so a clean fork checkout does not
  depend on upstream tags existing in the fork remote.

## 0.146.0-beta.1 - 2026-07-29

### Changed

- Rebased Quiet for Codex on OpenAI Codex 0.146.0 while retaining the
  app-owned terminal surface, compact work groups, failure visibility,
  lifecycle cards, and fork safety behavior.
- Added GitHub Actions build provenance for each platform archive and a
  read-only hosted workflow for generating reviewable TUI snapshot patches.

No release assets were published for beta.1 because its clean hosted checkout
did not contain the documented upstream tag. Beta.2 supersedes that failed
attempt.

## 0.145.0-beta.4 - 2026-07-29

### Added

- Added theme-aware, full-row hover feedback for clickable `Work` group headers.

### Fixed

- Retained the previously detected terminal palette when a focus-triggered
  requery fails, keeping adaptive colors stable.

## 0.145.0-beta.3 - 2026-07-23

### Fixed

- Retried transient Windows executable-handle cleanup failures during release
  smoke tests while keeping persistent locks fatal.

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
