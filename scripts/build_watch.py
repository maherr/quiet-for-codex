#!/usr/bin/env python3
"""Live telemetry for long Cargo/nextest runs.

Attach automatically or by PID, or launch with ``--run``. Ctrl-C only detaches;
launch mode also captures output and learns ETA baselines.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import shlex
import statistics
import subprocess
import sys
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable
from typing import Iterable

import psutil
from rich.align import Align
from rich.columns import Columns
from rich.console import Console, Group, RenderableType
from rich.live import Live
from rich.panel import Panel
from rich.progress_bar import ProgressBar
from rich.table import Table
from rich.text import Text


CACHE_DIR = Path.home() / ".cache" / "codex-build-watch"
HISTORY_PATH = CACHE_DIR / "history.json"
PHASES = ("DISCOVER", "BUILD", "TEST", "VERIFY")
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
STARTING_TESTS_RE = re.compile(
    r"Starting\s+(\d+)\s+tests?\s+across\s+\d+\s+binar(?:y|ies)"
)
TEST_RESULT_RE = re.compile(r"^\s*(PASS|FAIL|SKIP|LEAK|TIMEOUT)\b")
SUMMARY_RE = re.compile(r"Summary\s+\[[^]]*]\s+(\d+)\s+tests?\s+run")
CRATE_RE = re.compile(r"(?:^|\s)--crate-name(?:=|\s+)([^\s]+)")
BUILD_COMMAND_RE = re.compile(r"(?:^|\s)(?:cargo\s+nextest\s+run|just\s+test)(?:\s|$)")


@dataclass(frozen=True)
class ProcInfo:
    pid: int
    ppid: int
    started_at: float
    cpu_seconds: float
    rss_bytes: int
    comm: str
    argv: tuple[str, ...]
    cwd: Path | None

    @property
    def command(self) -> str:
        return shlex.join(self.argv)


@dataclass
class OutputFacts:
    lines: deque[str] = field(default_factory=lambda: deque(maxlen=8))
    tests_done: int = 0
    tests_total: int = 0
    saw_build_finished: bool = False
    saw_test_summary: bool = False

    def consume(self, raw: str) -> None:
        line = ANSI_RE.sub("", raw).rstrip()
        if not line:
            return
        self.lines.append(line)
        self.saw_build_finished |= (
            "Finished `test` profile" in line or "Finished test profile" in line
        )
        if match := STARTING_TESTS_RE.search(line):
            self.tests_total = int(match.group(1))
        self.tests_done += int(TEST_RESULT_RE.match(line) is not None)
        if match := SUMMARY_RE.search(line):
            self.tests_done = int(match.group(1))
            self.saw_test_summary = True


@dataclass(frozen=True)
class Sample:
    root_pid: int
    alive: bool
    phase: str
    target: str
    elapsed: float
    eta: str
    eta_basis: str
    progress: float | None
    jobs: int
    cpu_percent: float
    rss_bytes: int
    mem_used: int
    mem_total: int
    swap_used: int
    swap_total: int
    disk_read_rate: float
    disk_write_rate: float
    artifacts: int
    workspace_targets: int
    repo: Path
    tests_done: int
    tests_total: int
    recent: tuple[str, ...]
    cpu_history: tuple[float, ...]
    io_history: tuple[float, ...]
    exit_code: int | None = None


def process_info(process: psutil.Process) -> ProcInfo | None:
    try:
        times = process.cpu_times()
        return ProcInfo(
            pid=process.pid,
            ppid=process.ppid(),
            started_at=process.create_time(),
            cpu_seconds=times.user + times.system,
            rss_bytes=process.memory_info().rss,
            comm=process.name(),
            argv=tuple(process.cmdline()),
            cwd=Path(process.cwd()),
        )
    except (psutil.Error, OSError):
        return None


def scan_processes() -> dict[int, ProcInfo]:
    rows = (process_info(process) for process in psutil.process_iter())
    return {row.pid: row for row in rows if row is not None}


def descendants(root_pid: int, processes: dict[int, ProcInfo]) -> list[ProcInfo]:
    children: dict[int, list[int]] = {}
    for process in processes.values():
        children.setdefault(process.ppid, []).append(process.pid)
    pending, seen, found = [root_pid], set(), []
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        if process := processes.get(pid):
            found.append(process)
        pending.extend(children.get(pid, ()))
    return found


def choose_build_root(processes: dict[int, ProcInfo]) -> ProcInfo | None:
    candidates = [
        process
        for process in processes.values()
        if BUILD_COMMAND_RE.search(process.command)
    ]
    if not candidates:
        return None
    candidate_pids = {process.pid for process in candidates}
    roots = [process for process in candidates if process.ppid not in candidate_pids]
    return max(roots or candidates, key=lambda process: process.started_at)


def find_repo(cwd: Path | None) -> Path:
    current = (cwd or Path.cwd()).resolve()
    for path in (current, *current.parents):
        if (path / "justfile").is_file() and (path / ".git").exists():
            return path
    return current


def active_target(processes: Iterable[ProcInfo]) -> str:
    rustc = [process for process in processes if process.comm == "rustc"]
    if not rustc:
        return "waiting for the next work unit"
    newest = max(rustc, key=lambda process: process.started_at)
    match = CRATE_RE.search(newest.command)
    return match.group(1).replace("_", "-") if match else newest.comm


def classify_phase(processes: Iterable[ProcInfo], facts: OutputFacts) -> str:
    processes = tuple(processes)
    comms = {process.comm for process in processes}
    if facts.saw_test_summary:
        return "VERIFY"
    if facts.saw_build_finished or facts.tests_total:
        return "TEST"
    if comms.intersection({"ld", "ld.lld", "collect2"}) or any(
        process.comm in {"cc", "gcc", "clang"}
        and "target/debug/deps" in process.command
        for process in processes
    ):
        return "BUILD"
    if "rustc" in comms:
        return "BUILD"
    if any("target/debug/deps/" in process.command for process in processes):
        return "TEST"
    return "DISCOVER"


def count_new_artifacts(target_dir: Path, started_at: float) -> int:
    deps = target_dir / "debug" / "deps"
    if not deps.is_dir():
        return 0
    try:
        return sum(
            path.stat().st_mtime >= started_at - 1 and path.stat().st_mode & 0o111 != 0
            for path in deps.iterdir()
            if path.is_file()
        )
    except OSError:
        return 0


def workspace_test_targets(repo: Path) -> int:
    manifest_dir = (
        repo / "codex-rs" if (repo / "codex-rs" / "Cargo.toml").is_file() else repo
    )
    try:
        output = subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=manifest_dir,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=15,
        )
        metadata = json.loads(output)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
        return 0
    return sum(
        target.get("test") or "test" in target.get("kind", ())
        for package in metadata.get("packages", ())
        for target in package.get("targets", ())
    )


def history_key(repo: Path, command: str) -> str:
    return f"{repo.resolve()}\0{re.sub(r'\s+', ' ', command).strip()}"


def load_history(path: Path = HISTORY_PATH) -> dict[str, list[dict[str, float | int]]]:
    try:
        value = json.loads(path.read_text())
        return value if isinstance(value, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def save_history(
    key: str, duration: float, exit_code: int, path: Path = HISTORY_PATH
) -> None:
    history = load_history(path)
    rows = history.setdefault(key, [])
    rows.append({"duration": round(duration, 3), "exit_code": exit_code})
    history[key] = rows[-12:]
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(history, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def estimate_eta(
    elapsed: float,
    phase: str,
    facts: OutputFacts,
    history_rows: Iterable[dict[str, float | int]],
    test_elapsed: float | None = None,
) -> tuple[str, str, float | None]:
    successful = [
        float(row["duration"]) for row in history_rows if row.get("exit_code") == 0
    ]
    if facts.tests_total and facts.tests_done:
        progress = min(1.0, facts.tests_done / facts.tests_total)
        observed = test_elapsed if test_elapsed is not None else elapsed
        return (
            f"~{format_duration(observed * (1 - progress) / progress)}",
            "live test throughput",
            progress,
        )
    if successful:
        baseline = statistics.median(successful[-5:])
        if elapsed >= baseline:
            return (
                "OVERRUN",
                f"{format_duration(elapsed - baseline)} past median",
                0.97,
            )
        progress = min(0.97, elapsed / baseline) if baseline else None
        return (
            f"~{format_duration(max(0, baseline - elapsed))}",
            f"median of {len(successful[-5:])} run(s)",
            progress,
        )
    if phase == "VERIFY":
        return "<1m", "final checks", 0.98
    return "CALIBRATING", "first completed run teaches the estimate", None


def format_duration(seconds: float) -> str:
    seconds = max(0, int(round(seconds)))
    hours, remainder = divmod(seconds, 3600)
    minutes, seconds = divmod(remainder, 60)
    if hours:
        return f"{hours}h {minutes:02d}m"
    if minutes:
        return f"{minutes}m {seconds:02d}s"
    return f"{seconds}s"


def format_bytes(value: float) -> str:
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(value) < 1024 or unit == "TiB":
            return f"{value:.1f} {unit}"
        value /= 1024
    return "0 B"


def sparkline(values: Iterable[float], width: int = 22) -> str:
    points = list(values)[-width:]
    if not points:
        return "·" * width
    peak, blocks = max(points) or 1, "▁▂▃▄▅▆▇█"
    return "".join(
        blocks[min(7, math.floor(value / peak * 7))] for value in points
    ).rjust(width, "·")


def command_exit_code(exit_code: int | None) -> int:
    if exit_code is None:
        return 0
    return 128 - exit_code if exit_code < 0 else exit_code


def normalize_run_command(items: Iterable[str]) -> list[str]:
    command = list(items)
    return command[1:] if command[:1] == ["--"] else command


def wait_until_finished(
    sample_fn: Callable[[], Sample],
    interval: float,
    sleep_fn: Callable[[float], None] = time.sleep,
) -> Sample:
    """Poll without rendering until the observed process exits."""
    while True:
        sample = sample_fn()
        if not sample.alive:
            return sample
        sleep_fn(max(0.1, interval))


class LogTail:
    def __init__(self, path: Path, facts: OutputFacts) -> None:
        self.path, self.facts, self.offset, self.partial = path, facts, 0, ""

    def poll(self) -> None:
        try:
            with self.path.open(errors="replace") as handle:
                handle.seek(self.offset)
                chunk, self.offset = handle.read(), handle.tell()
        except OSError:
            return
        if not chunk:
            return
        lines = (self.partial + chunk).splitlines(keepends=True)
        self.partial = ""
        if lines and not lines[-1].endswith(("\n", "\r")):
            self.partial = lines.pop()
        for line in lines:
            self.facts.consume(line)


class Monitor:
    def __init__(
        self,
        root_pid: int,
        owned: subprocess.Popen[bytes] | None = None,
        log_path: Path | None = None,
        launched_at: float | None = None,
        launched_command: tuple[str, ...] | None = None,
        launched_cwd: Path | None = None,
    ) -> None:
        root = scan_processes().get(root_pid)
        if root is None:
            if (
                owned is None
                or launched_at is None
                or launched_command is None
                or launched_cwd is None
            ):
                raise RuntimeError(f"process {root_pid} is not running")
            self.root_pid, self.started_at = root_pid, launched_at
            self.repo = find_repo(launched_cwd)
            self.command = shlex.join(launched_command)
        else:
            self.root_pid, self.started_at = root_pid, root.started_at
            self.repo, self.command = find_repo(root.cwd), root.command
        self.target_dir = self.repo / "codex-rs" / "target"
        if not self.target_dir.exists():
            self.target_dir = self.repo / "target"
        self.key = history_key(self.repo, self.command)
        self.history_rows = load_history().get(self.key, [])
        self.workspace_targets = workspace_test_targets(self.repo)
        self.facts, self.owned = OutputFacts(), owned
        self.log_tail = LogTail(log_path, self.facts) if log_path else None
        self.previous_time, self.previous_cpu = time.monotonic(), {}
        disk = psutil.disk_io_counters(nowrap=True)
        self.previous_disk = (disk.read_bytes, disk.write_bytes) if disk else (0, 0)
        self.cpu_history: deque[float] = deque(maxlen=48)
        self.io_history: deque[float] = deque(maxlen=48)
        self.saved_history = False
        self.test_started_at: float | None = None

    def sample(self) -> Sample:
        if self.log_tail:
            self.log_tail.poll()
        now, processes = time.monotonic(), scan_processes()
        interval, live = (
            max(0.001, now - self.previous_time),
            descendants(self.root_pid, processes),
        )
        current_cpu = {process.pid: process.cpu_seconds for process in live}
        cpu = sum(
            max(0, value - self.previous_cpu.get(pid, value))
            for pid, value in current_cpu.items()
        )
        cpu_percent, self.previous_cpu = cpu / interval * 100, current_cpu
        disk = psutil.disk_io_counters(nowrap=True)
        current_disk = (
            (disk.read_bytes, disk.write_bytes) if disk else self.previous_disk
        )
        read_rate = max(0, current_disk[0] - self.previous_disk[0]) / interval
        write_rate = max(0, current_disk[1] - self.previous_disk[1]) / interval
        self.previous_time, self.previous_disk = now, current_disk
        self.cpu_history.append(cpu_percent)
        self.io_history.append(read_rate + write_rate)
        phase, elapsed = (
            classify_phase(live, self.facts),
            max(0, time.time() - self.started_at),
        )
        if self.facts.tests_total and self.test_started_at is None:
            self.test_started_at = now
        eta, eta_basis, progress = estimate_eta(
            elapsed,
            phase,
            self.facts,
            self.history_rows,
            test_elapsed=(
                max(0, now - self.test_started_at)
                if self.test_started_at is not None
                else None
            ),
        )
        exit_code = self.owned.poll() if self.owned else None
        alive = self.root_pid in processes
        if self.owned and exit_code is not None:
            alive, phase = False, "VERIFY" if exit_code == 0 else "FAILED"
            if not self.saved_history:
                save_history(self.key, elapsed, exit_code)
                self.saved_history = True
        memory, swap = psutil.virtual_memory(), psutil.swap_memory()
        jobs = sum(
            process.comm in {"rustc", "cc", "gcc", "clang", "ld", "ld.lld"}
            for process in live
        )
        return Sample(
            root_pid=self.root_pid,
            alive=alive,
            phase=phase,
            target=active_target(live),
            elapsed=elapsed,
            eta=eta,
            eta_basis=eta_basis,
            progress=progress,
            jobs=jobs,
            cpu_percent=cpu_percent,
            rss_bytes=sum(process.rss_bytes for process in live),
            mem_used=memory.used,
            mem_total=memory.total,
            swap_used=swap.used,
            swap_total=swap.total,
            disk_read_rate=read_rate,
            disk_write_rate=write_rate,
            artifacts=count_new_artifacts(self.target_dir, self.started_at),
            workspace_targets=self.workspace_targets,
            repo=self.repo,
            tests_done=self.facts.tests_done,
            tests_total=self.facts.tests_total,
            recent=tuple(self.facts.lines),
            cpu_history=tuple(self.cpu_history),
            io_history=tuple(self.io_history),
            exit_code=exit_code,
        )


def phase_index(phase: str) -> int:
    try:
        return PHASES.index(phase)
    except ValueError:
        return 3 if phase == "FAILED" else 0


def demo_sample() -> Sample:
    return Sample(
        root_pid=4_124_702,
        alive=True,
        phase="BUILD",
        target="codex-cli · login integration suite",
        elapsed=582,
        eta="~11m 20s",
        eta_basis="median of 3 comparable runs",
        progress=0.61,
        jobs=2,
        cpu_percent=186,
        rss_bytes=6_418_000_000,
        mem_used=21_900_000_000,
        mem_total=66_500_000_000,
        swap_used=7_300_000_000,
        swap_total=8_500_000_000,
        disk_read_rate=18_000_000,
        disk_write_rate=142_000_000,
        artifacts=94,
        workspace_targets=220,
        repo=Path("~/src/codex-quiet").expanduser(),
        tests_done=0,
        tests_total=0,
        recent=(
            "Compiling codex-core v0.146.1",
            "Compiling codex-tui v0.146.1",
            "Linking login-947e0565cc3ca78e",
        ),
        cpu_history=(20, 45, 80, 130, 175, 190, 186),
        io_history=(4, 9, 42, 70, 150, 132, 160),
    )


def phases_renderable(sample: Sample) -> RenderableType:
    line = Text()
    current = phase_index(sample.phase)
    for index, phase in enumerate(PHASES):
        if index:
            line.append("  ›  ", style="bright_blue")
        if index < current:
            line.append(f"✓ {phase}", style="bold cyan")
        elif index == current:
            line.append(f"◆ {phase}", style="bold orange3")
        else:
            line.append(f"· {phase}", style="grey50")
    if sample.progress is None:
        progress: RenderableType = Text(
            "▓▓▓░░░░░░░░░░░  observing live work", style="cyan"
        )
    else:
        progress = ProgressBar(
            total=100,
            completed=sample.progress * 100,
            width=None,
            style="grey23",
            complete_style="cyan",
        )
    label = (
        f"{sample.tests_done:,}/{sample.tests_total:,} tests"
        if sample.tests_total
        else f"{sample.artifacts} new executable artifacts · {sample.workspace_targets} workspace test targets"
    )
    eta = Text.assemble(
        ("ETA  ", "grey70"),
        (sample.eta, "bold orange3"),
        (f"   {sample.eta_basis}", "grey50"),
    )
    rows = Table.grid(expand=True)
    rows.add_row(Align.center(line))
    rows.add_row(progress)
    rows.add_row(Text(label, style="bold"))
    rows.add_row(eta)
    return Panel(rows, title="[bold cyan]PIPELINE[/]", border_style="bright_blue")


def dashboard(sample: Sample) -> RenderableType:
    if sample.phase == "FAILED" or sample.exit_code not in (None, 0):
        status = "FAILED"
    else:
        status = (
            "RUNNING"
            if sample.alive
            else ("PASS" if sample.exit_code == 0 else "ENDED")
        )
    status_style = "black on cyan" if status != "FAILED" else "bold white on magenta"
    header = Text.assemble(
        (" FORGE ", "bold black on cyan"),
        ("  // CODEX QUIET BUILD TELEMETRY", "bold"),
        (" " * 4),
        (f" {status} ", status_style),
    )
    activity = Table.grid(padding=(0, 1), expand=True)
    activity.add_column(style="grey62", width=12)
    activity.add_column()
    activity.add_row("TARGET", Text(sample.target, style="bold"))
    activity.add_row("JOBS", str(sample.jobs))
    activity.add_row(
        "BUILD CPU", f"{sample.cpu_percent:5.1f}%  {sparkline(sample.cpu_history)}"
    )
    activity.add_row("BUILD RSS", format_bytes(sample.rss_bytes))
    activity.add_row("ELAPSED", format_duration(sample.elapsed))
    machine = Table.grid(padding=(0, 1), expand=True)
    machine.add_column(style="grey62", width=12)
    machine.add_column()
    machine.add_row(
        "RAM", f"{format_bytes(sample.mem_used)} / {format_bytes(sample.mem_total)}"
    )
    swap = f"{format_bytes(sample.swap_used)} / {format_bytes(sample.swap_total)}"
    if sample.swap_total and sample.swap_used / sample.swap_total >= 0.8:
        swap += "  [bold orange3]HIGH[/]"
    machine.add_row("SWAP", swap)
    machine.add_row("SYSTEM DISK ↓", f"{format_bytes(sample.disk_read_rate)}/s")
    machine.add_row("SYSTEM DISK ↑", f"{format_bytes(sample.disk_write_rate)}/s")
    machine.add_row("I/O TREND", Text(sparkline(sample.io_history), style="cyan"))
    recent = sample.recent or (
        "Watching process activity; attach mode has no captured build log.",
    )
    recent_text = Text("\n".join(recent[-6:]), style="grey70")
    footer = Text(
        f"PID {sample.root_pid}  ·  {sample.repo.name}  ·  Ctrl-C detaches safely",
        style="grey50",
        justify="center",
    )
    return Group(
        header,
        phases_renderable(sample),
        Columns(
            (
                Panel(activity, title="[bold cyan]ACTIVE WORK[/]", border_style="blue"),
                Panel(machine, title="[bold cyan]MACHINE[/]", border_style="blue"),
            ),
            equal=True,
            expand=True,
        ),
        Panel(
            recent_text, title="[bold cyan]RECENT SIGNAL[/]", border_style="bright_blue"
        ),
        footer,
    )


def launch(
    command: list[str], cwd: Path
) -> tuple[subprocess.Popen[bytes], Path, float]:
    runs = CACHE_DIR / "runs"
    runs.mkdir(parents=True, exist_ok=True)
    log_path = runs / f"build-{time.strftime('%Y%m%dT%H%M%S')}.log"
    started_at = time.time()
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            command,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    return process, log_path, started_at


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Watch a Cargo/nextest build in a live TUI."
    )
    parser.add_argument("--pid", type=int, help="attach to this build process")
    parser.add_argument(
        "--run", nargs=argparse.REMAINDER, help="launch and watch a command"
    )
    parser.add_argument(
        "--cwd", type=Path, default=Path.cwd(), help="working directory for --run"
    )
    parser.add_argument(
        "--interval", type=float, default=0.5, help="refresh interval in seconds"
    )
    parser.add_argument(
        "--once", action="store_true", help="render one sample and exit"
    )
    parser.add_argument(
        "--demo", action="store_true", help="render a stable demonstration dashboard"
    )
    parser.add_argument(
        "--color",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="enable the accessible blue/orange/teal palette",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    terminal = sys.stdout.isatty()
    console = Console(
        force_terminal=terminal,
        color_system="truecolor" if terminal and args.color else None,
        no_color=not (terminal and args.color),
    )
    if args.demo:
        console.print(dashboard(demo_sample()))
        return 0
    owned, log_path, pid = None, None, args.pid
    launched_at, launched_command, launched_cwd = None, None, None
    if args.run is not None:
        command = normalize_run_command(args.run)
        if not command:
            console.print("[bold magenta]--run needs a command[/]")
            return 2
        launched_command = tuple(command)
        launched_cwd = args.cwd.resolve()
        try:
            owned, log_path, launched_at = launch(command, launched_cwd)
        except FileNotFoundError as error:
            console.print(f"[bold magenta]Could not launch command: {error}[/]")
            return 127
        except OSError as error:
            console.print(f"[bold magenta]Could not launch command: {error}[/]")
            return 1
        pid = owned.pid
    if pid is None:
        root = choose_build_root(scan_processes())
        if root is None:
            console.print(
                "[bold orange3]No running `just test` or `cargo nextest run` build found.[/]"
            )
            return 1
        pid = root.pid
    try:
        monitor = Monitor(
            pid,
            owned=owned,
            log_path=log_path,
            launched_at=launched_at,
            launched_command=launched_command,
            launched_cwd=launched_cwd,
        )
    except RuntimeError as error:
        console.print(f"[bold magenta]{error}[/]")
        return 1
    if args.once:
        time.sleep(min(args.interval, 0.25))
        console.print(dashboard(monitor.sample()))
        return 0
    if not console.is_terminal:
        first = monitor.sample()
        console.print(dashboard(first))
        final = (
            first
            if not first.alive
            else wait_until_finished(monitor.sample, args.interval)
        )
        console.print(dashboard(final))
        return command_exit_code(final.exit_code if owned else None)
    try:
        with Live(
            dashboard(monitor.sample()),
            console=console,
            refresh_per_second=8,
            screen=True,
        ) as live:
            while True:
                sample = monitor.sample()
                live.update(dashboard(sample), refresh=True)
                if not sample.alive:
                    time.sleep(2)
                    break
                time.sleep(max(0.1, args.interval))
    except KeyboardInterrupt:
        pass
    if owned and owned.poll() is None:
        console.print(f"Detached; build PID {owned.pid} continues. Log: {log_path}")
    return command_exit_code(owned.returncode if owned else None)


if __name__ == "__main__":
    raise SystemExit(main())
