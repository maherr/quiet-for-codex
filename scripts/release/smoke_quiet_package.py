#!/usr/bin/env python3
"""Run noninteractive launch checks against a finalized Quiet for Codex package."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path


QUIET_DAEMON_DISABLED_MESSAGE = (
    "daemon-managed app-server routes are disabled in Quiet for Codex because "
    "the upstream implementation installs and updates stock Codex"
)
DISABLED_DAEMON_ROUTES = (
    ("app-server", "daemon", "bootstrap"),
    ("app-server", "daemon", "start"),
    ("app-server", "daemon", "stop"),
    ("app-server", "daemon", "restart"),
    ("app-server", "daemon", "enable-remote-control"),
    ("app-server", "daemon", "disable-remote-control"),
    ("app-server", "daemon", "pid-update-loop"),
    ("app-server", "daemon", "version"),
    ("remote-control",),
    ("remote-control", "start"),
    ("remote-control", "stop"),
    ("remote-control", "pair"),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Smoke test a Quiet for Codex package."
    )
    parser.add_argument("archive", type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    return parser.parse_args()


def run_checked(
    command: list[str], *, stdin_empty: bool = False
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=True,
        stdin=subprocess.DEVNULL if stdin_empty else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
    )


def run_captured(
    command: list[str], *, environment: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
        env=environment,
    )


def assert_disabled_daemon_route(
    quiet: Path, route: tuple[str, ...], *, codex_home: Path
) -> None:
    environment = os.environ.copy()
    environment["CODEX_HOME"] = str(codex_home)
    result = run_captured([str(quiet), *route], environment=environment)
    output = result.stdout + result.stderr
    route_text = " ".join(route)
    if result.returncode == 0:
        raise RuntimeError(f"Disabled route unexpectedly succeeded: {route_text}")
    if QUIET_DAEMON_DISABLED_MESSAGE not in output:
        raise RuntimeError(
            f"Disabled route did not return the Quiet safety error: {route_text}\n"
            f"stdout: {result.stdout!r}\nstderr: {result.stderr!r}"
        )


def assert_fast_failure(binary: Path, expected_message: str) -> None:
    result = run_captured([str(binary)])
    output = result.stdout + result.stderr
    if result.returncode == 0:
        raise RuntimeError(
            f"Bundled helper unexpectedly succeeded without input: {binary}"
        )
    if expected_message not in output:
        raise RuntimeError(
            f"Bundled helper did not return {expected_message!r}: {binary}\n"
            f"stdout: {result.stdout!r}\nstderr: {result.stderr!r}"
        )


def extract_archive(archive: Path, destination: Path) -> None:
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(destination)
        return
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive, "r:gz") as bundle:
            bundle.extractall(destination)
        return
    raise RuntimeError(f"Unsupported package archive: {archive}")


def smoke_package(args: argparse.Namespace, package_dir: Path) -> None:
    suffix = ".exe" if args.target.endswith("-pc-windows-msvc") else ""
    quiet = package_dir / "bin" / f"codex-quiet{suffix}"
    host = package_dir / "bin" / f"codex-code-mode-host{suffix}"
    ripgrep = package_dir / "codex-path" / f"rg{suffix}"
    metadata_path = package_dir / "codex-package.json"
    v8_notices = package_dir / "THIRD_PARTY_LICENSES" / "v8" / "V8_RUSTY_V8_NOTICES.txt"

    if not v8_notices.is_file() or v8_notices.stat().st_size < 100_000:
        raise RuntimeError("Archive is missing the generated V8 and rusty_v8 notices.")
    if not v8_notices.read_text(encoding="utf-8").startswith(
        "Quiet for Codex V8 and rusty_v8 third-party notices\n"
    ):
        raise RuntimeError("Archive contains an invalid V8 and rusty_v8 notice report.")

    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    expected_entrypoint = f"bin/codex-quiet{suffix}"
    if metadata.get("entrypoint") != expected_entrypoint:
        raise RuntimeError(
            f"Expected entrypoint {expected_entrypoint!r}, got {metadata.get('entrypoint')!r}"
        )
    if metadata.get("unofficialFork") is not True:
        raise RuntimeError("Package metadata does not identify the unofficial fork.")

    version = run_checked([str(quiet), "--version"])
    expected_version = f"codex-quiet {args.version}"
    if version.stdout.strip() != expected_version:
        raise RuntimeError(
            f"Expected version output {expected_version!r}, got {version.stdout!r}"
        )
    help_result = run_checked([str(quiet), "--help"])
    if "usage" not in help_result.stdout.lower():
        raise RuntimeError("Help output has no usage line.")
    if "\n  app " in help_result.stdout.lower():
        raise RuntimeError("Help output exposes the disabled stock Desktop app route.")
    smoke_codex_home = package_dir / ".smoke-codex-home"
    smoke_codex_home.mkdir()
    for route in DISABLED_DAEMON_ROUTES:
        assert_disabled_daemon_route(quiet, route, codex_home=smoke_codex_home)
    run_checked([str(host)], stdin_empty=True)
    ripgrep_version = run_checked([str(ripgrep), "--version"])
    if not ripgrep_version.stdout.lower().startswith("ripgrep "):
        raise RuntimeError(
            f"Bundled ripgrep returned unexpected version output: {ripgrep_version.stdout!r}"
        )
    if args.target.endswith("-unknown-linux-musl"):
        bwrap = package_dir / "codex-resources" / "bwrap"
        bwrap_version = run_checked([str(bwrap), "--version"])
        if "bubblewrap " not in bwrap_version.stdout.lower():
            raise RuntimeError(
                f"Bundled bubblewrap returned unexpected version output: {bwrap_version.stdout!r}"
            )
    if args.target.endswith("-pc-windows-msvc"):
        assert_fast_failure(
            package_dir / "codex-resources" / "codex-windows-sandbox-setup.exe",
            "expected payload argument",
        )
        assert_fast_failure(
            package_dir / "codex-resources" / "codex-command-runner.exe",
            "no pipe-in provided",
        )

    print(version.stdout.strip())
    print(f"Smoke checks passed for {args.target}")


def main() -> int:
    args = parse_args()
    archive = args.archive.resolve()
    if not archive.is_file():
        raise RuntimeError(f"Package archive does not exist: {archive}")
    with tempfile.TemporaryDirectory(prefix="codex-quiet-smoke-") as temp_dir:
        package_dir = Path(temp_dir)
        extract_archive(archive, package_dir)
        smoke_package(args, package_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
