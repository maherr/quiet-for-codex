#!/usr/bin/env python3

from __future__ import annotations

import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
ARCHIVER = (
    REPO_ROOT / ".github" / "scripts" / "archive-release-symbols-and-strip-binaries.sh"
)


def bash_executable() -> str:
    if platform.system() != "Windows":
        return "bash"

    program_files = Path(os.environ.get("ProgramFiles", r"C:\Program Files"))
    for candidate in (
        program_files / "Git" / "bin" / "bash.exe",
        program_files / "Git" / "usr" / "bin" / "bash.exe",
    ):
        if candidate.is_file():
            return str(candidate)
    raise FileNotFoundError("Git for Windows bash.exe not found")


def native_linux_target() -> str:
    machine = platform.machine().lower()
    if machine in {"aarch64", "arm64"}:
        return "aarch64-unknown-linux-gnu"
    return "x86_64-unknown-linux-gnu"


class ArchiveReleaseSymbolsTest(unittest.TestCase):
    @unittest.skipUnless(
        platform.system() == "Linux" and Path("/bin/true").is_file(), "Linux fixture"
    )
    def test_linux_archive_matches_build_id_and_records_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = native_linux_target()
            release_dir = root / "release"
            archive_dir = root / "dist"
            runner_temp = root / "runner"
            release_dir.mkdir()
            shutil.copy2("/bin/true", release_dir / "codex")
            environment = os.environ.copy()
            environment.update(
                {"RUNNER_TEMP": str(runner_temp), "GITHUB_SHA": "a" * 40}
            )
            subprocess.run(
                [
                    bash_executable(),
                    str(ARCHIVER),
                    "--target",
                    target,
                    "--artifact-name",
                    f"fixture-{target}",
                    "--release-dir",
                    str(release_dir),
                    "--archive-dir",
                    str(archive_dir),
                    "--binaries",
                    "codex",
                ],
                check=True,
                env=environment,
                text=True,
                timeout=20,
            )
            archive = (
                archive_dir
                / f"codex-symbols-fixture-{target}.tar.gz"
            )
            with tarfile.open(archive, "r:gz") as bundle:
                names = bundle.getnames()
                manifest_member = next(
                    name for name in names if name.endswith("/BUILD_IDS.txt")
                )
                manifest = bundle.extractfile(manifest_member)
                self.assertIsNotNone(manifest)
                manifest_text = manifest.read().decode("utf-8")

        self.assertTrue(any(name.endswith("/codex.debug") for name in names))
        self.assertIn("source=" + "a" * 40, manifest_text)
        self.assertRegex(manifest_text, r"codex build_id=[0-9a-f]+ ")
        self.assertRegex(manifest_text, r"shipped_binary_sha256=[0-9a-f]{64}")
        self.assertRegex(manifest_text, r"debug_sha256=[0-9a-f]{64}")

    @unittest.skipUnless(
        platform.system() == "Linux" and Path("/bin/true").is_file(), "Linux fixture"
    )
    def test_linux_archive_rejects_a_binary_without_a_build_id(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = native_linux_target()
            release_dir = root / "release"
            archive_dir = root / "dist"
            release_dir.mkdir()
            binary = release_dir / "codex"
            shutil.copy2("/bin/true", binary)
            subprocess.run(
                ["objcopy", "--remove-section", ".note.gnu.build-id", str(binary)],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=20,
            )
            result = subprocess.run(
                [
                    bash_executable(),
                    str(ARCHIVER),
                    "--target",
                    target,
                    "--artifact-name",
                    "fixture-missing-build-id",
                    "--release-dir",
                    str(release_dir),
                    "--archive-dir",
                    str(archive_dir),
                    "--binaries",
                    "codex",
                ],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=20,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Release binary has no GNU build ID", result.stderr)

    def test_windows_archive_accepts_rust_normalized_pdb_name(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = "aarch64-pc-windows-msvc"
            release_dir = root / "release"
            archive_dir = root / "dist"
            runner_temp = root / "runner"
            release_dir.mkdir()
            (release_dir / "codex.exe").write_bytes(b"codex executable")
            (release_dir / "codex.pdb").write_bytes(b"codex symbols")
            (release_dir / "codex-code-mode-host.exe").write_bytes(
                b"code mode host executable"
            )
            (release_dir / "codex_code_mode_host.pdb").write_bytes(
                b"code mode host symbols"
            )
            environment = os.environ.copy()
            environment.update(
                {
                    "RUNNER_TEMP": str(runner_temp),
                    "GITHUB_SHA": "b" * 40,
                }
            )
            subprocess.run(
                [
                    bash_executable(),
                    str(ARCHIVER),
                    "--target",
                    target,
                    "--artifact-name",
                    f"fixture-{target}",
                    "--release-dir",
                    str(release_dir),
                    "--archive-dir",
                    str(archive_dir),
                    "--binaries",
                    "codex codex-code-mode-host",
                ],
                check=True,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=20,
            )
            archive = archive_dir / f"codex-symbols-fixture-{target}.tar.gz"
            with tarfile.open(archive, "r:gz") as bundle:
                names = bundle.getnames()
                manifest_member = next(
                    name for name in names if name.endswith("/BUILD_IDS.txt")
                )
                manifest = bundle.extractfile(manifest_member)
                self.assertIsNotNone(manifest)
                manifest_text = manifest.read().decode("utf-8")

        self.assertTrue(any(name.endswith("/codex.pdb") for name in names))
        self.assertTrue(
            any(name.endswith("/codex-code-mode-host.pdb") for name in names)
        )
        self.assertRegex(manifest_text, r"codex binary_sha256=[0-9a-f]{64} ")
        self.assertRegex(
            manifest_text,
            r"codex-code-mode-host binary_sha256=[0-9a-f]{64} ",
        )

    @unittest.skipUnless(platform.system() == "Windows", "Windows Cargo fixture")
    def test_windows_cargo_pdb_naming_and_archive(self) -> None:
        machine = platform.machine().lower()
        target = (
            "aarch64-pc-windows-msvc"
            if machine in {"aarch64", "arm64"}
            else "x86_64-pc-windows-msvc"
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = root / "pdb-fixture"
            source_dir = project / "src"
            target_dir = root / "target"
            archive_dir = root / "dist"
            runner_temp = root / "runner"
            source_dir.mkdir(parents=True)
            (project / "Cargo.toml").write_text(
                """[package]
name = "codex-symbol-fixture"
version = "0.0.0"
edition = "2024"

[[bin]]
name = "codex"
path = "src/codex.rs"

[[bin]]
name = "codex-code-mode-host"
path = "src/codex-code-mode-host.rs"

[profile.release]
debug = "line-tables-only"
""",
                encoding="utf-8",
            )
            (source_dir / "codex.rs").write_text("fn main() {}\n", encoding="utf-8")
            (source_dir / "codex-code-mode-host.rs").write_text(
                "fn main() {}\n", encoding="utf-8"
            )
            subprocess.run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "--target",
                    target,
                    "--target-dir",
                    str(target_dir),
                    "--manifest-path",
                    str(project / "Cargo.toml"),
                ],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=180,
            )
            release_dir = target_dir / target / "release"
            self.assertTrue((release_dir / "codex.pdb").is_file())
            self.assertTrue((release_dir / "codex_code_mode_host.pdb").is_file())
            environment = os.environ.copy()
            environment.update(
                {
                    "RUNNER_TEMP": runner_temp.as_posix(),
                    "GITHUB_SHA": "c" * 40,
                }
            )
            subprocess.run(
                [
                    bash_executable(),
                    str(ARCHIVER),
                    "--target",
                    target,
                    "--artifact-name",
                    f"cargo-fixture-{target}",
                    "--release-dir",
                    release_dir.as_posix(),
                    "--archive-dir",
                    archive_dir.as_posix(),
                    "--binaries",
                    "codex codex-code-mode-host",
                ],
                check=True,
                env=environment,
                text=True,
                timeout=30,
            )
            archive = archive_dir / f"codex-symbols-cargo-fixture-{target}.tar.gz"
            with tarfile.open(archive, "r:gz") as bundle:
                names = bundle.getnames()

        self.assertTrue(any(name.endswith("/codex.pdb") for name in names))
        self.assertTrue(
            any(name.endswith("/codex-code-mode-host.pdb") for name in names)
        )


if __name__ == "__main__":
    unittest.main()
