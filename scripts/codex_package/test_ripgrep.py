#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent))

from codex_package.dotslash import artifact_for_target  # noqa: E402
from codex_package.ripgrep import RG_MANIFEST  # noqa: E402
from codex_package.targets import TARGET_SPECS  # noqa: E402


class RipgrepPackageTest(unittest.TestCase):
    def test_linux_musl_targets_never_select_glibc_ripgrep(self) -> None:
        for target in (
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
        ):
            artifact = artifact_for_target(
                TARGET_SPECS[target], RG_MANIFEST, artifact_label="ripgrep"
            )
            self.assertIsNotNone(artifact)
            assert artifact is not None
            self.assertIn("-unknown-linux-musl", artifact.archive_member)
            self.assertIn("-unknown-linux-musl", artifact.url)
            self.assertNotIn("-unknown-linux-gnu", artifact.archive_member)
            self.assertNotIn("-unknown-linux-gnu", artifact.url)


if __name__ == "__main__":
    unittest.main()
