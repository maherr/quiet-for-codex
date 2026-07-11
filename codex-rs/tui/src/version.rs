/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Local fork display label for user-facing TUI surfaces.
///
/// Keep `CODEX_CLI_VERSION` unchanged for protocol/update logic. The Quiet
/// label is derived from the same Cargo workspace version so an upstream
/// update cannot leave one user-facing surface on the previous version.
pub const CODEX_CLI_DISPLAY_NAME: &str = "codex-quiet";
pub const CODEX_CLI_DISPLAY_VERSION: &str = concat!("codex-quiet ", env!("CARGO_PKG_VERSION"));
