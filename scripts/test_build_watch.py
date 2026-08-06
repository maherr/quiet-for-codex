import contextlib
import io
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import scripts.build_watch as build_watch
from scripts.build_watch import OutputFacts
from scripts.build_watch import ProcInfo
from scripts.build_watch import choose_build_root
from scripts.build_watch import command_exit_code
from scripts.build_watch import estimate_eta
from scripts.build_watch import format_bytes
from scripts.build_watch import format_duration
from scripts.build_watch import normalize_run_command
from scripts.build_watch import save_history
from scripts.build_watch import sparkline
from scripts.build_watch import wait_until_finished


def proc(pid: int, ppid: int, command: tuple[str, ...], start: int) -> ProcInfo:
    return ProcInfo(
        pid=pid,
        ppid=ppid,
        started_at=start,
        cpu_seconds=0,
        rss_bytes=0,
        comm=command[0],
        argv=command,
        cwd=Path("/repo"),
    )


class BuildWatchTests(unittest.TestCase):
    def test_output_facts_parse_nextest_progress(self) -> None:
        facts = OutputFacts()
        lines = (
            "   Compiling codex-tui v0.1.0\n",
            "warning: harmless fixture\n",
            "    Finished `test` profile [unoptimized] target(s) in 2m\n",
            "    Starting 3577 tests across 180 binaries (4 skipped)\n",
            "        PASS [ 0.010s] codex-tui::one\n",
            "        PASS [ 0.020s] codex-tui::two\n",
            "Summary [ 9.2s] 3577 tests run: 3577 passed, 4 skipped\n",
        )
        for line in lines:
            facts.consume(line)
        self.assertEqual(facts.tests_total, 3577)
        self.assertEqual(facts.tests_done, 3577)
        self.assertTrue(facts.saw_build_finished)
        self.assertTrue(facts.saw_test_summary)

    def test_output_facts_handle_singular_binary_and_slow_as_nonterminal(self) -> None:
        facts = OutputFacts()
        for line in (
            "Starting 1 test across 1 binary (10 skipped)\n",
            "SLOW [ 1.000s] codex-tui::one\n",
            "PASS [ 2.000s] codex-tui::one\n",
            "Summary [ 2.1s] 1 test run: 1 passed\n",
        ):
            facts.consume(line)
        self.assertEqual((facts.tests_done, facts.tests_total), (1, 1))

    def test_choose_build_root_prefers_parent_and_newest_run(self) -> None:
        processes = {
            10: proc(10, 1, ("just", "test"), 100),
            11: proc(11, 10, ("cargo", "nextest", "run"), 101),
            20: proc(20, 1, ("just", "test"), 200),
        }
        self.assertEqual(choose_build_root(processes), processes[20])

    def test_eta_uses_live_test_progress_before_history(self) -> None:
        facts = OutputFacts(tests_done=50, tests_total=100)
        eta, basis, progress = estimate_eta(
            elapsed=120,
            phase="TEST",
            facts=facts,
            history_rows=({"duration": 999, "exit_code": 0},),
        )
        self.assertEqual(
            (eta, basis, progress), ("~2m 00s", "live test throughput", 0.5)
        )

    def test_test_eta_excludes_compile_time(self) -> None:
        facts = OutputFacts(tests_done=1, tests_total=100)
        eta, basis, progress = estimate_eta(
            elapsed=1_200,
            phase="TEST",
            facts=facts,
            history_rows=(),
            test_elapsed=10,
        )
        self.assertEqual(
            (eta, basis, progress), ("~16m 30s", "live test throughput", 0.01)
        )

    def test_eta_is_explicitly_calibrating_without_evidence(self) -> None:
        eta, basis, progress = estimate_eta(120, "BUILD", OutputFacts(), ())
        self.assertEqual(eta, "CALIBRATING")
        self.assertIn("first completed run", basis)
        self.assertIsNone(progress)

    def test_history_eta_reports_overrun_instead_of_zero_forever(self) -> None:
        eta, basis, progress = estimate_eta(
            125,
            "BUILD",
            OutputFacts(),
            ({"duration": 100, "exit_code": 0},),
        )
        self.assertEqual((eta, basis, progress), ("OVERRUN", "25s past median", 0.97))

    def test_history_is_bounded_and_atomic_shape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "history.json"
            for duration in range(14):
                save_history("key", duration, 0, path)
            rows = __import__("json").loads(path.read_text())["key"]
            self.assertEqual(len(rows), 12)
            self.assertEqual(rows[0]["duration"], 2)

    def test_formatting_and_visual_primitives(self) -> None:
        self.assertEqual(format_duration(3661), "1h 01m")
        self.assertEqual(format_bytes(1024**3), "1.0 GiB")
        self.assertEqual(len(sparkline((0, 1, 2, 3), 6)), 6)

    def test_owned_command_exit_code_is_preserved(self) -> None:
        self.assertEqual(command_exit_code(None), 0)
        self.assertEqual(command_exit_code(95), 95)
        self.assertEqual(command_exit_code(-9), 137)

    def test_only_leading_run_separator_is_removed(self) -> None:
        self.assertEqual(
            normalize_run_command(("--", "cmd", "--", "value")),
            ["cmd", "--", "value"],
        )

    def test_noninteractive_wait_does_not_detach_from_live_command(self) -> None:
        samples = iter((SimpleNamespace(alive=True), SimpleNamespace(alive=False)))
        sleeps: list[float] = []
        final = wait_until_finished(
            lambda: next(samples), interval=0.5, sleep_fn=sleeps.append
        )
        self.assertFalse(final.alive)
        self.assertEqual(sleeps, [0.5])

    def test_fast_owned_command_preserves_status_after_process_disappears(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            with mock.patch.object(build_watch, "CACHE_DIR", cache):
                process, log_path, started_at = build_watch.launch(
                    ["sh", "-c", "exit 95"], Path.cwd()
                )
            self.assertEqual(process.wait(), 95)
            with (
                mock.patch.object(build_watch, "scan_processes", return_value={}),
                mock.patch.object(
                    build_watch, "workspace_test_targets", return_value=0
                ),
                mock.patch.object(build_watch, "load_history", return_value={}),
                mock.patch.object(build_watch, "save_history"),
            ):
                monitor = build_watch.Monitor(
                    process.pid,
                    owned=process,
                    log_path=log_path,
                    launched_at=started_at,
                    launched_command=("sh", "-c", "exit 95"),
                    launched_cwd=Path.cwd(),
                )
                sample = monitor.sample()
            self.assertFalse(sample.alive)
            self.assertEqual(sample.exit_code, 95)

    def test_missing_command_returns_shell_style_status(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with (
                mock.patch.object(build_watch, "CACHE_DIR", Path(directory)),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                status = build_watch.main(["--run", "/definitely/not/a/command"])
        self.assertEqual(status, 127)


if __name__ == "__main__":
    unittest.main()
