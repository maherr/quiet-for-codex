#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import sys
import tempfile
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


class SmokeQuietPackageReplayTest(unittest.TestCase):
    def test_paginated_fixture_has_dense_suffix_and_prior_final(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            rollout = smoke_quiet_package.write_paginated_replay_fixture(
                codex_home, root
            )

            records = [
                json.loads(line)
                for line in rollout.read_text(encoding="utf-8").splitlines()
            ]

        self.assertEqual(
            [record["ordinal"] for record in records], list(range(len(records)))
        )
        self.assertEqual(records[0]["payload"]["history_mode"], "paginated")
        serialized = json.dumps(records)
        self.assertIn(smoke_quiet_package.SMOKE_FINAL_MARKER, serialized)
        self.assertIn(smoke_quiet_package.SMOKE_SUFFIX_MARKER, serialized)
        reasoning = [
            record
            for record in records
            if record["payload"].get("item", {}).get("type") == "Reasoning"
        ]
        self.assertEqual(len(reasoning), 240)

    def test_replay_config_is_offline_read_only_and_trusted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real_cwd = root / "private" / "cwd"
            real_cwd.mkdir(parents=True)
            cwd_alias = root / "cwd-alias"
            cwd_alias.symlink_to(real_cwd, target_is_directory=True)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            smoke_quiet_package.write_replay_smoke_config(codex_home, cwd_alias)
            config = (codex_home / "config.toml").read_text(encoding="utf-8")

        self.assertIn('sandbox_mode = "read-only"', config)
        self.assertIn('approval_policy = "never"', config)
        self.assertIn("requires_openai_auth = false", config)
        self.assertIn('trust_level = "trusted"', config)
        self.assertIn(json.dumps(str(real_cwd.resolve())), config)
        self.assertNotIn(json.dumps(str(cwd_alias)), config)

    def test_paginated_fixture_records_the_canonical_cwd(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real_cwd = root / "private" / "cwd"
            real_cwd.mkdir(parents=True)
            cwd_alias = root / "cwd-alias"
            cwd_alias.symlink_to(real_cwd, target_is_directory=True)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            rollout = smoke_quiet_package.write_paginated_replay_fixture(
                codex_home, cwd_alias
            )
            first_record = json.loads(
                rollout.read_text(encoding="utf-8").splitlines()[0]
            )

        self.assertEqual(first_record["payload"]["cwd"], str(real_cwd.resolve()))

    def test_jsonrpc_response_rejects_error_and_non_json_stdout(self) -> None:
        response = smoke_quiet_package.jsonrpc_response(
            '{"id":1,"result":{"ok":true}}\n', 1
        )
        self.assertEqual(response["result"], {"ok": True})
        with self.assertRaises(RuntimeError):
            smoke_quiet_package.jsonrpc_response(
                '{"id":2,"error":{"message":"failed"}}\n', 2
            )
        with self.assertRaises(RuntimeError):
            smoke_quiet_package.jsonrpc_response("not json\n", 3)

    def test_jsonrpc_runner_keeps_stdin_open_for_async_response(self) -> None:
        child = """
import json
import sys
import time

for _ in range(3):
    if not sys.stdin.readline():
        raise SystemExit(9)
time.sleep(0.05)
print(json.dumps({"id": 2, "result": {"restored": True}}), flush=True)
sys.stdin.read()
"""
        with tempfile.TemporaryDirectory() as directory:
            response = smoke_quiet_package.run_jsonrpc_until_response(
                [sys.executable, "-u", "-c", child],
                cwd=Path(directory),
                environment=os.environ.copy(),
                requests=(
                    {"method": "initialize", "id": 1},
                    {"method": "initialized"},
                    {"method": "thread/resume", "id": 2},
                ),
                request_id=2,
                timeout=2,
            )

        self.assertEqual(response["result"], {"restored": True})


if __name__ == "__main__":
    unittest.main()
