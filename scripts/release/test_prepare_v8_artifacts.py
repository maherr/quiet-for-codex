#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from prepare_v8_artifacts import ArtifactError  # noqa: E402
from prepare_v8_artifacts import append_github_env  # noqa: E402
from prepare_v8_artifacts import prepare_artifacts  # noqa: E402


def sha256(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


class PrepareV8ArtifactsTest(unittest.TestCase):
    def test_prepares_direct_archive_and_binding_with_manifest_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_content = b"pinned archive"
            binding_content = b"pinned binding"
            archive_source = root / "source.a.gz"
            binding_source = root / "source.rs"
            archive_source.write_bytes(archive_content)
            binding_source.write_bytes(binding_content)
            manifest = self.write_manifest(
                root,
                {
                    "target": "fixture-unix",
                    "url": archive_source.as_uri(),
                    "sha256": sha256(archive_content),
                    "bindingUrl": binding_source.as_uri(),
                    "bindingSha256": sha256(binding_content),
                },
            )

            prepared = prepare_artifacts(manifest, "fixture-unix", root / "output")

            self.assertEqual(prepared.archive.read_bytes(), archive_content)
            self.assertEqual(prepared.binding.read_bytes(), binding_content)
            github_env = root / "github-env"
            append_github_env(github_env, prepared)
            env_text = github_env.read_text(encoding="utf-8")
            self.assertIn(f"RUSTY_V8_ARCHIVE={prepared.archive}\n", env_text)
            self.assertIn(f"RUSTY_V8_SRC_BINDING_PATH={prepared.binding}\n", env_text)

    def test_extracts_windows_binding_from_checksum_pinned_crate(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            version = "149.2.0"
            member_path = "gen/src_binding_release_fixture-windows.rs"
            binding_content = b"windows binding"
            crate_source = root / "v8.crate"
            with tarfile.open(crate_source, "w:gz") as crate:
                info = tarfile.TarInfo(f"v8-{version}/{member_path}")
                info.size = len(binding_content)
                crate.addfile(info, io.BytesIO(binding_content))
            archive_content = b"windows archive"
            archive_source = root / "source.lib.gz"
            archive_source.write_bytes(archive_content)
            manifest = self.write_manifest(
                root,
                {
                    "target": "fixture-windows",
                    "url": archive_source.as_uri(),
                    "sha256": sha256(archive_content),
                    "bindingCratePath": member_path,
                    "bindingSha256": sha256(binding_content),
                },
                crate_url=crate_source.as_uri(),
                crate_sha256=sha256(crate_source.read_bytes()),
            )

            prepared = prepare_artifacts(manifest, "fixture-windows", root / "output")

            self.assertEqual(prepared.archive.read_bytes(), archive_content)
            self.assertEqual(prepared.binding.read_bytes(), binding_content)

    def test_rejects_artifact_hash_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_source = root / "source.a.gz"
            binding_source = root / "source.rs"
            archive_source.write_bytes(b"changed archive")
            binding_source.write_bytes(b"binding")
            manifest = self.write_manifest(
                root,
                {
                    "target": "fixture",
                    "url": archive_source.as_uri(),
                    "sha256": "0" * 64,
                    "bindingUrl": binding_source.as_uri(),
                    "bindingSha256": sha256(b"binding"),
                },
            )

            with self.assertRaisesRegex(ArtifactError, "SHA-256 mismatch"):
                prepare_artifacts(manifest, "fixture", root / "output")

    @staticmethod
    def write_manifest(
        root: Path,
        artifact: dict[str, str],
        *,
        crate_url: str = "https://example.invalid/v8.crate",
        crate_sha256: str = "1" * 64,
    ) -> Path:
        manifest_path = root / "manifest.json"
        manifest_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "lockedInputs": {
                        "rustCrate": {
                            "version": "149.2.0",
                            "url": crate_url,
                            "checksum": crate_sha256,
                        },
                        "artifacts": [artifact],
                    },
                }
            ),
            encoding="utf-8",
        )
        return manifest_path


if __name__ == "__main__":
    unittest.main()
