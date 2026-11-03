#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p target
clang-19 --target=riscv64-unknown-elf -march=rv64imc_zba_zbb_zbc_zbs \
    -mabi=lp64 -nostdlib -fuse-ld=lld -Wl,-T,tests/guest.ld \
    tests/isa.S -o target/isa.elf
cargo build --release --offline
target/release/riscv-tci-vm --stats target/isa.elf
qemu-riscv64 target/isa.elf
echo "Clang-assembled ISA checks passed in both interpreters."
