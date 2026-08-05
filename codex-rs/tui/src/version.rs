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
/// The `+local` suffix is load-bearing, not decoration.
///
/// Quiet carries its own version, separate from the Codex base it is built on,
/// and a release renders both. A source build has no Quiet release version to
/// report, so it falls back to `CARGO_PKG_VERSION`, which is the Codex base.
/// Without a marker that fallback is just a bare version and reads like a
/// release. `+local` is semver build metadata, so it sorts and parses cleanly
/// while keeping the channels distinguishable at a glance:
///
/// - local/source build -> `codex-quiet 0.146.0+local`  (the number is the Codex base)
/// - prerelease         -> `codex-quiet 1.0.0-beta.1 (codex 0.146.0)`
/// - stable release     -> `codex-quiet 1.0.0 (codex 0.146.0)`
///
/// Only the fallback is marked. Release builds set `CODEX_QUIET_DISPLAY_VERSION`
/// explicitly and are untouched by this.
pub const CODEX_CLI_DISPLAY_VERSION: &str = match option_env!("CODEX_QUIET_DISPLAY_VERSION") {
    Some(version) => version,
    None => concat!("codex-quiet ", env!("CARGO_PKG_VERSION"), "+local"),
};
pub const CODEX_CLI_PRODUCT_NAME: &str = "Quiet for Codex";

#[cfg(test)]
mod tests {
    use super::*;

    /// The status card renders `CODEX_CLI_DISPLAY_VERSION`, so this is the string
    /// Maher actually reads to tell which build he is running. Guard it on both
    /// sides: a source build must be marked, a release build must not be. The
    /// exec crate carries the mirror of this test.
    #[test]
    fn display_version_distinguishes_a_source_build_from_a_stable_release() {
        assert!(CODEX_CLI_DISPLAY_VERSION.starts_with("codex-quiet "));

        let is_release_build = option_env!("CODEX_QUIET_DISPLAY_VERSION").is_some();
        if is_release_build {
            assert!(
                !CODEX_CLI_DISPLAY_VERSION.contains("+local"),
                "release build must not be marked local: {CODEX_CLI_DISPLAY_VERSION}"
            );
        } else {
            assert!(
                CODEX_CLI_DISPLAY_VERSION.ends_with("+local"),
                "a source build renders the same string as a stable release without \
                 the marker: {CODEX_CLI_DISPLAY_VERSION}"
            );
        }
    }

    /// The updater parses `CODEX_CLI_VERSION`, which must stay a bare semver.
    /// If the `+local` marker ever leaks into it, version comparison breaks.
    #[test]
    fn cli_version_stays_parseable_and_unmarked() {
        assert!(
            !CODEX_CLI_VERSION.contains("+local"),
            "CODEX_CLI_VERSION feeds update comparison and must stay bare: {CODEX_CLI_VERSION}"
        );
    }
}
