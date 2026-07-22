# Support

Quiet for Codex is a public beta. Support tiers describe completed validation
and mandatory release gates, not what Rust can theoretically compile. A target
is not published unless its listed release gate passes for that exact archive.

## Platform matrix

| Platform | Release target | Tier | Validation and release gate |
| --- | --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-musl` | Tested beta | Completed: full TUI suite and real Fedora terminal use. Gate: native GitHub-hosted build, package smoke, and exact-archive POSIX installer smoke |
| macOS Apple Silicon | `aarch64-apple-darwin` | Tested beta | Completed: native M4 release build, command and helper-host smoke, feature probe, and real inline PTY onboarding render. Gate: native GitHub-hosted build, package smoke, and exact-archive POSIX installer smoke |
| macOS Intel | `x86_64-apple-darwin` | CI beta | Completed: cross-built command and helper-host smoke under Rosetta on an M4. Gate: native GitHub-hosted Intel build, package smoke, and exact-archive POSIX installer smoke |
| Windows x86_64 | `x86_64-pc-windows-msvc` | CI beta | Gate: native GitHub-hosted build, package smoke, and exact-archive PowerShell installer smoke. No completed owner-operated interactive TUI acceptance test |
| Linux arm64 | `aarch64-unknown-linux-musl` | CI beta | Gate: native GitHub-hosted arm64 build, package smoke, and exact-archive POSIX installer smoke. No completed owner-operated arm64 Linux test |
| Windows arm64 | `aarch64-pc-windows-msvc` | CI preview | Gate: native GitHub-hosted arm64 build, package smoke, and exact-archive PowerShell installer smoke. No completed owner-operated arm64 Windows test |
| WSL2 x86_64 | Linux x86_64 package | Compatibility path | Uses the Linux package; there is no separate WSL release gate |

The six target archives are published as one release only after every native
build, package command/helper-host smoke, and installer smoke passes. Each
installer test routes the exact finalized archive and generated checksum through
an offline downloader fixture. The POSIX test covers checksum validation,
layout, command and current symlinks, reinstall, version output, and helper-host
EOF. The PowerShell test covers checksum validation, layout, the command shim,
reinstall, and version output. These gates do not exercise GitHub's live asset
delivery, which requires a separate post-publication download probe.

## Not currently supported

- 32-bit operating systems
- Linux architectures other than x86_64 and arm64
- BSD and other non-Linux Unix systems
- macOS versions that cannot run the matching upstream Codex CLI release
- Windows versions or enterprise policies that block unsigned local tools
- The experimental packaged `shell_zsh_fork` backend; Quiet falls back to the
  normal user shell when the shared configuration enables it

The Linux release uses a musl target for broad portability, but it is not a
claim of compatibility with every kernel, container policy, C library setup,
or terminal emulator.

## Terminal compatibility

The retained alternate screen is tested in modern terminal emulators. Terminal
multiplexers, embedded IDE terminals, and unusual mouse protocols can expose
different behavior. Include terminal and multiplexer versions in bug reports.

Use this escape hatch if retained mode is incompatible:

```shell
codex-quiet --no-alt-screen
```

This restores upstream-style inline terminal behavior without changing your
sessions or configuration.

## Unsigned beta binaries

The macOS beta is not Apple-notarized. A browser-downloaded archive may trigger
Gatekeeper. Verify the release checksum first, then use macOS System Settings,
Privacy & Security, Open Anyway if you choose to trust the artifact.

The Windows beta is not Authenticode-signed. SmartScreen or an organization
policy may warn or block it. Verify the release checksum and repository source.
Do not disable a managed security policy to install Quiet for Codex.

Linux, macOS, and Windows archives publish SHA-256 checksums with the release.

## What belongs here

Open a Quiet for Codex issue for:

- retained-screen rendering, scrolling, selection, folding, or resize bugs;
- regressions that occur only in this fork;
- release archive, installer, or platform compatibility problems;
- fork documentation.

Report issues that reproduce unchanged in the matching official CLI to
[`openai/codex`](https://github.com/openai/codex/issues). Report sensitive
security issues according to [SECURITY.md](SECURITY.md), never in a public issue.

## Service support

The fork does not control OpenAI accounts, billing, rate limits, model access,
API availability, or cloud-side behavior. Use official OpenAI support and
documentation for those concerns.
