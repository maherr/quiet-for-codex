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
                    "bash",
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
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
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
                    "bash",
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


if __name__ == "__main__":
    unittest.main()
