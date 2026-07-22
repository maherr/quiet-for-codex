# Fork changes

Quiet for Codex is derived from OpenAI's Codex CLI and is maintained as an
independent, unofficial fork.

## Current upstream base

- Upstream repository: <https://github.com/openai/codex>
- Base release: `rust-v0.145.0`
- Base commit: `25af12f7e61572b0bc18ddb1008be543b91519b0`

Git history is the authoritative record of modifications. To list every path
that differs from the current base:

```shell
git diff --name-status 25af12f7e61572b0bc18ddb1008be543b91519b0...HEAD
```

Unless a file states otherwise, paths in that diff have been modified or added
for this fork after the upstream base. Original copyright, license, and notice
text is retained.

## Deliberate differences

### Retained terminal surface

The TUI can own an alternate screen with a pinned composer and retained,
scrollable conversation history. It maps rendered cells back to source text so
mouse selection and copying remain useful after wrapping and resize.

### Compact activity presentation

Successful commands can fold into outcome-first `Work` groups. Failures and
results requiring attention remain expanded. Background terminals, hooks, and
collaborator activity use compact lifecycle presentation.

### Conversation panes and inspection

Conversation panes can remain live, receive focus, resize, and preserve their
own navigation state. Compact work can be inspected in place or through the
complete transcript.

### Fork identity and distribution

The display version identifies `codex-quiet`. Public packages install a
`codex-quiet` command beside the official `codex` command. Fork releases use
their own repository, checksums, installers, support policy, and update channel.
Quiet disables the upstream self-update path and remote announcement feed. It
also omits the `app` CLI subcommand and hides `/app`, so this beta cannot
download, install, or hand a session to the official Desktop app. Browser login
success stays on the local confirmation page, including when an app-server
client requests the upstream hosted Desktop handoff.

Quiet does not send feedback or logs to OpenAI's upstream Sentry endpoint. The
TUI command is hidden, the app-server upload method fails closed, and support
is routed through this repository's issue tracker.

Daemon-managed app-server and daemon-backed remote-control commands are hidden
and fail closed. The matching upstream routes manage a stock standalone Codex
installation and updater, which would cross Quiet's separate-install and
separate-update boundary. Foreground app-server operation remains available.
Binary beta packages omit the experimental patched-zsh payload. A shared
configuration that enables the zsh-fork feature falls back to the normal user
shell instead of failing startup.

## Compatibility intent

The fork aims to preserve upstream protocol, authentication, configuration,
session, tool, and safety behavior for the matching base release. TUI behavior
is allowed to diverge where needed to provide the retained Quiet interface.

`--no-alt-screen` and `tui.alternate_screen = "never"` remain available as a
compatibility escape hatch.

## Upstream sync policy

Each Quiet release names its exact upstream base. Upstream updates are merged or
reapplied in a dedicated version update, followed by the TUI suite, snapshot
review, platform builds, and real-machine smoke tests where available.

An upstream bug that reproduces unchanged is not represented as a Quiet fix.
Fork-specific workarounds should link the upstream issue and state why local
divergence is necessary.
