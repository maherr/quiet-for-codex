# Changelog

## 1.2.1 - 2026-08-10

### Added

- Publishes the 1.2 interaction cohort: the `#2F68C2` assistant marker,
  fixed-phrase working row with model and effort, `↓ newer` navigation cue,
  exact-width composer repair, complete resumed history, and hosted-only build
  flow described in 1.2.0 below.

### Fixed

- Made both Linux musl release targets emit SHA-1 GNU build IDs so each
  stripped executable remains verifiably paired with its debug sidecar.
- Added an arm64 hosted pre-tag symbol-archiver gate and distinct diagnostics
  for missing, mismatched, or strip-mutated build IDs. `quiet-v1.2.0` was not
  published after this gate rejected its Linux arm64 package; its immutable tag
  remains the failed source record.

## 1.2.0 - 2026-08-10

### Added

- Added a persistent `#2F68C2` blue marker for assistant replies, with the
  existing large dot and indentation retained as non-color cues. Truecolor
  terminals receive the exact color and limited-color terminals use a bright
  blue fallback.
- Added a bright one-cell Braille spinner for the main working row. Its phrase
  is selected once per user turn, and the effective model plus reasoning effort
  remain visible throughout the turn. Reduced-motion mode uses a static dot.
- Added a `↓ newer` footer cue while retained history is scrolled away from the
  live tail. An empty composer also shows the truthful `End` shortcut.
- Added per-platform debug-symbol archives keyed to the main executable and
  code-mode host platform identity, checksums, and source revision.

### Changed

- Capped the main activity animation at eight frames per second, coalesced its
  default terminal-title animation into the same activity-scoped redraw, and
  limited per-frame motion to the spinner cell. Compact tool indicators keep
  their existing treatment.
- Release monitoring now binds the run to the expected source SHA and tag, then
  polls the GitHub API until the workflow and its dynamically discovered matrix
  jobs finish successfully. Unknown skipped jobs fail the gate instead of
  counting as a pass.
- Finalized package smoke tests now resume a real persisted paginated transcript
  through the packaged app-server and, on POSIX targets, the packaged TUI.
- Feature-branch CI now builds a source-SHA-bound x86_64 Linux candidate package
  for private real-session and terminal verification before a release tag is
  created. The ephemeral artifact includes its checksum and expires after seven
  days.

### Fixed

- Preserved completed assistant replies when a dense newer turn exhausts the
  initial paginated-history budget. Summary anchors and detail pages merge by
  item ID in transcript order without duplicating the final reply.
- Added an immediate `Restoring history…` startup frame and kept live events
  buffered until initial replay is projected, so resume does not flash a partial
  or reordered transcript.
- Kept the effective post-resume model and reasoning override in the active
  working row instead of reverting visually to an earlier wrapper default.
- Removed the composer phantom blank row after the caret leaves an exactly full
  logical-line boundary, while preserving the boundary insertion point and
  intentional blank lines.

## 1.1.0 - 2026-08-10

### Added

- Added pane-scoped tool-run identities and explicit semantic settlement
  markers. Replay, raw mode, transcript inspection, and selection continue to
  use the original source cells.
- Added fixed-schema local outcome counters for bounded legacy tool-history
  restoration, without retaining commands, paths, or output bodies.
- Added content-blind debug counters and a dedicated retained-frame benchmark
  for render latency, input-to-draw latency, prepared-line cache reuse, and the
  owned-frame overhead above a footer-only diagnostic control. The release
  decision still uses a real owned-versus-inline terminal comparison.

### Changed

- Completed tool calls now remain as stable chronological rows while a tool
  succession is live. Eligible work folds once when the succession reaches an
  assistant, plan, reasoning, barrier, interruption, replay, or explicit turn
  boundary, instead of growing and rewriting a live `Work` group.
- A single ordinary action stays as its normal completed row. Two or more safe
  semantic actions fold, including a single command source that contains
  multiple actions. Failures, user-shell commands, approvals, warnings,
  action-required output, visible reasoning, and unfinished patches remain
  visible.
- Collapsed `Work` headers now occupy exactly one responsive terminal row.
  `Alt+I` prefers the last clicked group, then the hovered group, the last
  visible group, and finally the latest group. Repeated shortcut text moved to
  the contextual footer.
- Rich history and diff backgrounds now extend to the owned viewport edge, with
  a full-width divider separating retained history from the composer.
- Ordinary running state and routine hook progress now reuse the composer's
  existing footer lane, so idle and running transitions do not change the
  bottom pane's height. Quiet hook success disappears immediately, while
  failed, blocked, stopped, actionable, or output-bearing hook results remain
  in history.
- Running motion is capped at 10 frames per second under animations. The
  separate hook animation loop and transcript activity animation were removed;
  reduced-motion status is event-driven apart from optional elapsed seconds.
- Retained projection now updates only affected ranges, preserves scroll and
  selection anchors, reuses stable wrapper and prepared-line caches, and keeps
  hook, token, rate-limit, and primary-tool invalidation independent. Inline
  mode performs no grouping reflow while a run is open and at most one when it
  settles.
- Initial and thread-switch replay now builds settled groups before the first
  visible frame, avoiding an expanded-then-collapsed flash.

### Fixed

- Preserved pending tool-run settlement across selection-safe deferred commits,
  so a run that begins during text selection still folds exactly once after its
  source cells enter the viewport.
- Kept ordinary running and routine hook status visible when a composer popup
  or special footer mode temporarily occupies the normal footer lane.
- Prevented invisible tool-run settlement markers from adding blank rows to the
  complete transcript overlay.
- Invalidated completed exec and MCP classification memos whenever output,
  completion, or failure state changes, and bypassed those memos while a call
  is still active.
- Replaced pointer-derived group identity with stable run IDs, preserving
  hover and expansion state through background-terminal promotion and local
  lifecycle updates.
- Kept finalized live cells visible until their history commit is acknowledged,
  closing the one-frame tail gap between active and retained history.
- Corrected the variadic benchmark recipe so smoke-test and filter arguments
  reach Cargo instead of silently triggering an unfiltered workspace run, and
  isolated the retained-render benchmark behind a feature-gated binary so it
  does not compile unrelated TUI binaries or test dependencies.

## 1.0.3 - 2026-08-07

Stable release of `1.0.3-beta.1`, promoted unchanged: the first stable Quiet on
the upstream Codex `rust-v0.147.0` base.

### Changed

- Rebased Quiet onto upstream Codex `rust-v0.147.0`. All Quiet presentation
  behavior carries over unchanged: failure-safe Work bundles, lifecycle cards,
  per-group inspection, the owned-screen transcript with its fixed composer,
  and the inline fallback.
- Adopted upstream's paginated scrollback history inside the compact tool
  grouping renderer, so the earlier-messages notice and history top-up work
  with Quiet's grouped transcript.
- Consolidated closed-thread bookkeeping on Quiet's bounded retirement
  tombstones, which now also cover upstream's new late-request rejection for
  closed side conversations.

### Removed

- Dropped Quiet's focus-time palette retention patch. Upstream `rust-v0.147.0`
  removed focus-time palette requeries entirely, which supersedes the patch and
  is verified by an upstream regression test.

## 1.0.3-beta.1 - 2026-08-07

### Changed

- Rebased Quiet onto upstream Codex `rust-v0.147.0`. All Quiet presentation
  behavior carries over unchanged: failure-safe Work bundles, lifecycle cards,
  per-group inspection, the owned-screen transcript with its fixed composer,
  and the inline fallback.
- Adopted upstream's paginated scrollback history inside the compact tool
  grouping renderer, so the earlier-messages notice and history top-up work
  with Quiet's grouped transcript.
- Consolidated closed-thread bookkeeping on Quiet's bounded retirement
  tombstones, which now also cover upstream's new late-request rejection for
  closed side conversations.

### Removed

- Dropped Quiet's focus-time palette retention patch. Upstream `rust-v0.147.0`
  removed focus-time palette requeries entirely, which supersedes the patch and
  is verified by an upstream regression test.

## 1.0.2 - 2026-08-06

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
