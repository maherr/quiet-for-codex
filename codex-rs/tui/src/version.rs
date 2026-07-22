/// The public Quiet release version, including a prerelease suffix when set by
/// the release workflow. Local/source builds fall back to the upstream Cargo
/// workspace version.
pub const CODEX_CLI_VERSION: &str = match option_env!("CODEX_QUIET_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Local fork display label for user-facing TUI surfaces.
///
/// Protocol-facing crates retain the upstream Cargo version. These constants
/// identify the public Quiet build on user-facing TUI surfaces.
pub const CODEX_CLI_DISPLAY_NAME: &str = "codex-quiet";
pub const CODEX_CLI_DISPLAY_VERSION: &str = match option_env!("CODEX_QUIET_DISPLAY_VERSION") {
    Some(version) => version,
    None => concat!("codex-quiet ", env!("CARGO_PKG_VERSION")),
};
pub const CODEX_CLI_PRODUCT_NAME: &str = "Quiet for Codex";
