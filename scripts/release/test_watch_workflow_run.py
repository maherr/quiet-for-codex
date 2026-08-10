#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))

import watch_workflow_run


def job(name: str, conclusion: str, status: str = "completed") -> dict[str, str]:
    return {"name": name, "status": status, "conclusion": conclusion}


class WatchWorkflowRunTest(unittest.TestCase):
    def test_polls_until_completed_success_and_discovers_all_job_pages(self) -> None:
        run_states = iter(
            (
                {"status": "queued", "conclusion": None},
                {"status": "in_progress", "conclusion": None},
                {"status": "completed", "conclusion": "success"},
            )
        )
        calls: list[str] = []

        def api(_repository: str, endpoint: str) -> dict[str, object]:
            calls.append(endpoint)
            if "/jobs?" not in endpoint:
                return next(run_states)
            if endpoint.endswith("page=1"):
                return {
                    "total_count": 3,
                    "jobs": [
                        job("Build x86_64-unknown-linux-musl", "success"),
                        job("Validate release tag", "success"),
                    ],
                }
            return {
                "total_count": 3,
                "jobs": [job("Provenance attestation canary", "skipped")],
            }

        clock = iter((0.0, 0.1, 0.2, 0.3, 0.4))
        result = watch_workflow_run.wait_for_workflow_run(
            watch_workflow_run.CANONICAL_REPOSITORY,
            123,
            interval_seconds=0.01,
            timeout_seconds=10,
            api=api,
            sleep=lambda _seconds: None,
            monotonic=lambda: next(clock),
        )

        self.assertEqual(result["conclusion"], "success")
        self.assertTrue(any("page=2" in call for call in calls))

    def test_completed_failure_is_never_accepted(self) -> None:
        with self.assertRaises(watch_workflow_run.WorkflowRunError):
            watch_workflow_run.wait_for_workflow_run(
                watch_workflow_run.CANONICAL_REPOSITORY,
                123,
                interval_seconds=1,
                timeout_seconds=10,
                api=lambda _repo, _endpoint: {
                    "status": "completed",
                    "conclusion": "failure",
                },
            )

    def test_unknown_skipped_job_is_not_counted_as_pass(self) -> None:
        with self.assertRaises(watch_workflow_run.WorkflowRunError):
            watch_workflow_run.validate_completed_jobs(
                [
                    job("Build x86_64-unknown-linux-musl", "success"),
                    job("Replay fixture", "skipped"),
                ]
            )

    def test_incomplete_job_is_rejected_after_run_completion(self) -> None:
        with self.assertRaises(watch_workflow_run.WorkflowRunError):
            watch_workflow_run.validate_completed_jobs(
                [job("Build x86_64-unknown-linux-musl", "success", "in_progress")]
            )

    def test_wrong_source_sha_or_ref_is_rejected_before_polling_jobs(self) -> None:
        with self.assertRaises(watch_workflow_run.WorkflowRunError):
            watch_workflow_run.wait_for_workflow_run(
                watch_workflow_run.CANONICAL_REPOSITORY,
                123,
                interval_seconds=1,
                timeout_seconds=10,
                expected_head_sha="a" * 40,
                expected_ref="quiet-v1.2.0",
                api=lambda _repo, _endpoint: {
                    "status": "completed",
                    "conclusion": "success",
                    "head_sha": "b" * 40,
                    "head_branch": "quiet-v1.1.0",
                },
            )


if __name__ == "__main__":
    unittest.main()
