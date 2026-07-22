#!/usr/bin/env python3
"""Reject stock executable, URI, and upload claims from embedded Quiet UI."""

from __future__ import annotations

import re
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
ASSET_ROOT = REPO_ROOT / "codex-rs" / "skills" / "src" / "assets"
TUI_SOURCE_ROOT = REPO_ROOT / "codex-rs" / "tui" / "src"
TUI_TOOLTIPS = REPO_ROOT / "codex-rs" / "tui" / "tooltips.txt"
STOCK_COMMAND_RE = re.compile(
    r"(?<![\w-])codex[ \t]+(?:"
    r"app|plugin|mcp|app-server|exec|resume|login|logout|sandbox|cloud|"
    r"completion|doctor|features|remote-control"
    r")\b"
)
STOCK_TUI_APP_NAME_RE = re.compile(
    r'(?:StatusSurfacePreviewItem|TerminalTitleItem)::AppName\s*=>[^\n]*"codex"'
    r'|"Codex app name"'
)


def main() -> int:
    if not ASSET_ROOT.is_dir():
        raise SystemExit(f"embedded skill asset directory is missing: {ASSET_ROOT}")
    files = sorted(path for path in ASSET_ROOT.rglob("*") if path.is_file())
    if not files:
        raise SystemExit(f"embedded skill asset directory is empty: {ASSET_ROOT}")
    tui_sources = sorted(TUI_SOURCE_ROOT.rglob("*.rs"))
    if not tui_sources:
        raise SystemExit(f"compiled TUI source directory is empty: {TUI_SOURCE_ROOT}")
    if not TUI_TOOLTIPS.is_file():
        raise SystemExit(f"compiled TUI tooltip file is missing: {TUI_TOOLTIPS}")

    # Positive controls keep a broken or over-broad clean sweep from passing.
    if STOCK_COMMAND_RE.search("run codex plugin list") is None:
        raise SystemExit("stock executable detector failed its positive control")
    if STOCK_COMMAND_RE.search("run codex-quiet plugin list") is not None:
        raise SystemExit("stock executable detector rejects codex-quiet")
    if (
        STOCK_TUI_APP_NAME_RE.search(
            'TerminalTitleItem::AppName => Some("codex".to_string())'
        )
        is None
    ):
        raise SystemExit("stock TUI app-name detector failed its positive control")
    if (
        STOCK_TUI_APP_NAME_RE.search(
            "TerminalTitleItem::AppName => CODEX_CLI_DISPLAY_NAME"
        )
        is not None
    ):
        raise SystemExit("stock TUI app-name detector rejects Quiet identity")

    violations: list[str] = []
    for path in [*files, TUI_TOOLTIPS]:
        content = path.read_bytes().decode("utf-8", errors="ignore")
        for line_number, line in enumerate(content.splitlines(), start=1):
            if "codex://" in line:
                violations.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: stock codex:// URI"
                )
            if STOCK_COMMAND_RE.search(line) is not None:
                violations.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: stock codex executable"
                )
            if path == TUI_TOOLTIPS and "/feedback" in line:
                violations.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: hidden /feedback command"
                )

    for path in tui_sources:
        content = path.read_text(encoding="utf-8")
        for line_number, line in enumerate(content.splitlines(), start=1):
            if "codex://" in line:
                violations.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: stock codex:// URI"
                )
            if STOCK_TUI_APP_NAME_RE.search(line) is not None:
                violations.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: stock CLI app name"
                )

    if violations:
        print("Embedded Quiet skills contain stock Codex identity examples:")
        print("\n".join(violations))
        return 1

    print(
        f"Quiet identity check passed across {len(files)} embedded skill files, "
        f"{len(tui_sources)} compiled TUI sources, and tooltips: no stock "
        "codex:// URIs, executable examples, CLI app names, or /feedback "
        "upload claim."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
