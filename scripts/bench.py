#!/usr/bin/env python3
"""Repeat the guide.md workload, including translation, without external packages."""
import argparse
from pathlib import Path
import re
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("elf", type=Path)
parser.add_argument("--iterations", type=int, default=100)
parser.add_argument("--runs", type=int, default=5)
args = parser.parse_args()
if args.runs < 1 or args.iterations < 1:
    parser.error("runs and iterations must be positive")
root = Path(__file__).resolve().parent.parent
subprocess.run(["cargo", "build", "--release", "--offline"], cwd=root, check=True)
command = [str(root / "target/release/riscv-tci-vm"), "--stats", "--max-instructions",
           str(10_000_000 * args.iterations + 1_000_000), str(args.elf.resolve()), str(args.iterations)]
samples = []
counts = set()
for run in range(args.runs + 1):
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    match = re.search(r"instructions=(\d+).*run=([\d.]+)ms", result.stderr)
    if not match:
        raise RuntimeError(f"Missing VM statistics: {result.stderr}")
    count, elapsed = int(match[1]), float(match[2])
    if run:  # First subprocess warms host caches; each run creates a new VM.
        samples.append(elapsed)
        counts.add(count)
        print(f"run {run}: {elapsed:.3f} ms, {count / elapsed / 1000:.2f} MIPS")
if len(counts) != 1:
    raise RuntimeError(f"Instruction counts varied: {counts}")
median = statistics.median(samples)
print(f"median: {median:.3f} ms, {counts.pop() / median / 1000:.2f} MIPS; "
      f"range: {min(samples):.3f}–{max(samples):.3f} ms")
