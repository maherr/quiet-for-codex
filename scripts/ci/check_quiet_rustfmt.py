#!/usr/bin/env python3
"""Check rustfmt only for Rust files owned by the Quiet fork delta."""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import NoReturn


REPO_ROOT = Path(__file__).resolve().parents[2]
FORK_CHANGES = REPO_ROOT / "FORK_CHANGES.md"
RUSTFMT_CONFIG = Path(__file__).with_name("rustfmt.toml")
BASE_COMMIT_RE = re.compile(r"^- Base commit: `([0-9a-f]{40})`$", re.MULTILINE)


def fail(message: str) -> NoReturn:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def documented_upstream_base() -> str:
    match = BASE_COMMIT_RE.search(FORK_CHANGES.read_text(encoding="utf-8"))
    if match is None:
        fail(f"could not find the upstream base commit in {FORK_CHANGES}")
    return match.group(1)


def git_output(*args: str) -> bytes:
    result = subprocess.run(
        ["git", *args],
        cwd=REPO_ROOT,
        check=False,
        stdout=subprocess.PIPE,
    )
    if result.returncode != 0:
        fail(f"git {' '.join(args)} failed with exit code {result.returncode}")
    return result.stdout


def null_paths(output: bytes) -> set[str]:
    return {
        value.decode("utf-8", errors="surrogateescape")
        for value in output.split(b"\0")
        if value
    }


def quiet_rust_paths(base_commit: str) -> list[str]:
    sources = (
        git_output(
            "diff",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
            f"{base_commit}...HEAD",
            "--",
            "codex-rs",
        ),
        git_output(
            "diff",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
            "--cached",
            "--",
            "codex-rs",
        ),
        git_output(
            "diff",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
            "--",
            "codex-rs",
        ),
        git_output(
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "codex-rs",
        ),
    )
    candidates = set().union(*(null_paths(source) for source in sources))
    return sorted(
        path
        for path in candidates
        if path.endswith(".rs") and (REPO_ROOT / path).is_file()
    )


def verify_base(base_commit: str) -> None:
    git_output("rev-parse", "--verify", f"{base_commit}^{{commit}}")
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", base_commit, "HEAD"],
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        fail(
            "the documented upstream base is not an ancestor of HEAD; "
            "update FORK_CHANGES.md when rebasing Quiet"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--list",
        action="store_true",
        help="print the selected Rust paths without running rustfmt",
    )
    args = parser.parse_args()

    base_commit = documented_upstream_base()
    verify_base(base_commit)
    paths = quiet_rust_paths(base_commit)
    if args.list:
        print("\n".join(paths))
        return 0

    rustfmt = shutil.which("rustfmt")
    if rustfmt is None:
        fail("rustfmt is not installed")
    if not paths:
        print(f"No Quiet-owned Rust files differ from {base_commit[:12]}.")
        return 0

    print(
        f"Checking {len(paths)} Quiet-owned Rust files against "
        f"upstream base {base_commit[:12]}."
    )
    result = subprocess.run(
        [
            rustfmt,
            "--check",
            "--config-path",
            str(RUSTFMT_CONFIG),
            "--config",
            "skip_children=true",
            *paths,
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
