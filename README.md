# Quiet for Codex

Quiet for Codex is an independent, community-maintained fork of the OpenAI Codex
CLI with a calmer terminal interface. It keeps the composer anchored, makes
long tool runs easier to scan, and preserves normal terminal selection and
scrolling.

> [!IMPORTANT]
> Quiet for Codex is unofficial. It is not an OpenAI product, is not endorsed by
> OpenAI, and does not present OpenAI branding as its own. The names OpenAI and
> Codex identify the upstream project and compatible service; they do not imply
> sponsorship.

The current beta is based on upstream [`rust-v0.146.0`](https://github.com/openai/codex/releases/tag/rust-v0.146.0).

## See the difference

This illustrative turn has the same successful outcomes in both views: inspect
three files, make three edits, then pass formatting, tests, a release build, and
a workspace check. It compares presentation only, not execution speed, tool
count, or token use.

### Before: Official Codex (8 blocks, 16 timeline lines)

```text
• Explored
  ├ Read src/app.rs
  ├ Read src/keybindings.rs
  ├ Read tests/keybindings.rs
  └ Search toggle_panel

• Edited src/app.rs

• Edited src/keybindings.rs

• Edited tests/keybindings.rs

• Ran cargo fmt --check
  └ passed

• Ran cargo test -p codex-tui
  └ 48 passed

• Ran cargo build --release
  └ finished

• Ran cargo check --workspace
  └ passed
```

### After: Quiet for Codex (1 row, 3 timeline lines)

```text
▸ Work: read 3 files · searched · 3 files edited
  tests passed · build passed · check passed · ran command
  Alt+I inspect · Alt+O all
```

That is 81% less vertical timeline for this illustrative turn. Quiet folds
eligible successful work into one outcome-first row without discarding the
underlying details. Click the `Work` row or press `Alt+I` for exact commands,
outputs, and durations; `Alt+O` temporarily opens all groups, and `Ctrl+T`
opens the complete transcript. During the turn, the composer stays anchored
below retained, selectable history.

## What changes

- The composer stays pinned to the bottom in an app-owned alternate screen.
- Eligible successful commands collapse into outcome-first `▸ Work` groups.
  Failures and results that need action stay expanded.
- A `Work` header expands in place when clicked and collapses when clicked
  again. Dragging from the row still selects text.
- Background terminals and collaborator fleets render as compact lifecycle
  cards.
- The current command remains visible during long multi-command exploration.
- Terminal scrolling, resize reflow, mouse selection, copying, and transcript
  replay work inside retained history.
- Noisy successful output and routine hook rows take less space.

The retained-interface changes are concentrated in the Rust TUI. Except for the
fork-safety divergences documented in [Fork changes](FORK_CHANGES.md),
configuration, sessions, tools, and compatible service access track the
corresponding upstream Codex CLI release. Binary beta packages omit the
experimental patched-zsh backend; if that upstream feature is enabled in shared
configuration, Quiet falls back to the normal user shell.

## Which should I use?

| If you care most about | Official Codex | Quiet for Codex |
| --- | --- | --- |
| Official distribution and support | OpenAI release and support channels | Unofficial, community-maintained beta |
| Long tool-heavy turns | Successful calls remain as individual blocks in the main flow | Eligible successful work folds into outcome-first `Work` rows |
| Failure visibility | Failed and successful calls use the normal upstream presentation | Failures stay visible while eligible routine success folds |
| Inspecting details | Inline previews and the complete transcript | Click, `Alt+I`, `Alt+O`, or the complete transcript |
| Desktop integration and updates | Upstream Desktop handoff and update path, where available | Desktop handoff is disabled and releases use a separate update channel |
| Trying both | Runs as `codex` | Installs beside it as `codex-quiet` and reuses existing `~/.codex` data |

Choose Quiet if transcript density is the problem you want to solve. Stay with
official Codex if official distribution, Desktop integration, and immediate
upstream releases matter more.

## Install

Quiet for Codex installs beside the official CLI as `codex-quiet`. It does not
replace or remove a `codex` command you already have.

### macOS or Linux

```shell
curl -fsSL https://raw.githubusercontent.com/maherr/quiet-for-codex/quiet-v0.146.0-beta.3/scripts/release/install.sh | sh
```

### Windows PowerShell

```powershell
& ([scriptblock]::Create((irm -UseBasicParsing https://raw.githubusercontent.com/maherr/quiet-for-codex/quiet-v0.146.0-beta.3/scripts/release/install.ps1)))
```

The installers select the matching archive from the newest Quiet for Codex
entry on the
[GitHub releases page](https://github.com/maherr/quiet-for-codex/releases),
verify its published SHA-256 checksum, and install a user-local command. See
[Installing and building](docs/install.md) for manual installation, exact
paths, checksum and build-provenance verification, and source builds.

Release binaries are not yet Apple-notarized or Windows code-signed. See the
[platform notes](SUPPORT.md#unsigned-beta-binaries) before installing on those
systems.

## Run

```shell
codex-quiet
```

Quiet for Codex uses the same `~/.codex` directory as upstream, so an existing
login, configuration, sessions, and project trust settings remain available.
The executable and update channel stay separate from the official CLI.

Use the inline terminal mode if an alternate screen does not work well in your
terminal:

```shell
codex-quiet --no-alt-screen
```

You can also set this permanently:

```toml
[tui]
alternate_screen = "never"
```

### Quiet controls

| Control | Action |
| --- | --- |
| Click a `Work` header | Expand or collapse that group |
| `Alt+I` | Inspect the latest compact group |
| `Alt+O` | Temporarily show all groups, then restore individual folds |
| `Ctrl+T` | Open the complete transcript |
| Mouse drag | Select source text for copying |

## Platform status

Linux x86_64 and macOS on Apple Silicon receive real-machine smoke tests for
this beta. Other published targets have narrower validation. Native Windows is
supported as a beta target, not only through WSL2. See [Support](SUPPORT.md) for
the exact matrix and what each tier means.

No release is claimed to support every Linux distribution, CPU architecture,
terminal emulator, or enterprise policy. If your platform is outside the
matrix, building from source may still work.

## Authentication and safety

Quiet for Codex connects to the same services as the upstream CLI. Sign-in,
subscription eligibility, API billing, model availability, and data handling
are governed by the applicable OpenAI terms and account settings. Refer to the
[official authentication documentation](https://developers.openai.com/codex/auth)
and [agent approvals and security documentation](https://developers.openai.com/codex/agent-approvals-security)
for those service-level details.

Security reports about the fork should follow [SECURITY.md](SECURITY.md).

## Contributing

Bug reports, terminal compatibility reports, documentation fixes, and focused
pull requests are welcome. Read [Contributing](docs/contributing.md) before
opening a change. Fork-specific changes and the upstream boundary are recorded
in [Fork changes](FORK_CHANGES.md).

## License and attribution

The repository is distributed under the [Apache License 2.0](LICENSE). It is a
modified work based on OpenAI's Codex CLI. Original notices are preserved in
[NOTICE](NOTICE), and bundled components are indexed in
[THIRD_PARTY.md](THIRD_PARTY.md).
