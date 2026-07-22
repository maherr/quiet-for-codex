#!/usr/bin/env python3
"""Turn an upstream Codex package directory into a Quiet for Codex archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

from codex_package.archive import write_archive  # noqa: E402


RELEASE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$")
TARGET_RE = re.compile(r"^[A-Za-z0-9_.-]+$")
REQUIRED_LEGAL_FILES = ("LICENSE", "NOTICE", "FORK_CHANGES.md", "THIRD_PARTY.md")
RUST_LICENSE_FILENAME = "RUST_DEPENDENCY_LICENSES.txt"
V8_NOTICE_FILENAME = "V8_RUSTY_V8_NOTICES.txt"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Finalize a Quiet for Codex package and write its release archive."
    )
    parser.add_argument("--package-dir", type=Path, required=True)
    parser.add_argument("--archive-output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--rust-licenses", type=Path, required=True)
    parser.add_argument("--v8-notices", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def finalize_package(
    package_dir: Path,
    *,
    version: str,
    target: str,
    repo_root: Path,
    rust_licenses: Path,
    v8_notices: Path,
) -> None:
    validate_release_identity(version, target)
    package_dir = package_dir.resolve()
    repo_root = repo_root.resolve()
    rust_licenses = rust_licenses.resolve()
    v8_notices = v8_notices.resolve()
    metadata_path = package_dir / "codex-package.json"
    if not metadata_path.is_file():
        raise RuntimeError(f"Missing package metadata: {metadata_path}")

    is_windows = target.endswith("-pc-windows-msvc")
    suffix = ".exe" if is_windows else ""
    source_entrypoint = package_dir / "bin" / f"codex{suffix}"
    quiet_entrypoint = package_dir / "bin" / f"codex-quiet{suffix}"
    code_mode_host = package_dir / "bin" / f"codex-code-mode-host{suffix}"

    if quiet_entrypoint.exists() and source_entrypoint.exists():
        raise RuntimeError(
            f"Both source and Quiet entrypoints exist in {package_dir / 'bin'}"
        )
    if source_entrypoint.is_file():
        source_entrypoint.replace(quiet_entrypoint)
    if not quiet_entrypoint.is_file():
        raise RuntimeError(f"Missing Quiet entrypoint: {quiet_entrypoint}")
    if not code_mode_host.is_file():
        raise RuntimeError(f"Missing code-mode host: {code_mode_host}")

    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    upstream_version = metadata.get("version")
    metadata.update(
        {
            "version": version,
            "upstreamVersion": upstream_version,
            "target": target,
            "variant": "codex-quiet",
            "entrypoint": f"bin/codex-quiet{suffix}",
            "project": "codex-quiet",
            "unofficialFork": True,
        }
    )
    metadata_path.write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    for file_name in REQUIRED_LEGAL_FILES:
        source = repo_root / file_name
        if not source.is_file():
            raise RuntimeError(f"Missing required release notice: {source}")
        shutil.copyfile(source, package_dir / file_name)

    copy_third_party_licenses(
        package_dir,
        target=target,
        repo_root=repo_root,
        rust_licenses=rust_licenses,
        v8_notices=v8_notices,
    )

    # The upstream package builder includes a patched zsh executable on Unix.
    # Quiet does not ship it until its exact source and license bundle are tracked.
    bundled_zsh = package_dir / "codex-resources" / "zsh"
    if bundled_zsh.exists():
        shutil.rmtree(bundled_zsh)

    validate_final_package(package_dir, target=target)


def validate_release_identity(version: str, target: str) -> None:
    if RELEASE_VERSION_RE.fullmatch(version) is None:
        raise RuntimeError(f"Invalid Quiet release version: {version}")
    if TARGET_RE.fullmatch(target) is None:
        raise RuntimeError(f"Invalid target triple: {target}")


def copy_third_party_licenses(
    package_dir: Path,
    *,
    target: str,
    repo_root: Path,
    rust_licenses: Path,
    v8_notices: Path,
) -> None:
    if not rust_licenses.is_file() or rust_licenses.stat().st_size < 10_000:
        raise RuntimeError(
            f"Rust dependency license report is missing or unexpectedly small: {rust_licenses}"
        )
    report_prefix = rust_licenses.read_text(encoding="utf-8")[:4096]
    if (
        "Quiet for Codex Rust dependency licenses" not in report_prefix
        or "Used by:" not in report_prefix
    ):
        raise RuntimeError(
            f"Rust dependency license report has an unexpected format: {rust_licenses}"
        )

    if not v8_notices.is_file() or v8_notices.stat().st_size < 100_000:
        raise RuntimeError(
            f"V8 and rusty_v8 notice report is missing or unexpectedly small: {v8_notices}"
        )
    v8_prefix = v8_notices.read_text(encoding="utf-8")[:16_384]
    required_v8_markers = (
        "Quiet for Codex V8 and rusty_v8 third-party notices",
        "rusty_v8 commit:",
        "V8 commit:",
        "Unix artifact build commit:",
        "Component: rusty_v8",
        "Component: V8 JavaScript engine",
        "not legal advice",
    )
    missing_v8_markers = [
        marker for marker in required_v8_markers if marker not in v8_prefix
    ]
    if missing_v8_markers:
        raise RuntimeError(
            "V8 and rusty_v8 notice report has an unexpected format; missing: "
            + ", ".join(missing_v8_markers)
        )

    destination = package_dir / "THIRD_PARTY_LICENSES"
    destination.mkdir()
    shutil.copyfile(rust_licenses, destination / RUST_LICENSE_FILENAME)
    v8_destination = destination / "v8" / V8_NOTICE_FILENAME
    v8_destination.parent.mkdir()
    shutil.copyfile(v8_notices, v8_destination)

    component_licenses = [
        (
            repo_root / "third_party" / "wezterm" / "LICENSE",
            destination / "wezterm" / "LICENSE",
        ),
        (
            repo_root / "scripts" / "release" / "licenses" / "ripgrep-LICENSE-MIT",
            destination / "ripgrep" / "LICENSE-MIT",
        ),
        (
            repo_root / "scripts" / "release" / "licenses" / "ripgrep-UNLICENSE",
            destination / "ripgrep" / "UNLICENSE",
        ),
    ]
    if target.endswith("-unknown-linux-musl"):
        component_licenses.append(
            (
                repo_root / "codex-rs" / "vendor" / "bubblewrap" / "COPYING",
                destination / "bubblewrap" / "COPYING",
            )
        )

    for source, target_path in component_licenses:
        if not source.is_file() or source.stat().st_size == 0:
            raise RuntimeError(f"Missing required component license: {source}")
        target_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target_path)


def validate_final_package(package_dir: Path, *, target: str) -> None:
    suffix = ".exe" if target.endswith("-pc-windows-msvc") else ""
    required = [
        package_dir / "codex-package.json",
        package_dir / "bin" / f"codex-quiet{suffix}",
        package_dir / "bin" / f"codex-code-mode-host{suffix}",
        package_dir / "codex-path" / f"rg{suffix}",
        *(package_dir / file_name for file_name in REQUIRED_LEGAL_FILES),
        package_dir / "THIRD_PARTY_LICENSES" / RUST_LICENSE_FILENAME,
        package_dir / "THIRD_PARTY_LICENSES" / "v8" / V8_NOTICE_FILENAME,
        package_dir / "THIRD_PARTY_LICENSES" / "wezterm" / "LICENSE",
        package_dir / "THIRD_PARTY_LICENSES" / "ripgrep" / "LICENSE-MIT",
        package_dir / "THIRD_PARTY_LICENSES" / "ripgrep" / "UNLICENSE",
    ]
    if target.endswith("-unknown-linux-musl"):
        required.append(package_dir / "codex-resources" / "bwrap")
        required.append(package_dir / "THIRD_PARTY_LICENSES" / "bubblewrap" / "COPYING")
    if target.endswith("-pc-windows-msvc"):
        required.extend(
            [
                package_dir / "codex-resources" / "codex-command-runner.exe",
                package_dir / "codex-resources" / "codex-windows-sandbox-setup.exe",
            ]
        )
    missing = [
        str(path.relative_to(package_dir)) for path in required if not path.is_file()
    ]
    if missing:
        raise RuntimeError(
            f"Final package is missing required files: {', '.join(missing)}"
        )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    args = parse_args()
    package_dir = args.package_dir.resolve()
    archive_output = args.archive_output.resolve()
    finalize_package(
        package_dir,
        version=args.version,
        target=args.target,
        repo_root=args.repo_root,
        rust_licenses=args.rust_licenses,
        v8_notices=args.v8_notices,
    )
    write_archive(package_dir, archive_output, force=args.force)
    print(f"{sha256_file(archive_output)}  {archive_output.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
