#!/usr/bin/env python3
"""Run noninteractive launch checks against a finalized Quiet for Codex package."""

from __future__ import annotations

import argparse
import json
import os
import queue
import select
import shutil
import subprocess
import tarfile
import tempfile
import threading
import time
import uuid
import zipfile
from pathlib import Path


QUIET_DAEMON_DISABLED_MESSAGE = (
    "daemon-managed app-server routes are disabled in Quiet for Codex because "
    "the upstream implementation installs and updates stock Codex"
)
DISABLED_DAEMON_ROUTES = (
    ("app-server", "daemon", "bootstrap"),
    ("app-server", "daemon", "start"),
    ("app-server", "daemon", "stop"),
    ("app-server", "daemon", "restart"),
    ("app-server", "daemon", "enable-remote-control"),
    ("app-server", "daemon", "disable-remote-control"),
    ("app-server", "daemon", "pid-update-loop"),
    ("app-server", "daemon", "version"),
    ("remote-control",),
    ("remote-control", "start"),
    ("remote-control", "stop"),
    ("remote-control", "pair"),
)
SMOKE_CLEANUP_ATTEMPTS = 20
SMOKE_CLEANUP_RETRY_SECONDS = 0.25
SMOKE_THREAD_ID = "8a3ad8f7-0ef2-44ee-80cb-cdc560e7d91f"
SMOKE_FINAL_MARKER = "QUIET_REPLAY_DURABLE_FINAL_8A3AD8F7"
SMOKE_SUFFIX_MARKER = "QUIET_REPLAY_BUSY_SUFFIX_8A3AD8F7"
RESTORING_HISTORY_MARKER = "Restoring history…"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Smoke test a Quiet for Codex package."
    )
    parser.add_argument("archive", type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    return parser.parse_args()


def run_checked(
    command: list[str], *, stdin_empty: bool = False
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=True,
        stdin=subprocess.DEVNULL if stdin_empty else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
    )


def run_captured(
    command: list[str], *, environment: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
        env=environment,
    )


def assert_disabled_daemon_route(
    quiet: Path, route: tuple[str, ...], *, codex_home: Path
) -> None:
    environment = os.environ.copy()
    environment["CODEX_HOME"] = str(codex_home)
    result = run_captured([str(quiet), *route], environment=environment)
    output = result.stdout + result.stderr
    route_text = " ".join(route)
    if result.returncode == 0:
        raise RuntimeError(f"Disabled route unexpectedly succeeded: {route_text}")
    if QUIET_DAEMON_DISABLED_MESSAGE not in output:
        raise RuntimeError(
            f"Disabled route did not return the Quiet safety error: {route_text}\n"
            f"stdout: {result.stdout!r}\nstderr: {result.stderr!r}"
        )


def assert_fast_failure(binary: Path, expected_message: str) -> None:
    result = run_captured([str(binary)])
    output = result.stdout + result.stderr
    if result.returncode == 0:
        raise RuntimeError(
            f"Bundled helper unexpectedly succeeded without input: {binary}"
        )
    if expected_message not in output:
        raise RuntimeError(
            f"Bundled helper did not return {expected_message!r}: {binary}\n"
            f"stdout: {result.stdout!r}\nstderr: {result.stderr!r}"
        )


def extract_archive(archive: Path, destination: Path) -> None:
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(destination)
        return
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive, "r:gz") as bundle:
            bundle.extractall(destination)
        return
    raise RuntimeError(f"Unsupported package archive: {archive}")


def cleanup_smoke_directory(
    path: Path,
    *,
    attempts: int = SMOKE_CLEANUP_ATTEMPTS,
    retry_delay_seconds: float = SMOKE_CLEANUP_RETRY_SECONDS,
) -> None:
    """Remove a smoke directory after transient executable handles are released."""
    if attempts < 1:
        raise ValueError("Smoke cleanup attempts must be positive.")
    for attempt in range(attempts):
        try:
            shutil.rmtree(path)
            return
        except FileNotFoundError:
            return
        except PermissionError:
            if attempt + 1 == attempts:
                raise
            time.sleep(retry_delay_seconds)


def write_replay_smoke_config(codex_home: Path, cwd: Path) -> None:
    """Write an offline, trusted, read-only config for packaged replay checks."""
    project_key = json.dumps(str(cwd.resolve()))
    (codex_home / "config.toml").write_text(
        "\n".join(
            (
                'model = "smoke-model"',
                'model_provider = "smoke"',
                'approval_policy = "never"',
                'sandbox_mode = "read-only"',
                "",
                "[model_providers.smoke]",
                'name = "Offline release smoke"',
                'base_url = "http://127.0.0.1:9/v1"',
                'wire_api = "responses"',
                "request_max_retries = 0",
                "stream_max_retries = 0",
                "requires_openai_auth = false",
                "supports_websockets = false",
                "",
                "[features]",
                "plugins = false",
                "",
                f"[projects.{project_key}]",
                'trust_level = "trusted"',
                "",
            )
        ),
        encoding="utf-8",
    )


def write_paginated_replay_fixture(codex_home: Path, cwd: Path) -> Path:
    """Create a durable paginated transcript with a dense newer suffix."""
    cwd = cwd.resolve()
    timestamp = "2025-01-05T12:00:00Z"
    rollout_dir = codex_home / "sessions" / "2025" / "01" / "05"
    rollout_dir.mkdir(parents=True, exist_ok=True)
    rollout_path = rollout_dir / (
        f"rollout-2025-01-05T12-00-00-{SMOKE_THREAD_ID}.jsonl"
    )
    records: list[dict[str, object]] = []

    def append(record_type: str, payload: dict[str, object]) -> None:
        records.append(
            {
                "timestamp": timestamp,
                "type": record_type,
                "payload": payload,
                "ordinal": len(records),
            }
        )

    append(
        "session_meta",
        {
            "session_id": SMOKE_THREAD_ID,
            "id": SMOKE_THREAD_ID,
            "timestamp": timestamp,
            "cwd": str(cwd),
            "originator": "codex",
            "cli_version": "0.0.0",
            "source": "cli",
            "model_provider": "smoke",
            "history_mode": "paginated",
        },
    )
    append(
        "response_item",
        {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "release replay smoke"}],
        },
    )
    append(
        "event_msg",
        {
            "type": "user_message",
            "message": "release replay smoke",
            "kind": "plain",
        },
    )

    def append_turn_item(turn_id: str, item: dict[str, object]) -> None:
        append(
            "event_msg",
            {
                "type": "item_completed",
                "thread_id": SMOKE_THREAD_ID,
                "turn_id": turn_id,
                "item": item,
            },
        )

    completed_turn = "smoke-completed-turn"
    append(
        "event_msg",
        {
            "type": "task_started",
            "turn_id": completed_turn,
            "model_context_window": None,
        },
    )
    append_turn_item(
        completed_turn,
        {
            "type": "UserMessage",
            "id": "smoke-completed-user",
            "content": [{"type": "text", "text": "show the durable final"}],
        },
    )
    append_turn_item(
        completed_turn,
        {
            "type": "AgentMessage",
            "id": "smoke-durable-final",
            "content": [{"type": "Text", "text": SMOKE_FINAL_MARKER}],
            "phase": "final_answer",
        },
    )
    append(
        "event_msg",
        {
            "type": "task_complete",
            "turn_id": completed_turn,
            "last_agent_message": SMOKE_FINAL_MARKER,
        },
    )

    busy_turn = "smoke-busy-suffix-turn"
    append(
        "event_msg",
        {
            "type": "task_started",
            "turn_id": busy_turn,
            "model_context_window": None,
        },
    )
    append_turn_item(
        busy_turn,
        {
            "type": "UserMessage",
            "id": "smoke-busy-user",
            "content": [{"type": "text", "text": SMOKE_SUFFIX_MARKER}],
        },
    )
    for index in range(240):
        append_turn_item(
            busy_turn,
            {
                "type": "Reasoning",
                "id": f"smoke-hidden-reasoning-{index}",
                "summary_text": [],
                "raw_content": [],
            },
        )
    append(
        "event_msg",
        {
            "type": "task_complete",
            "turn_id": busy_turn,
            "last_agent_message": None,
        },
    )
    rollout_path.write_text(
        "".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records),
        encoding="utf-8",
    )
    return rollout_path


def jsonrpc_response(output: str, request_id: int) -> dict[str, object]:
    for raw_line in output.splitlines():
        if not raw_line.strip():
            continue
        try:
            message = json.loads(raw_line)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"App-server emitted non-JSON stdout: {raw_line!r}") from error
        if message.get("id") == request_id:
            if "error" in message:
                raise RuntimeError(
                    f"App-server request {request_id} failed: {message['error']!r}"
                )
            return message
    raise RuntimeError(f"App-server emitted no response for request {request_id}.")


def run_jsonrpc_until_response(
    command: list[str],
    *,
    cwd: Path,
    environment: dict[str, str],
    requests: tuple[dict[str, object], ...],
    request_id: int,
    timeout: float = 30,
) -> dict[str, object]:
    """Keep stdin open until an asynchronous JSON-RPC response arrives."""
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdin is not None
    assert process.stdout is not None
    assert process.stderr is not None

    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    stdout_queue: queue.Queue[str | None] = queue.Queue()

    def read_stdout() -> None:
        try:
            for line in process.stdout:
                stdout_lines.append(line)
                stdout_queue.put(line)
        finally:
            stdout_queue.put(None)

    def read_stderr() -> None:
        stderr_lines.extend(process.stderr)

    stdout_thread = threading.Thread(target=read_stdout, daemon=True)
    stderr_thread = threading.Thread(target=read_stderr, daemon=True)
    stdout_thread.start()
    stderr_thread.start()

    response: dict[str, object] | None = None
    failure = f"App-server emitted no response for request {request_id}."
    try:
        for request in requests:
            process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + timeout
        while response is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                failure = f"App-server timed out waiting for request {request_id}."
                break
            try:
                raw_line = stdout_queue.get(timeout=remaining)
            except queue.Empty:
                failure = f"App-server timed out waiting for request {request_id}."
                break
            if raw_line is None:
                break
            try:
                message = json.loads(raw_line)
            except json.JSONDecodeError:
                failure = f"App-server emitted non-JSON stdout: {raw_line!r}"
                break
            if message.get("id") != request_id:
                continue
            if "error" in message:
                failure = (
                    f"App-server request {request_id} failed: {message['error']!r}"
                )
                break
            response = message
    except (BrokenPipeError, OSError) as error:
        failure = f"App-server input failed before request {request_id}: {error}"
    finally:
        try:
            process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
        stdout_thread.join(timeout=2)
        stderr_thread.join(timeout=2)
        process.stdout.close()
        process.stderr.close()

    stdout = "".join(stdout_lines)
    stderr = "".join(stderr_lines)
    if response is None:
        raise RuntimeError(
            f"{failure}\nreturn code: {process.returncode}\n"
            f"stdout: {stdout!r}\nstderr: {stderr!r}"
        )
    if process.returncode != 0:
        raise RuntimeError(
            "Packaged app-server exited nonzero after its resume response.\n"
            f"return code: {process.returncode}\n"
            f"stdout: {stdout!r}\nstderr: {stderr!r}"
        )
    return response


def assert_app_server_resume_replay(
    quiet: Path, codex_home: Path, cwd: Path
) -> None:
    requests = (
        {
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": {
                    "name": "quiet_release_smoke",
                    "title": "Quiet release smoke",
                    "version": "1",
                },
                "capabilities": {"experimentalApi": True},
            },
        },
        {"method": "initialized", "params": {}},
        {
            "method": "thread/resume",
            "id": 2,
            "params": {
                "threadId": SMOKE_THREAD_ID,
                "model": "smoke-model",
                "modelProvider": "smoke",
                "approvalPolicy": "never",
                "sandbox": "read-only",
            },
        },
    )
    environment = os.environ.copy()
    environment["CODEX_HOME"] = str(codex_home)
    response = run_jsonrpc_until_response(
        [str(quiet), "app-server"],
        cwd=cwd,
        environment=environment,
        requests=requests,
        request_id=2,
        timeout=30,
    )
    response_text = json.dumps(response, sort_keys=True)
    for marker in (SMOKE_FINAL_MARKER, SMOKE_SUFFIX_MARKER):
        if marker not in response_text:
            raise RuntimeError(f"Packaged resume dropped replay marker {marker!r}.")


def assert_tui_resume_replay(quiet: Path, codex_home: Path, cwd: Path) -> None:
    """Exercise the actual TUI projection on native Unix release packages."""
    if os.name != "posix":
        return
    import fcntl
    import pty
    import struct
    import termios

    master_fd, slave_fd = pty.openpty()
    fcntl.ioctl(slave_fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    environment = os.environ.copy()
    environment.update(
        {
            "CODEX_HOME": str(codex_home),
            "TERM": "xterm-256color",
            "COLORTERM": "truecolor",
        }
    )
    environment.pop("NO_COLOR", None)
    process = subprocess.Popen(
        [str(quiet), "resume", SMOKE_THREAD_ID, "--no-alt-screen"],
        cwd=cwd,
        env=environment,
        stdin=slave_fd,
        stdout=slave_fd,
        stderr=slave_fd,
        start_new_session=True,
        close_fds=True,
    )
    os.close(slave_fd)
    output = bytearray()
    deadline = time.monotonic() + 30
    next_page_up = time.monotonic() + 2
    try:
        while time.monotonic() < deadline:
            readable, _, _ = select.select([master_fd], [], [], 0.2)
            if readable:
                try:
                    output.extend(os.read(master_fd, 65536))
                except OSError:
                    break
            decoded = output.decode("utf-8", errors="ignore")
            if RESTORING_HISTORY_MARKER in decoded and SMOKE_FINAL_MARKER in decoded:
                break
            if process.poll() is not None:
                break
            if time.monotonic() >= next_page_up:
                os.write(master_fd, b"\x1b[5~")
                next_page_up = time.monotonic() + 0.35
        decoded = output.decode("utf-8", errors="ignore")
        missing = [
            marker
            for marker in (RESTORING_HISTORY_MARKER, SMOKE_FINAL_MARKER)
            if marker not in decoded
        ]
        if missing:
            raise RuntimeError(
                "Packaged TUI replay did not render required markers: "
                + ", ".join(repr(marker) for marker in missing)
            )
    finally:
        if process.poll() is None:
            for _ in range(2):
                try:
                    os.write(master_fd, b"\x03")
                except OSError:
                    break
                time.sleep(0.1)
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
        os.close(master_fd)


def smoke_package(args: argparse.Namespace, package_dir: Path) -> None:
    suffix = ".exe" if args.target.endswith("-pc-windows-msvc") else ""
    quiet = package_dir / "bin" / f"codex-quiet{suffix}"
    host = package_dir / "bin" / f"codex-code-mode-host{suffix}"
    ripgrep = package_dir / "codex-path" / f"rg{suffix}"
    metadata_path = package_dir / "codex-package.json"
    v8_notices = package_dir / "THIRD_PARTY_LICENSES" / "v8" / "V8_RUSTY_V8_NOTICES.txt"

    if not v8_notices.is_file() or v8_notices.stat().st_size < 100_000:
        raise RuntimeError("Archive is missing the generated V8 and rusty_v8 notices.")
    if not v8_notices.read_text(encoding="utf-8").startswith(
        "Quiet for Codex V8 and rusty_v8 third-party notices\n"
    ):
        raise RuntimeError("Archive contains an invalid V8 and rusty_v8 notice report.")

    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    expected_entrypoint = f"bin/codex-quiet{suffix}"
    if metadata.get("entrypoint") != expected_entrypoint:
        raise RuntimeError(
            f"Expected entrypoint {expected_entrypoint!r}, got {metadata.get('entrypoint')!r}"
        )
    if metadata.get("unofficialFork") is not True:
        raise RuntimeError("Package metadata does not identify the unofficial fork.")

    version = run_checked([str(quiet), "--version"])
    expected_version = f"codex-quiet {args.version}"
    if version.stdout.strip() != expected_version:
        raise RuntimeError(
            f"Expected version output {expected_version!r}, got {version.stdout!r}"
        )
    help_result = run_checked([str(quiet), "--help"])
    if "usage" not in help_result.stdout.lower():
        raise RuntimeError("Help output has no usage line.")
    if "\n  app " in help_result.stdout.lower():
        raise RuntimeError("Help output exposes the disabled stock Desktop app route.")
    smoke_codex_home = package_dir / ".smoke-codex-home"
    smoke_codex_home.mkdir()
    for route in DISABLED_DAEMON_ROUTES:
        assert_disabled_daemon_route(quiet, route, codex_home=smoke_codex_home)
    replay_codex_home = package_dir / ".smoke-replay-codex-home"
    replay_codex_home.mkdir()
    write_replay_smoke_config(replay_codex_home, package_dir)
    write_paginated_replay_fixture(replay_codex_home, package_dir)
    assert_app_server_resume_replay(quiet, replay_codex_home, package_dir)
    assert_tui_resume_replay(quiet, replay_codex_home, package_dir)
    run_checked([str(host)], stdin_empty=True)
    ripgrep_version = run_checked([str(ripgrep), "--version"])
    if not ripgrep_version.stdout.lower().startswith("ripgrep "):
        raise RuntimeError(
            f"Bundled ripgrep returned unexpected version output: {ripgrep_version.stdout!r}"
        )
    if args.target.endswith("-unknown-linux-musl"):
        bwrap = package_dir / "codex-resources" / "bwrap"
        bwrap_version = run_checked([str(bwrap), "--version"])
        if "bubblewrap " not in bwrap_version.stdout.lower():
            raise RuntimeError(
                f"Bundled bubblewrap returned unexpected version output: {bwrap_version.stdout!r}"
            )
    if args.target.endswith("-pc-windows-msvc"):
        assert_fast_failure(
            package_dir / "codex-resources" / "codex-windows-sandbox-setup.exe",
            "expected payload argument",
        )
        assert_fast_failure(
            package_dir / "codex-resources" / "codex-command-runner.exe",
            "no pipe-in provided",
        )

    print(version.stdout.strip())
    replay_surface = "app-server and TUI" if os.name == "posix" else "app-server"
    print(f"Resume/replay smoke passed through {replay_surface}")
    print(f"Smoke checks passed for {args.target}")


def main() -> int:
    args = parse_args()
    archive = args.archive.resolve()
    if not archive.is_file():
        raise RuntimeError(f"Package archive does not exist: {archive}")
    package_dir = Path(tempfile.mkdtemp(prefix="codex-quiet-smoke-"))
    try:
        extract_archive(archive, package_dir)
        smoke_package(args, package_dir)
    finally:
        cleanup_smoke_directory(package_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
