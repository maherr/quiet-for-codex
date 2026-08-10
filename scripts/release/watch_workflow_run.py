#!/usr/bin/env python3
"""Poll a Quiet release run through the GitHub API until it is conclusive."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
from collections.abc import Callable
from typing import Any


CANONICAL_REPOSITORY = "maherr/quiet-for-codex"
ALLOWED_SKIPPED_JOBS = {"Provenance attestation canary"}


class WorkflowRunError(RuntimeError):
    pass


def gh_api(repository: str, endpoint: str) -> dict[str, Any]:
    environment = os.environ.copy()
    environment["GH_REPO"] = repository
    result = subprocess.run(
        ["gh", "api", endpoint],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=environment,
        timeout=30,
    )
    if result.returncode != 0:
        raise WorkflowRunError(
            f"GitHub API request failed for {endpoint}: {result.stderr.strip()}"
        )
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise WorkflowRunError(
            f"GitHub API returned invalid JSON for {endpoint}."
        ) from error
    if not isinstance(payload, dict):
        raise WorkflowRunError(f"GitHub API returned a non-object for {endpoint}.")
    return payload


def fetch_all_jobs(
    repository: str,
    run_id: int,
    api: Callable[[str, str], dict[str, Any]] = gh_api,
) -> list[dict[str, Any]]:
    jobs: list[dict[str, Any]] = []
    page = 1
    while True:
        payload = api(
            repository,
            f"repos/{repository}/actions/runs/{run_id}/jobs?per_page=100&page={page}",
        )
        page_jobs = payload.get("jobs")
        if not isinstance(page_jobs, list):
            raise WorkflowRunError("GitHub jobs response has no jobs array.")
        jobs.extend(job for job in page_jobs if isinstance(job, dict))
        total_count = payload.get("total_count")
        if not isinstance(total_count, int):
            raise WorkflowRunError("GitHub jobs response has no integer total_count.")
        if len(jobs) >= total_count:
            return jobs
        if not page_jobs:
            raise WorkflowRunError(
                f"GitHub jobs pagination ended at {len(jobs)} of {total_count}."
            )
        page += 1


def validate_completed_jobs(jobs: list[dict[str, Any]]) -> None:
    if not jobs:
        raise WorkflowRunError("Completed workflow has no jobs.")
    failures: list[str] = []
    build_jobs = 0
    for job in jobs:
        name = str(job.get("name", "<unnamed>"))
        status = job.get("status")
        conclusion = job.get("conclusion")
        if name.startswith("Build "):
            build_jobs += 1
        if status != "completed":
            failures.append(f"{name}: status={status!r}")
        elif conclusion == "success":
            continue
        elif conclusion == "skipped" and name in ALLOWED_SKIPPED_JOBS:
            continue
        else:
            failures.append(f"{name}: conclusion={conclusion!r}")
    if build_jobs == 0:
        failures.append("no dynamically discovered Build matrix jobs")
    if failures:
        raise WorkflowRunError(
            "Release workflow jobs did not all reach an accepted conclusion:\n"
            + "\n".join(f"- {failure}" for failure in failures)
        )


def validate_run_identity(
    run: dict[str, Any],
    *,
    expected_head_sha: str | None,
    expected_ref: str | None,
) -> None:
    failures: list[str] = []
    if expected_head_sha is not None and run.get("head_sha") != expected_head_sha:
        failures.append(
            f"head_sha={run.get('head_sha')!r}, expected {expected_head_sha!r}"
        )
    if expected_ref is not None and run.get("head_branch") != expected_ref:
        failures.append(
            f"head_branch={run.get('head_branch')!r}, expected {expected_ref!r}"
        )
    if failures:
        raise WorkflowRunError(
            "Release workflow identity does not match the requested source:\n"
            + "\n".join(f"- {failure}" for failure in failures)
        )


def wait_for_workflow_run(
    repository: str,
    run_id: int,
    *,
    interval_seconds: float,
    timeout_seconds: float,
    expected_head_sha: str | None = None,
    expected_ref: str | None = None,
    api: Callable[[str, str], dict[str, Any]] = gh_api,
    sleep: Callable[[float], None] = time.sleep,
    monotonic: Callable[[], float] = time.monotonic,
) -> dict[str, Any]:
    deadline = monotonic() + timeout_seconds
    last_state: tuple[object, object] | None = None
    endpoint = f"repos/{repository}/actions/runs/{run_id}"
    while True:
        run = api(repository, endpoint)
        validate_run_identity(
            run,
            expected_head_sha=expected_head_sha,
            expected_ref=expected_ref,
        )
        state = (run.get("status"), run.get("conclusion"))
        if state != last_state:
            print(f"workflow {run_id}: status={state[0]} conclusion={state[1]}")
            last_state = state
        if state[0] == "completed":
            if state[1] != "success":
                raise WorkflowRunError(
                    f"Workflow {run_id} completed with conclusion {state[1]!r}."
                )
            jobs = fetch_all_jobs(repository, run_id, api)
            validate_completed_jobs(jobs)
            print(f"workflow {run_id}: verified {len(jobs)} dynamically listed jobs")
            return run
        if monotonic() >= deadline:
            raise WorkflowRunError(
                f"Workflow {run_id} did not complete within {timeout_seconds:g} seconds."
            )
        sleep(interval_seconds)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Wait for a Quiet release workflow through the GitHub API."
    )
    parser.add_argument("run_id", type=int)
    parser.add_argument("--repo", default=CANONICAL_REPOSITORY)
    parser.add_argument("--sha", required=True, help="Expected 40-character source SHA")
    parser.add_argument("--ref", required=True, help="Expected head branch or tag name")
    parser.add_argument("--interval", type=float, default=15.0)
    parser.add_argument("--timeout", type=float, default=7200.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.run_id <= 0:
        raise WorkflowRunError("run_id must be positive.")
    if re.fullmatch(r"[0-9a-f]{40}", args.sha) is None:
        raise WorkflowRunError("--sha must be a lowercase 40-character commit SHA.")
    if not args.ref.strip():
        raise WorkflowRunError("--ref must not be empty.")
    if args.interval <= 0 or args.timeout <= 0:
        raise WorkflowRunError("interval and timeout must be positive.")
    wait_for_workflow_run(
        args.repo,
        args.run_id,
        interval_seconds=args.interval,
        timeout_seconds=args.timeout,
        expected_head_sha=args.sha,
        expected_ref=args.ref,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
