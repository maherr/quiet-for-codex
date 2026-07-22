# Third-party notices

Quiet for Codex is a modified distribution of OpenAI's Codex CLI. The
repository's primary terms are the [Apache License 2.0](LICENSE), with
attribution in [NOTICE](NOTICE).

This file is an index for material with separate terms. It does not replace the
license text shipped with a component.

## Source included in this repository

| Component | Location | License |
| --- | --- | --- |
| Ratatui-derived code | Identified in `NOTICE` and source history | MIT |
| WezTerm-derived code | `third_party/wezterm/` | MIT, full text in `third_party/wezterm/LICENSE` |
| Bubblewrap | `codex-rs/vendor/bubblewrap/` | LGPL-2.0-or-later, full text in `codex-rs/vendor/bubblewrap/COPYING` |
| Rust crates | Exact versions in `codex-rs/Cargo.lock` | Per-crate terms; accepted license policy in `codex-rs/deny.toml` |

The source tree also contains generated samples and nested assets with their own
license files. Those files remain with the material they cover.

## Components in release packages

Depending on platform, a release package can include:

- `codex-code-mode-host`, built from this repository;
- `bwrap` on Linux, built from the vendored Bubblewrap source;
- `rg`, the ripgrep executable, distributed under MIT or Unlicense terms;
- Windows sandbox helper executables built from this repository.

Release archives preserve the root `LICENSE`, `NOTICE`, `FORK_CHANGES.md`, and
this index. They also include full texts under `THIRD_PARTY_LICENSES/`: a
generated Rust dependency report, the WezTerm and ripgrep terms, and the
Bubblewrap terms on Linux. They also include a generated V8 and `rusty_v8`
notice bundle with exact source commits, artifact provenance, and verified
license-text hashes. The bundle is a conservative notice inventory, not legal
advice or a claim that automated review can determine every obligation. Exact
versions and source URLs for downloaded helpers are pinned in
`scripts/codex_package/` and `scripts/release/v8-notices-manifest.json`.

## Dependency inventory

`codex-rs/Cargo.lock` is the exact Rust dependency inventory for a source
revision. `codex-rs/deny.toml` records accepted SPDX license expressions and
review exceptions. JavaScript and SDK dependency inventories remain in their
respective lockfiles and package manifests.

If an archive and this index disagree, treat that as a packaging bug and report
it before redistributing the archive.
