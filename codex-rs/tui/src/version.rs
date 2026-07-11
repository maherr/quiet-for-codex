/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Local fork display label for user-facing TUI surfaces.
///
/// Keep `CODEX_CLI_VERSION` unchanged for protocol/update logic: source builds
/// intentionally report `0.0.0`, and upstream update checks key off that value.
pub const CODEX_CLI_DISPLAY_NAME: &str = "codex-quiet";
pub const CODEX_CLI_DISPLAY_VERSION: &str = "codex-quiet 0.144.1";
