#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

cargo build --release --locked

# POSIX -p works on Linux and macOS; "real" is wall time, in seconds.
printf '\nRunning riscv-tci-vm benchmark (100 iterations)...\n'
/usr/bin/time -p \
    target/release/riscv-tci-vm --stats tests/fixtures/k256_ecdsa_ckbvm 100
printf '\nRunning ckb-debugger benchmark (--mode fast, 100 iterations)...\n'
/usr/bin/time -p \
    ckb-debugger --mode fast --bin tests/fixtures/k256_ecdsa_ckbvm -- 100
