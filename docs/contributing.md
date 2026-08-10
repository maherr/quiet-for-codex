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

For a full workspace run, Forge keeps the long compile visible and preserves
the test command's exit status:

```shell
just watch-test
# In another terminal, attach to an existing run:
just watch-build
```

It reports the active phase and crate, test progress, system load, and a
history-based ETA. The first successful run calibrates that estimate. Pressing
Ctrl-C detaches the dashboard without stopping the build; captured output stays
under `~/.cache/codex-build-watch/runs/`.

TUI changes must include or update relevant `insta` snapshots. Review every
changed snapshot rather than accepting them blindly. Platform-specific fixes
should include a regression test where the behavior can be exercised in CI.

Documentation-only changes do not require a Rust build.

## Release workflow monitoring

Pushes to a `tui/quiet-*` branch produce a seven-day
`quiet-linux-candidate-<source-sha>` artifact. GitHub builds the runnable
x86_64 Linux musl package, verifies its embedded Quiet and Codex versions, and
ships a checksum plus `source-sha.txt`. Use that hosted binary for private
real-session and terminal verification; never upload session data to Actions.

After a release tag starts the hosted workflow, use the repository's API-based
watcher rather than `gh run watch`:

```shell
python3 scripts/release/watch_workflow_run.py RUN_ID \
  --repo maherr/quiet-for-codex \
  --sha CANDIDATE_SHA \
  --ref RELEASE_TAG
```

The watcher first binds the run to the expected source SHA and tag, waits for the
workflow's final conclusion, discovers every matrix job through the paginated
jobs API, and rejects failed, cancelled, or unknown skipped jobs. A
successful-looking notification or an incomplete fixed job list is not a
release verdict.

## Pull request checklist

- State the problem and the behavior after the change.
- Identify the operating systems and terminals tested.
- List the exact validation commands and results.
- Add tests for behavior changes.
- Update README, support, install, or configuration documentation when user
  behavior changes.
- Keep upstream behavior intact unless the divergence is deliberate and
  documented in `FORK_CHANGES.md`.
- If the change touches release packaging, the version string, or either
  workflow, run `scripts/release/preflight.sh`. It runs the release gates
  locally, including the two that only fail once the release injects its
  display version, which a plain source build never exercises.
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
