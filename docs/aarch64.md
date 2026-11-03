# Native AArch64 port and Apple M4 measurements

Measured on 2026-11-03 on the user's Apple M4 Mac, with Rust 1.95.0,
Homebrew Clang 19.1.7, and ckb-debugger 1.1.1. Both benchmark executables
are Mach-O ARM64 binaries. README prose outside the performance table remains
the historical x86_64 description, as required by the repository instructions.

## Port

The build accepts little-endian AArch64 as well as x86_64 and explicitly
passes Cargo's target to Clang so the Rust and C objects target the same
architecture. Normal native builds still use `cargo build --release --locked`.
There is no M4-only instruction requirement or change to the interpreter's
execution algorithm, memory checks, instruction accounting, or dispatch flags.
Clang's default Apple ARM64 CPU target is apple-m1 on this installation.

The existing C operations, including 128-bit multiplication and computed
gotos, compile directly to AArch64. The C layout assertions pass, and the
Rust/C interface is exercised by the test suite. Division corner cases remain
explicit because C signed overflow and division by zero are undefined even
on a CPU whose division instructions do not trap.

`TCI_SANITIZE=1` now locates Clang's macOS compiler-rt UBSan library and adds
its link search path and runtime search path. Non-macOS builds retain the
existing libubsan linkage. Other host platforms were not executed here.

## Estimate and baseline

Before the port, the provisional estimate was 0.5–1.2 seconds for the bundled
100-iteration workload. The original source cannot run natively on this Mac:
its build script fails with `x86_64 host required`. No emulated x86_64 baseline
was measured. The historical Xeon result in README was 2.00 seconds wall time.

The first native baseline changed only the architecture allowlist, retaining
all original compiler flags and interpreter code. Each measurement pair follows
README's procedure: release build, one VM invocation, then one ckb-debugger
invocation, sequentially, without an extra warmup. On macOS use POSIX `time -p`
because `/usr/bin/time` does not support GNU `-f`:

```sh
cargo build --release --locked
/usr/bin/time -p target/release/riscv-tci-vm --stats tests/fixtures/k256_ecdsa_ckbvm 100
/usr/bin/time -p ckb-debugger --mode fast --bin tests/fixtures/k256_ecdsa_ckbvm -- 100
```

| Stage | Runner | Wall | User | System | Guest result |
| --- | --- | ---: | ---: | ---: | ---: |
| Architecture allowlist only | riscv-tci-vm | 0.72 s | 0.42 s | 0.00 s | 0 |
| Architecture allowlist only | ckb-debugger fast | 0.36 s | 0.35 s | 0.00 s | 0 |
| Completed port | riscv-tci-vm | 0.47 s | 0.40 s | 0.00 s | 0 |
| Completed port | ckb-debugger fast | 0.36 s | 0.35 s | 0.00 s | 0 |

VM execution changed from 429.404 ms (895.35 MIPS) to 402.467 ms
(955.28 MIPS); both retired 384,466,918 instructions across 1,320 translated
blocks. Final loading took 0.403 ms. Both comparator runs reported
725,894,181 CKB cycles, which are not retired instruction counts.

The observed before/after ratios are 1.53× by wall time and 1.07× by VM
execution time. No execution optimization was made, so these single-run
differences should not be interpreted as an optimization gain. Wall time also
includes process startup and cleanup. The final VM wall time is 4.26× faster
than the historical Xeon measurement, a comparison across different machines,
and 30.6% longer than ckb-debugger on this Mac.

## Validation

```sh
cargo test --release --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
TCI_SANITIZE=1 CARGO_TARGET_DIR=target/ubsan cargo test --locked
```

The independent `tests/isa.S` guest was assembled with Clang/LLD and passed
both this VM (282 retired instructions) and ckb-debugger (1,306 CKB cycles).
QEMU user-mode execution was unavailable on this Mac, so the QEMU comparison
in `scripts/check-isa.sh` was not run.
