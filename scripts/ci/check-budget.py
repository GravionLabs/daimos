#!/usr/bin/env python3
"""Compares the criterion results in target/criterion with budget.toml.

Prints a Markdown table (also appended to $GITHUB_STEP_SUMMARY) and exits 1 when a benchmark's p95
per-iteration time exceeds its threshold by more than the tolerance. A benchmark without a
threshold is listed but never fails; a threshold that matches no benchmark fails, so a renamed
benchmark cannot silently leave the budget.
"""
import fnmatch
import json
import math
import os
import sys
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[2]
criterion = Path(os.environ.get("CRITERION_DIR", root / "target" / "criterion"))
budget = tomllib.loads((root / "budget.toml").read_text(encoding="utf-8"))
tolerance = budget.get("tolerance", 0.2)
rules = budget.get("benchmark", [])


def p95_ms(sample: dict) -> float:
    per_iter = sorted(t / n for t, n in zip(sample["times"], sample["iters"]))
    index = max(0, math.ceil(0.95 * len(per_iter)) - 1)
    return per_iter[index] / 1e6  # nanoseconds to milliseconds


results = {}
for info in criterion.glob("**/new/benchmark.json"):
    sample = info.with_name("sample.json")
    if not sample.exists():
        continue
    name = json.loads(info.read_text(encoding="utf-8"))["full_id"]
    results[name] = p95_ms(json.loads(sample.read_text(encoding="utf-8")))

if not results:
    sys.exit(f"no criterion results in {criterion}; run `cargo bench -p daimos-core -p daimos-ext` first")

lines = ["| Benchmark | p95 | Budget | Status |", "| --- | ---: | ---: | --- |"]
failed = False
for name in sorted(results):
    rule = next((r for r in rules if fnmatch.fnmatch(name, r["match"])), None)
    value = results[name]
    if rule is None:
        lines.append(f"| {name} | {value:.3f} ms | none | listed |")
        continue
    limit = rule["p95_ms"] * (1 + tolerance)
    ok = value <= limit
    failed |= not ok
    lines.append(f"| {name} | {value:.3f} ms | {rule['p95_ms']} ms (+{tolerance:.0%}) | {'ok' if ok else 'OVER'} |")

for rule in rules:
    if not any(fnmatch.fnmatch(name, rule["match"]) for name in results):
        failed = True
        lines.append(f"| {rule['match']} | - | {rule['p95_ms']} ms | NO BENCHMARK |")

table = "\n".join(lines)
print(table)
summary = os.environ.get("GITHUB_STEP_SUMMARY")
if summary:
    with open(summary, "a", encoding="utf-8") as out:
        out.write(f"### Performance budget\n\n{table}\n\n")
sys.exit(1 if failed else 0)
