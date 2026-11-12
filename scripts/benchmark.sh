#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

cargo build --release --locked

# Allow the heavier fixtures to complete 100 iterations on both runners.
max_instructions=100000000000
max_cycles=100000000000

# POSIX -p works on Linux and macOS; "real" is wall time, in seconds.
for fixture in tests/fixtures/*_ckbvm; do
    printf '\nRunning riscv-tci-vm benchmark: %s (100 iterations)...\n' "${fixture##*/}"
    /usr/bin/time -p \
        target/release/riscv-tci-vm --stats --max-instructions "$max_instructions" "$fixture" 100
    printf '\nRunning ckb-debugger benchmark: %s (--mode fast, 100 iterations)...\n' "${fixture##*/}"
    /usr/bin/time -p \
        ckb-debugger --mode fast --max-cycles "$max_cycles" --bin "$fixture" -- 100
done
