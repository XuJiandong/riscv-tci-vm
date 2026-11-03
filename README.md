# riscv-tci-vm

A single-hart RV64IMCB interpreter for x86_64, with a Rust entry point, goblin
ELF loading, and a Clang 19 C execution loop. It interprets cached register
bytecode using QEMU TCI's architecture. It never generates or executes native
machine code.

## Build and run

Requires Rust 1.95.0, `clang-19`, and `ar`. The Rust toolchain is pinned in
`rust-toolchain.toml`; dependencies are pinned in `Cargo.lock`.

```sh
cargo build --release --locked
target/release/riscv-tci-vm --stats \
  tests/fixtures/k256_ecdsa_ckbvm 1
```

This executes the bundled cryptography workload and exits with code **0**.
The ELF in `tests/fixtures/k256_ecdsa_ckbvm` was moved from
`ckb-vm-contrib/ckb-vm-test-suite/programs/build/release/k256_ecdsa_ckbvm`.
It retires **3,844,947 instructions**. The number after the ELF path is a guest
argument. Arguments are passed exactly as supplied: the host ELF filename is
**not** inserted into `argv`. This workload expects a numeric `argv[0]`.

```text
riscv-tci-vm [--stats] [--max-instructions N] ELF [ARG ...]
```

The default limit is one billion instructions. Guest debug output and optional
statistics go to stderr. Syscall 93 exits with `a0` interpreted as an `int8_t`;
the CLI maps that to its corresponding 8-bit process exit status. Syscall 2177
writes the NUL-terminated byte string at `a0`, without adding a newline or
changing guest registers. Unknown syscalls, illegal instructions, breakpoints,
memory faults, and exhausted budgets produce an error and process status 1.

## Architecture

The design follows the local QEMU reference in `/home/xjd/projects/qemu`,
particularly `tcg/tci.c` and `tcg/tci/tcg-target.h`: a frontend lowers guest
instructions to register operations, caches translation blocks, and runs those
operations in an interpreter with explicit guest-memory helpers and exits.
This is an independent, specialized implementation, not QEMU's binary bytecode
format or its complete TCG framework. No CKB-VM implementation source was used.

* `src/decode.rs` lowers 16- and 32-bit instructions into common register IR.
  Branch destinations and immediate values are resolved during translation.
* `src/ir.rs` and `src/ops.def` define 24-byte bytecode instructions and a shared
  Rust/C opcode numbering. Guest registers occupy fixed virtual registers;
  writes to x0 use a private discard slot, preserving load faults and other
  effects. This avoids a general-purpose register allocator in the frontend.
* `src/vm.rs` translates blocks lazily, with at most 64 guest instructions per
  block. A dense halfword-PC table lets the C loop chain cached successors
  without returning to Rust. Rust handles cache misses, syscalls, and errors.
* `src/tci.c` uses computed-goto dispatch, unsigned wrapping arithmetic, and
  explicit divide-by-zero/overflow behavior. `build.rs` tunes Clang 19's tail
  duplication to retain dispatch branches at individual handlers, giving the
  host branch predictor more opcode context. Unaligned accesses use `memcpy`
  after checking the complete access. Naturally aligned loads/stores check
  only the first page's permission because their widths divide the page size;
  unaligned accesses check both endpoints. Every instruction has an associated
  guest PC, so faults and instruction-budget stops are precise.
* The cache is bounded to about one million IR operations before flushing on a
  miss. Executable guest pages and their mappings are immutable after loading,
  so cached translations cannot become stale. No cache pointer survives a
  flush inside the interpreter.

The bytecode and guest bytes remain ordinary non-executable host data. There is
no JIT, executable allocation, runtime machine-code patching, or native guest
code execution. Host compiler optimization is performed only at build time.

## ISA and memory

RV64I, M, and the RV64C integer forms are implemented. B means the ratified
Zba/Zbb/Zbs set described in the [RISC-V bit-manipulation specification](https://docs.riscv.org/reference/isa/unpriv/b-st-ext.html).
Zbc carry-less multiplication is also implemented because the supplied ELF
advertises it. FENCE and FENCE.I are no-ops in the single-hart, immutable-code
model. This is a user-mode execution environment: atomics (A), floating point,
vectors, privileged instructions, CSRs, dynamic linking, and general Linux
syscalls are not implemented.

Guest RAM is exactly **4 MiB**, divided into **1,024 pages of 4 KiB**. The first
page is inaccessible. Other RAM defaults to RW, and ELF image pages receive the
union of their segments' permissions. Any page that would be both writable and
executable is rejected, including conflicts between different segments sharing
a page. Fetches require X, loads/debug strings require R, and stores require W.
Cross-page accesses check every touched page before making changes.

The loader accepts static little-endian ELF64 RISC-V executables, rejects
overlapping segment byte ranges and malformed ranges/alignment, copies file
data, and zeroes BSS. It reserves the top 256 KiB for the stack and arguments.
SP is 16-byte aligned; the stack contains argc, argv, an empty environment, and
AT_NULL. Registers a0/a1 also contain argc/argv at entry. Translation storage is
host metadata outside the 4 MiB guest address space; the dense block table alone
occupies 16 MiB of host virtual memory.

The library exposes `Vm::from_elf`, `Vm::run`, read-only registers/PC, and retired
instruction counts. `run` takes an additional instruction budget and a `Write`
sink for debug output. An instruction-limit error leaves the VM resumable.

## Validation

```sh
cargo test --release --locked
cargo clippy --all-targets --locked -- -D warnings

# Run only the bundled k256 test (override the ELF path with K256_ELF).
cargo test --release --locked --test k256

# Assemble an independent guest and run it here and in QEMU.
# Requires ld.lld and qemu-riscv64 in addition to clang-19.
sh scripts/check-isa.sh

# Instrument the C interpreter with undefined-behavior checks.
# Requires the host UBSan runtime (libubsan).
TCI_SANITIZE=1 CARGO_TARGET_DIR=target/ubsan \
  cargo test --locked
```

The Rust tests cover over 21,000 arithmetic/bit-manipulation vectors, signed and
unsigned divide corners, x0, aliasing JALR, memory widths, unaligned access,
cross-page permissions, executable-page writes, NX execution, malformed ELFs,
BSS, startup arguments, syscalls, precise faults, and budget/resume behavior.
`tests/isa.S` independently checks assembler encodings, all RV64C integer forms,
and M/B/Zbc operations against QEMU. The bundled k256 test runs in the default
test suite, checks exit code 0, and confirms that over one million guest
instructions were executed.

## Performance

Compare a release build with `ckb-debugger` in `fast` mode using the same bundled
ELF. Pass `100` as guest `argv[0]` to execute 100 workload iterations within one
guest invocation, amortizing guest startup and cleanup. Run each command once,
sequentially, without an extra warmup run:

```sh
cargo build --release --locked
/usr/bin/time -f 'wall=%e s user=%U s system=%S s' \
  target/release/riscv-tci-vm --stats tests/fixtures/k256_ecdsa_ckbvm 100
/usr/bin/time -f 'wall=%e s user=%U s system=%S s' \
  ckb-debugger --mode fast --bin tests/fixtures/k256_ecdsa_ckbvm -- 100
```

The table below reports the latest measurements of the current implementation.
After each retained optimization, update this table and its summary using the
final benchmark run of both runners; keep earlier results in the historical
experiment sections below.

On the development Intel Xeon E5-2670 v2 @ 2.50 GHz x86_64 Linux host, measured
on 2026-11-03 with Debian Clang 19.1.7, Rust 1.95.0, and `ckb-debugger 1.1.1`:

| Runner | Wall time | User CPU time | System CPU time | Guest result |
| --- | ---: | ---: | ---: | ---: |
| riscv-tci-vm (current: dispatch, memory-access, and superinstruction optimizations) | 2.00 s | 1.99 s | 0.00 s | 0 |
| ckb-debugger (`--mode fast`) | 2.18 s | 2.18 s | 0.00 s | 0 |

The current VM executed in **1,944.300 ms**, retiring **384,466,918 instructions**
across **1,320 translated blocks**, with 0.244 ms loading and 197.74 MIPS.
Its wall time was about **8.9% shorter** than ckb-debugger in this comparison;
ckb-debugger reported **725,894,181 CKB cycles**. CKB cycles and retired
instructions are different measures; the runner comparison uses wall time.
Wall time includes process startup, loading, execution, and cleanup, excluding
the build. These are single-run local measurements with substantial timing
variation, not averages or a portable throughput guarantee. The latest
[memory-access optimization](#safe-memory-access-paths-todo-item-4) improved
execution by **1.06×** against its fresh baseline, with **1.04×** median speedup
in five additional rounds.
