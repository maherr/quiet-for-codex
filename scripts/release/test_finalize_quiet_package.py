#!/usr/bin/env python3

from __future__ import annotations

import json
import stat
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))

from finalize_quiet_package import finalize_package
from finalize_quiet_package import validate_release_identity


class FinalizeQuietPackageTest(unittest.TestCase):
    def test_finalizes_unix_package(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package_dir = self.create_fixture(root, windows=False)
            rust_licenses = self.create_license_fixture(root)
            v8_notices = self.create_v8_notice_fixture(root)
            finalize_package(
                package_dir,
                version="0.145.0-beta.1",
                target="aarch64-apple-darwin",
                repo_root=root,
                rust_licenses=rust_licenses,
                v8_notices=v8_notices,
            )

            quiet = package_dir / "bin" / "codex-quiet"
            self.assertTrue(quiet.is_file())
            self.assertFalse((package_dir / "bin" / "codex").exists())
            self.assertFalse((package_dir / "codex-resources" / "zsh").exists())
            metadata = json.loads(
                (package_dir / "codex-package.json").read_text(encoding="utf-8")
            )
            self.assertEqual(metadata["entrypoint"], "bin/codex-quiet")
            self.assertEqual(metadata["upstreamVersion"], "0.145.0")
            self.assertTrue(metadata["unofficialFork"])
            bundled_notices = (
                package_dir / "THIRD_PARTY_LICENSES" / "v8" / "V8_RUSTY_V8_NOTICES.txt"
            )
            self.assertEqual(bundled_notices.read_bytes(), v8_notices.read_bytes())

    def test_finalizes_windows_package(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package_dir = self.create_fixture(root, windows=True)
            rust_licenses = self.create_license_fixture(root)
            v8_notices = self.create_v8_notice_fixture(root)
            finalize_package(
                package_dir,
                version="0.145.0-beta.1",
                target="aarch64-pc-windows-msvc",
                repo_root=root,
                rust_licenses=rust_licenses,
                v8_notices=v8_notices,
            )

            self.assertTrue((package_dir / "bin" / "codex-quiet.exe").is_file())
            self.assertTrue(
                (package_dir / "codex-resources" / "codex-command-runner.exe").is_file()
            )

    def test_rejects_unvalidated_v8_notice_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package_dir = self.create_fixture(root, windows=False)
            rust_licenses = self.create_license_fixture(root)
            invalid_notices = root / "V8_RUSTY_V8_NOTICES.txt"
            invalid_notices.write_text("not a notice\n" + "x" * 100_000)
            with self.assertRaisesRegex(RuntimeError, "unexpected format"):
                finalize_package(
                    package_dir,
                    version="0.145.0-beta.1",
                    target="aarch64-apple-darwin",
                    repo_root=root,
                    rust_licenses=rust_licenses,
                    v8_notices=invalid_notices,
                )

    def test_rejects_invalid_release_identity(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Invalid Quiet release version"):
            validate_release_identity("latest", "x86_64-unknown-linux-musl")
        with self.assertRaisesRegex(RuntimeError, "Invalid Quiet release version"):
            validate_release_identity("0.145.0", "x86_64-unknown-linux-musl")
        with self.assertRaisesRegex(RuntimeError, "Invalid Quiet release version"):
            validate_release_identity("0.145.0-beta", "x86_64-unknown-linux-musl")
        with self.assertRaisesRegex(RuntimeError, "Invalid Quiet release version"):
            validate_release_identity("0.145.0-beta.01", "x86_64-unknown-linux-musl")
        validate_release_identity("0.145.0-beta.2", "x86_64-unknown-linux-musl")
        with self.assertRaisesRegex(RuntimeError, "Invalid target triple"):
            validate_release_identity("0.145.0-beta.1", "../linux")

    def create_fixture(self, root: Path, *, windows: bool) -> Path:
        suffix = ".exe" if windows else ""
        package_dir = root / "package"
        bin_dir = package_dir / "bin"
        resource_dir = package_dir / "codex-resources"
        path_dir = package_dir / "codex-path"
        bin_dir.mkdir(parents=True)
        resource_dir.mkdir()
        path_dir.mkdir()
        zsh_dir = resource_dir / "zsh" / "bin"
        zsh_dir.mkdir(parents=True)
        self.write_executable(zsh_dir / f"zsh{suffix}")
        self.write_executable(bin_dir / f"codex{suffix}")
        self.write_executable(bin_dir / f"codex-code-mode-host{suffix}")
        self.write_executable(path_dir / f"rg{suffix}")
        if windows:
            self.write_executable(resource_dir / "codex-command-runner.exe")
            self.write_executable(resource_dir / "codex-windows-sandbox-setup.exe")
        (package_dir / "codex-package.json").write_text(
            json.dumps(
                {
                    "layoutVersion": 1,
                    "version": "0.145.0",
                    "target": "fixture",
                    "variant": "codex",
                    "entrypoint": f"bin/codex{suffix}",
                }
            ),
            encoding="utf-8",
        )
        for file_name in ("LICENSE", "NOTICE", "FORK_CHANGES.md", "THIRD_PARTY.md"):
            (root / file_name).write_text(file_name + "\n", encoding="utf-8")
        return package_dir

    @staticmethod
    def create_license_fixture(root: Path) -> Path:
        component_files = [
            root / "third_party" / "wezterm" / "LICENSE",
            root / "codex-rs" / "vendor" / "bubblewrap" / "COPYING",
            root / "scripts" / "release" / "licenses" / "ripgrep-LICENSE-MIT",
            root / "scripts" / "release" / "licenses" / "ripgrep-UNLICENSE",
        ]
        for path in component_files:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("license\n", encoding="utf-8")
        report = root / "RUST_DEPENDENCY_LICENSES.txt"
        report.write_text(
            "Quiet for Codex Rust dependency licenses\n\nUsed by:\n" + "x" * 10_000,
            encoding="utf-8",
        )
        return report

    @staticmethod
    def create_v8_notice_fixture(root: Path) -> Path:
        report = root / "V8_RUSTY_V8_NOTICES.txt"
        report.write_text(
            "Quiet for Codex V8 and rusty_v8 third-party notices\n"
            "rusty_v8 commit: "
            "5d0e31ea6bf67f4559faa759b91e22bc3f1cd696\n"
            "V8 commit: 64e3c08f8fbde9e391a490087b317b3c5365f1ba\n"
            "Unix artifact build commit: "
            "1e1b8ed914d7b4aec4d987ffaf3d1c3e97f3fa4d\n"
            "This inventory is not legal advice.\n"
            "Component: rusty_v8\n"
            "Component: V8 JavaScript engine\n" + "x" * 100_000,
            encoding="utf-8",
        )
        return report

    @staticmethod
    def write_executable(path: Path) -> None:
        path.write_bytes(b"fixture")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)


if __name__ == "__main__":
    unittest.main()
