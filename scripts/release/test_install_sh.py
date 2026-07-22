#!/usr/bin/env python3
"""Offline end-to-end test for the POSIX Quiet for Codex installer."""

from __future__ import annotations

import hashlib
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPO_ROOT / "scripts" / "release" / "install.sh"
VERSION = "0.145.0-beta.1"


def native_target() -> str:
    key = (platform.system(), platform.machine().lower())
    targets = {
        ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
        ("Linux", "amd64"): "x86_64-unknown-linux-musl",
        ("Linux", "aarch64"): "aarch64-unknown-linux-musl",
        ("Linux", "arm64"): "aarch64-unknown-linux-musl",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Darwin", "amd64"): "x86_64-apple-darwin",
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "aarch64"): "aarch64-apple-darwin",
    }
    try:
        return targets[key]
    except KeyError as error:
        raise RuntimeError(f"Unsupported installer test platform: {key}") from error


CURL_FIXTURE = r"""#!/bin/sh
set -eu

url=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      output="$2"
      shift
      ;;
    -*) ;;
    *) url="$1" ;;
  esac
  shift
done

printf '%s\n' "$url" >> "$QUIET_TEST_URL_LOG"
case "$url" in
  */releases\?per_page=20) cat "$QUIET_TEST_FIXTURES/releases.json" ;;
  */SHA256SUMS) cp "$QUIET_TEST_FIXTURES/SHA256SUMS" "$output" ;;
  */codex-quiet-*.tar.gz) cp "$QUIET_TEST_FIXTURES/asset.tar.gz" "$output" ;;
  *)
    printf 'Unexpected fixture URL: %s\n' "$url" >&2
    exit 1
    ;;
esac
"""


class PosixInstallerTests(unittest.TestCase):
    def test_checksum_install_layout_and_reinstall(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            version = os.environ.get("QUIET_INSTALL_TEST_VERSION", VERSION)
            target = native_target()
            asset_name = f"codex-quiet-{version}-{target}.tar.gz"
            supplied_archive = os.environ.get("QUIET_INSTALL_TEST_ARCHIVE")
            fixtures = root / "fixtures"
            package = root / "package"
            mock_bin = root / "mock-bin"
            home = root / "home"
            install_root = home / ".local" / "share" / "codex-quiet"
            bin_dir = home / ".local" / "bin"
            url_log = root / "urls.log"
            fixtures.mkdir()
            mock_bin.mkdir()

            archive = fixtures / "asset.tar.gz"
            if supplied_archive:
                supplied_path = Path(supplied_archive).resolve()
                if supplied_path.name != asset_name:
                    self.fail(
                        f"Expected release archive {asset_name}, got {supplied_path.name}"
                    )
                shutil.copyfile(supplied_path, archive)
            else:
                self.write_executable(
                    package / "bin" / "codex-quiet",
                    "#!/bin/sh\nprintf '%s\\n' 'codex-quiet fixture'\n",
                )
                self.write_executable(
                    package / "bin" / "codex-code-mode-host",
                    "#!/bin/sh\nexit 0\n",
                )
                with tarfile.open(archive, "w:gz") as output:
                    output.add(package / "bin", arcname="bin")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (fixtures / "SHA256SUMS").write_text(
                f"{digest}  {asset_name}\n", encoding="utf-8"
            )
            (fixtures / "releases.json").write_text(
                '[{"tag_name":"v999.0.0","prerelease":false},'
                f'{{"tag_name":"quiet-v{version}","prerelease":true}}]\n',
                encoding="utf-8",
            )
            self.write_executable(mock_bin / "curl", CURL_FIXTURE)

            env = {
                **os.environ,
                "PATH": f"{mock_bin}{os.pathsep}{os.environ['PATH']}",
                "HOME": str(home),
                "CODEX_QUIET_INSTALL_ROOT": str(install_root),
                "CODEX_QUIET_BIN_DIR": str(bin_dir),
                "QUIET_TEST_FIXTURES": str(fixtures),
                "QUIET_TEST_URL_LOG": str(url_log),
            }

            env.pop("CODEX_QUIET_RELEASE", None)
            for release in (None, version):
                if release is not None:
                    env["CODEX_QUIET_RELEASE"] = release
                subprocess.run(
                    ["/bin/sh", str(INSTALLER)],
                    check=True,
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )

            visible_command = bin_dir / "codex-quiet"
            self.assertTrue(visible_command.is_symlink())
            version_output = subprocess.run(
                [str(visible_command), "--version"],
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip()
            self.assertIn("codex", version_output.lower())
            subprocess.run(
                [str(install_root / "current" / "bin" / "codex-code-mode-host")],
                check=True,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=20,
            )
            self.assertTrue((install_root / "current").is_symlink())
            self.assertFalse((install_root / "current" / "bin" / "codex").exists())

            expected_base = (
                "https://github.com/maherr/quiet-for-codex/releases/download/"
                f"quiet-v{version}"
            )
            urls = url_log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(
                urls,
                [
                    "https://api.github.com/repos/maherr/quiet-for-codex/releases?per_page=20",
                    f"{expected_base}/{asset_name}",
                    f"{expected_base}/SHA256SUMS",
                    f"{expected_base}/{asset_name}",
                    f"{expected_base}/SHA256SUMS",
                ],
            )

    @staticmethod
    def write_executable(path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        path.chmod(0o755)


if __name__ == "__main__":
    unittest.main()
