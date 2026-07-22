#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

from generate_v8_notices import NoticeError  # noqa: E402
from generate_v8_notices import generate_notice  # noqa: E402


class GenerateV8NoticesTest(unittest.TestCase):
    def test_real_manifest_is_deterministic_and_records_exact_provenance(self) -> None:
        first = generate_notice(REPO_ROOT, SCRIPT_DIR / "v8-notices-manifest.json")
        second = generate_notice(REPO_ROOT, SCRIPT_DIR / "v8-notices-manifest.json")

        self.assertEqual(first, second)
        self.assertGreater(len(first.encode("utf-8")), 250_000)
        self.assertIn(
            "rusty_v8 commit: 5d0e31ea6bf67f4559faa759b91e22bc3f1cd696",
            first,
        )
        self.assertIn("V8 commit: 64e3c08f8fbde9e391a490087b317b3c5365f1ba", first)
        self.assertIn(
            "Unix artifact build commit: 1e1b8ed914d7b4aec4d987ffaf3d1c3e97f3fa4d",
            first,
        )
        self.assertIn("Component: libc++abi", first)
        self.assertIn("Component: ICU", first)
        self.assertIn("Some V8 source-tree notices are included conservatively", first)
        self.assertIn("not legal advice", first)

    def test_rejects_cargo_lock_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root, manifest_path = self.create_fixture(Path(temp_dir))
            lock_path = repo_root / "codex-rs" / "Cargo.lock"
            lock_path.write_text(
                lock_path.read_text(encoding="utf-8").replace(
                    'version = "149.2.0"', 'version = "150.0.0"'
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(NoticeError, "Locked v8 drifted"):
                generate_notice(repo_root, manifest_path)

    def test_rejects_source_pin_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root, manifest_path = self.create_fixture(Path(temp_dir))
            (repo_root / "pins.txt").write_text("different\n", encoding="utf-8")
            with self.assertRaisesRegex(NoticeError, "Pinned input drifted"):
                generate_notice(repo_root, manifest_path)

    def test_rejects_license_hash_or_file_set_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            repo_root, manifest_path = self.create_fixture(root)
            license_root = manifest_path.parent / "licenses" / "v8"
            (license_root / "LICENSE").write_text("changed\n", encoding="utf-8")
            with self.assertRaisesRegex(NoticeError, "license hash drifted"):
                generate_notice(repo_root, manifest_path)

            (license_root / "LICENSE").write_text("fixture license\n", encoding="utf-8")
            (license_root / "UNTRACKED").write_text("extra\n", encoding="utf-8")
            with self.assertRaisesRegex(NoticeError, "license set drifted"):
                generate_notice(repo_root, manifest_path)

    def test_rejects_patch_contamination_even_when_hash_is_blessed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            repo_root, manifest_path = self.create_fixture(root)
            license_path = manifest_path.parent / "licenses" / "v8" / "LICENSE"
            contaminated = b"fixture license\n*** Add File: /home/example/next\n"
            license_path.write_bytes(contaminated)
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["components"][0]["licenseFiles"][0]["sha256"] = hashlib.sha256(
                contaminated
            ).hexdigest()
            manifest_path.write_text(
                json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(NoticeError, "workspace contamination"):
                generate_notice(repo_root, manifest_path)

    @staticmethod
    def create_fixture(root: Path) -> tuple[Path, Path]:
        repo_root = root / "repo"
        manifest_dir = root / "manifest"
        (repo_root / "codex-rs").mkdir(parents=True)
        (manifest_dir / "licenses" / "v8").mkdir(parents=True)
        (repo_root / "codex-rs" / "Cargo.lock").write_text(
            "[[package]]\n"
            'name = "v8"\n'
            'version = "149.2.0"\n'
            'checksum = "'
            "46dccf61a364b61bbaac70a8ba64a1a1006e87123b7d62eaeec999a3ba31ecdb"
            '"\n',
            encoding="utf-8",
        )
        (repo_root / "pins.txt").write_text("exact-pin\n", encoding="utf-8")
        license_content = b"fixture license\n"
        (manifest_dir / "licenses" / "v8" / "LICENSE").write_bytes(license_content)
        commit = "1" * 40
        source_url = f"https://example.invalid/source/tree/{commit}"
        manifest = {
            "schemaVersion": 1,
            "licenseDirectory": "licenses/v8",
            "lockedInputs": {
                "rustCrate": {
                    "name": "v8",
                    "version": "149.2.0",
                    "checksum": "46dccf61a364b61bbaac70a8ba64a1a1006e87123b7d62eaeec999a3ba31ecdb",
                    "url": "https://static.crates.io/crates/v8/v8-149.2.0.crate",
                    "lockFile": "codex-rs/Cargo.lock",
                },
                "rustyV8Source": {
                    "tag": "v149.2.0",
                    "commit": commit,
                    "sourceUrl": source_url,
                },
                "v8Source": {
                    "tag": "14.9.207.2",
                    "commit": commit,
                    "sourceUrl": source_url,
                    "archiveIntegrity": "sha256-fixture",
                },
                "unixArtifactBuild": {
                    "tag": "rusty-v8-v149.2.0",
                    "commit": commit,
                    "sourceUrl": source_url,
                    "releaseUrl": "https://example.invalid/release",
                },
                "artifacts": [
                    {
                        "target": "fixture-target",
                        "provider": "fixture",
                        "url": "https://example.invalid/artifact",
                        "sha256": "2" * 64,
                        "bindingUrl": "https://example.invalid/binding",
                        "bindingSha256": "3" * 64,
                    }
                ],
            },
            "requiredPins": [{"path": "pins.txt", "literals": ["exact-pin"]}],
            "components": [
                {
                    "name": "fixture",
                    "version": "1",
                    "scope": "test",
                    "declaredLicense": "fixture terms",
                    "commit": commit,
                    "sourceUrl": source_url,
                    "licenseFiles": [
                        {
                            "path": "LICENSE",
                            "sourcePath": "LICENSE",
                            "sourceUrl": f"https://example.invalid/source/{commit}/LICENSE",
                            "sha256": hashlib.sha256(license_content).hexdigest(),
                        }
                    ],
                }
            ],
        }
        manifest_path = manifest_dir / "manifest.json"
        manifest_path.write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        return repo_root, manifest_path


if __name__ == "__main__":
    unittest.main()
