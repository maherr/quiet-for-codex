# Changelog

## Unreleased

### Added

- App-owned alternate-screen transcript with a fixed bottom composer, retained
  scrolling, mouse selection, clipboard copy, resize reflow, and replay.
- Failure-safe, outcome-first `Work` groups with `Alt+I` inspection and `Alt+O`
  expansion. Failed and action-required operations remain fully visible.
- Source-backed lifecycle cards for background terminals and collaborator fleets.
- Cargo-derived Quiet display versions across the CLI and TUI.

### Preserved

- `--no-alt-screen` and `tui.alternate_screen = "never"` as immediate inline
  fallbacks.
- Full raw source history through `Ctrl+T` and raw-output mode.

The upstream Codex changelog is published on the
[OpenAI Codex releases page](https://github.com/openai/codex/releases).
