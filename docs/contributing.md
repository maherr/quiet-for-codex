# Contributing

Quiet for Codex accepts focused community contributions. This document applies to
the unofficial fork at `maherr/quiet-for-codex`, not to the upstream OpenAI
repository.

## Before opening a change

1. Search existing issues and pull requests.
2. For a bug, confirm whether it reproduces in the latest Quiet for Codex beta
   and in the matching upstream Codex CLI release.
3. Open an issue before a large behavior change. Small fixes and documentation
   corrections can go directly to a pull request.
4. Keep the change inside the fork's scope: terminal usability, retained
   history, compact activity presentation, platform portability, packaging,
   tests, and supporting documentation.

Issues that reproduce unchanged upstream should usually be reported to
[`openai/codex`](https://github.com/openai/codex/issues). Link the upstream issue
from a Quiet issue if the fork needs a temporary compatibility fix.

## Development workflow

Create a topic branch from `main`, keep commits focused, and explain the user
impact in the pull request.

The Rust workspace lives in `codex-rs`:

```shell
cd codex-rs
cargo build --bin codex
```

For Rust changes, follow the repository's `AGENTS.md` instructions. The usual
minimum validation is:

```shell
just fmt
just fix -p <crate-you-touched>
just test -p <crate-you-touched>
```

TUI changes must include or update relevant `insta` snapshots. Review every
changed snapshot rather than accepting them blindly. Platform-specific fixes
should include a regression test where the behavior can be exercised in CI.

Documentation-only changes do not require a Rust build.

## Pull request checklist

- State the problem and the behavior after the change.
- Identify the operating systems and terminals tested.
- List the exact validation commands and results.
- Add tests for behavior changes.
- Update README, support, install, or configuration documentation when user
  behavior changes.
- Keep upstream behavior intact unless the divergence is deliberate and
  documented in `FORK_CHANGES.md`.
- Do not include credentials, private session data, user prompts, proprietary
  source code, or personal information in fixtures or screenshots.

## Licensing

The fork does not use OpenAI's contributor invitation process or CLA bot. By
submitting a contribution, you agree that it may be distributed under this
repository's [Apache License 2.0](../LICENSE), and you represent that you have
the right to submit it under those terms.

Retain existing copyright, license, and attribution notices. New dependencies
must have a license compatible with the repository and must be represented in
the third-party notice process used by release artifacts.

## Conduct

Be specific, technical, and respectful. Harassment, personal attacks, and
publication of private data are not accepted.
