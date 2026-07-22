# Security Policy

## Supported versions

Security fixes are provided for the latest Quiet for Codex beta release. Older
betas may be asked to upgrade before a report is investigated.

## Report a fork-specific vulnerability

Do not open a public issue for a vulnerability, suspected credential exposure,
or exploit details.

Use GitHub's private vulnerability reporting form:

<https://github.com/maherr/quiet-for-codex/security/advisories/new>

Include the Quiet for Codex version, operating system and architecture,
terminal, reproduction steps, expected impact, and whether the issue also
occurs in the matching upstream Codex CLI release. Remove API keys, access
tokens, account identifiers, session contents, and private source code from the
report.

## Upstream and service vulnerabilities

This fork does not operate OpenAI's authentication, API, model, or cloud
services. A vulnerability that reproduces unchanged in the official Codex CLI
or affects an OpenAI service should be reported through OpenAI's official
[Bugcrowd program](https://bugcrowd.com/engagements/openai), not to this fork.

If the source is unclear, report it privately here and explain the uncertainty.
The maintainer can route it without publishing the details.

## Security boundary

Quiet for Codex preserves upstream approval, sandbox, and network-control
behavior except where a documented fork change says otherwise. The fork
primarily changes terminal presentation, but it still runs a coding agent that
can read files and execute commands under the permissions you grant. Review
approval and sandbox settings before using it on sensitive repositories.

For the underlying security model, read OpenAI's
[agent approvals and security documentation](https://developers.openai.com/codex/agent-approvals-security).

## Release integrity

Quiet for Codex beta artifacts published by this project are attached to
releases in this repository and have SHA-256 checksums. The macOS and Windows
beta artifacts are not yet code-signed. Verify the checksum before running a
downloaded artifact and do not install binaries reposted by third parties.
