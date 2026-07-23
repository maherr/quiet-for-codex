#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch


sys.path.insert(0, str(Path(__file__).resolve().parent))

import smoke_quiet_package


class SmokeQuietPackageCleanupTest(unittest.TestCase):
    def test_retries_a_transient_executable_handle(self) -> None:
        path = Path("smoke-fixture")
        with (
            patch.object(
                smoke_quiet_package.shutil,
                "rmtree",
                side_effect=[PermissionError(32, "in use"), None],
            ) as remove,
            patch.object(smoke_quiet_package.time, "sleep") as sleep,
        ):
            smoke_quiet_package.cleanup_smoke_directory(
                path,
                attempts=2,
                retry_delay_seconds=0.25,
            )

        self.assertEqual(remove.call_count, 2)
        sleep.assert_called_once_with(0.25)

    def test_fails_loud_when_the_handle_never_releases(self) -> None:
        path = Path("smoke-fixture")
        with (
            patch.object(
                smoke_quiet_package.shutil,
                "rmtree",
                side_effect=PermissionError(32, "still in use"),
            ) as remove,
            patch.object(smoke_quiet_package.time, "sleep") as sleep,
            self.assertRaises(PermissionError),
        ):
            smoke_quiet_package.cleanup_smoke_directory(
                path,
                attempts=3,
                retry_delay_seconds=0.25,
            )

        self.assertEqual(remove.call_count, 3)
        self.assertEqual(sleep.call_count, 2)

    def test_missing_directory_is_already_clean(self) -> None:
        path = Path("smoke-fixture")
        with patch.object(
            smoke_quiet_package.shutil,
            "rmtree",
            side_effect=FileNotFoundError,
        ):
            smoke_quiet_package.cleanup_smoke_directory(path)


if __name__ == "__main__":
    unittest.main()
