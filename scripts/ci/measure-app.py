#!/usr/bin/env python3
"""Starts the real app, drives it with the CLI, and prints what the performance budget names:
hotkey (here: CLI show) to first paint, process start to hotkey ready, and resident memory after
start and after 100 queries. The numbers go to a Markdown table (also $GITHUB_STEP_SUMMARY).

They are informational until a baseline exists (follow-up: make first paint a failing check). The
script fails only when the app does not start or does not answer the CLI. Run it inside a display
(xvfb-run, scripts/ci/run-in-sway.sh) on Linux.
"""
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

root = Path(__file__).resolve().parents[2]
exe = ".exe" if os.name == "nt" else ""
app = os.environ.get("APP", str(root / "target" / "debug" / f"daimos-app{exe}"))
cli = os.environ.get("CLI", str(root / "target" / "debug" / f"daimos{exe}"))


def rss_mb(pid: int) -> float | None:
    try:
        if sys.platform.startswith("linux"):
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
        elif sys.platform == "darwin":
            out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True)
            return int(out.stdout.strip()) / 1024
        else:
            out = subprocess.run(
                ["powershell", "-NoProfile", "-Command", f"(Get-Process -Id {pid}).WorkingSet64"],
                capture_output=True, text=True,
            )
            return int(out.stdout.strip()) / (1024 * 1024)
    except (OSError, ValueError):
        return None
    return None


def children_total_mb(pid: int) -> float | None:
    """The app plus its web view processes: on Linux and macOS the web content runs in children."""
    if os.name == "nt":
        return rss_mb(pid)
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss="], capture_output=True, text=True).stdout
    rows = [tuple(map(int, line.split())) for line in out.splitlines() if line.strip()]
    family, grew = {pid}, True
    while grew:
        grew = False
        for p, parent, _ in rows:
            if parent in family and p not in family:
                family.add(p)
                grew = True
    return sum(rss for p, _, rss in rows if p in family) / 1024


def run_cli(*args: str) -> int:
    return subprocess.run([cli, *args], capture_output=True, timeout=30).returncode


def main() -> int:
    log = tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False)
    proc = subprocess.Popen([app], stdout=log, stderr=subprocess.STDOUT)
    text = lambda: Path(log.name).read_text(errors="replace")  # noqa: E731

    def wait_for(pattern: str, count: int = 1, seconds: float = 60) -> bool:
        deadline = time.time() + seconds
        while time.time() < deadline:
            if len(re.findall(pattern, text())) >= count:
                return True
            if proc.poll() is not None:
                return False
            time.sleep(0.1)
        return False

    try:
        if not wait_for("listening on"):
            print(text(), file=sys.stderr)
            print("the app did not start", file=sys.stderr)
            return 1
        time.sleep(2)  # let the web view settle before the first measurement
        memory_start = children_total_mb(proc.pid)

        paints = []
        for i in range(10):
            if run_cli("show", f"q{i}") != 0:
                print("the CLI could not show the window", file=sys.stderr)
                return 1
            wait_for("window visibility: shown", i + 1, 10)
            wait_for("hotkey to first paint", len(paints) + 1, 3)
            run_cli("hide")
            wait_for("window visibility: hidden", i + 1, 10)
            paints = [float(m) for m in re.findall(r"hotkey to first paint in ([\d.]+) ms", text())]

        for i in range(100):
            run_cli("show", f"query {i}")
        run_cli("hide")
        time.sleep(1)
        memory_after = children_total_mb(proc.pid)

        ready = re.search(r"registered (\d+) ms after process start", text())
        rows = [
            ("Process start to hotkey ready", f"{ready.group(1)} ms" if ready else "n/a (no hotkey)", "< 500 ms"),
            (
                "Hotkey to first paint (median of 10, debug build)",
                f"{statistics.median(paints):.1f} ms" if paints else "n/a (no paint reported)",
                "< 50 ms",
            ),
            ("Resident memory after start (app + web view)", f"{memory_start:.0f} MB" if memory_start else "n/a", "< 150 MB"),
            ("Resident memory after 100 queries", f"{memory_after:.0f} MB" if memory_after else "n/a", "< 150 MB"),
        ]
        table = "| Measurement | Value | Budget |\n| --- | ---: | ---: |\n" + "\n".join(
            f"| {a} | {b} | {c} |" for a, b, c in rows
        )
        print(table)
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            label = os.environ.get("MEASURE_LABEL", sys.platform)
            with open(summary, "a", encoding="utf-8") as out:
                out.write(f"### App measurements ({label}, informational)\n\n{table}\n\n")
        return 0
    finally:
        run_cli("quit")
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()
        os.unlink(log.name)


if __name__ == "__main__":
    sys.exit(main())
