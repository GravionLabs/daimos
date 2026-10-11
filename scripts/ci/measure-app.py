#!/usr/bin/env python3
"""Starts the real app, drives it with the CLI, and prints what the performance budget names:
hotkey (here: CLI show) to first paint, process start to hotkey ready, and the memory of the app
and its web view after start and after 100 queries (proportional memory, not RSS: see the budget). The numbers go to a Markdown table (also $GITHUB_STEP_SUMMARY).

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


def process_family(pid: int) -> set[int]:
    """The process and everything below it: the web view runs in child processes on Linux."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid="], capture_output=True, text=True).stdout
    rows = [tuple(map(int, line.split())) for line in out.splitlines() if line.strip()]
    family, grew = {pid}, True
    while grew:
        grew = False
        for p, parent in rows:
            if parent in family and p not in family:
                family.add(p)
                grew = True
    return family


def linux_pss_mb(pid: int) -> float | None:
    """Proportional set size: shared libraries count once, split between the processes using them.
    RSS would count the GTK, WebKit and Mesa libraries again in every process (about 3x too much)."""
    try:
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            if line.startswith("Pss:"):
                return int(line.split()[1]) / 1024
    except OSError:
        pass
    return None


def macos_footprint_mb(pid: int) -> float | None:
    """The footprint of Activity Monitor's "Memory" column (`footprint` ships with macOS)."""
    out = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True).stdout
    match = re.search(r"Footprint:\s+([\d.,]+)\s*(KB|MB|GB)", out)
    if not match:
        return None
    value = float(match.group(1).replace(",", ""))
    return value * {"KB": 1 / 1024, "MB": 1.0, "GB": 1024.0}[match.group(2)]


def windows_private_mb(pid: int) -> float | None:
    """Private working set of the process and its children (WebView2 runs in child processes)."""
    script = (
        "$p = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId;"
        f"$f = @{{{pid} = $true}}; $grew = $true;"
        "while ($grew) { $grew = $false; foreach ($x in $p) { if ($f.ContainsKey([int]$x.ParentProcessId) -and -not $f.ContainsKey([int]$x.ProcessId)) { $f[[int]$x.ProcessId] = $true; $grew = $true } } };"
        "$sum = 0; Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | ForEach-Object { if ($f.ContainsKey([int]$_.IDProcess)) { $sum += $_.WorkingSetPrivate } };"
        "[math]::Round($sum / 1MB, 1)"
    )
    out = subprocess.run(["powershell", "-NoProfile", "-Command", script], capture_output=True, text=True)
    try:
        return float(out.stdout.strip())
    except ValueError:
        return None


def memory_mb(pid: int) -> tuple[float | None, str]:
    """The memory of the app and its web view, and the name of the metric (the platforms differ)."""
    try:
        if sys.platform.startswith("linux"):
            values = [linux_pss_mb(p) for p in process_family(pid)]
            return (sum(v for v in values if v is not None) or None), "PSS"
        if sys.platform == "darwin":
            # WKWebView runs its content in XPC services that are not children of the app.
            web = subprocess.run(["pgrep", "-f", "com.apple.WebKit"], capture_output=True, text=True).stdout.split()
            values = [macos_footprint_mb(p) for p in [pid, *map(int, web)]]
            return (sum(v for v in values if v is not None) or None), "footprint"
        return windows_private_mb(pid), "private working set"
    except (OSError, ValueError):
        return None, "unavailable"


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
        memory_start, metric = memory_mb(proc.pid)

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
        memory_after, _ = memory_mb(proc.pid)

        ready = re.search(r"registered (\d+) ms after process start", text())
        rows = [
            ("Process start to hotkey ready", f"{ready.group(1)} ms" if ready else "n/a (no hotkey)", "< 500 ms"),
            (
                "Hotkey to first paint (median of 10, debug build)",
                f"{statistics.median(paints):.1f} ms" if paints else "n/a (no paint reported)",
                "< 50 ms",
            ),
            (f"Memory after start (app + web view, {metric})", f"{memory_start:.0f} MB" if memory_start else "n/a", "< 150 MB"),
            (f"Memory after 100 queries ({metric})", f"{memory_after:.0f} MB" if memory_after else "n/a", "idle budget only"),
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
