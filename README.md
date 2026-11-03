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

On the development Intel Xeon E5-2670 v2 @ 2.50 GHz x86_64 Linux host, measured
on 2026-11-03 with Debian Clang 19.1.7, Rust 1.95.0, and `ckb-debugger 1.1.1`:

| Runner | Wall time | User CPU time | System CPU time | Guest result |
| --- | ---: | ---: | ---: | ---: |
| riscv-tci-vm before dispatch optimization | 2.47 s | 2.46 s | 0.00 s | 0 |
| ckb-debugger (`--mode fast`), baseline comparison | 2.05 s | 2.04 s | 0.00 s | 0 |
| riscv-tci-vm after dispatch optimization | 2.00 s | 2.00 s | 0.00 s | 0 |
| ckb-debugger (`--mode fast`), final comparison | 1.98 s | 1.97 s | 0.00 s | 0 |

riscv-tci-vm improved by **1.24× in wall time** (2.47 / 2.00) and **1.23× in
execution time** (2,468.087 / 2,002.720 ms). Its final wall time was about 1%
longer than ckb-debugger. Wall time includes process startup, ELF loading,
execution, and cleanup; the build is excluded. These are single-run local
measurements, not averages or a portable throughput guarantee.

riscv-tci-vm reported **384,466,918 retired instructions** and **1,320 translated
blocks** both before and after. Loading was 0.259 / 0.812 ms; execution throughput
was 155.78 / 191.97 MIPS, including translation from an empty cache. ckb-debugger
reported **725,894,181 CKB cycles** in both runs. CKB cycles and retired
instructions are different measures; the runner comparison uses wall time.

The dispatch experiment changed only native compiler settings. Clang 19's
default tail-duplication predecessor/successor limits of 16 constrain the
86-opcode dispatch graph. `build.rs` raises both limits and the instruction
thresholds (`tail-dup-size`, `tail-dup-indirect-size`, and
`tail-dup-placement-threshold`) to 128. Disassembly confirms that the retained
configuration expands `tci_run` from **2,750 to 8,781 bytes** and from **5 to 150
indirect jump sites**. For example, `Add` now ends in its own immediate/register
dispatch branches instead of jumping to the common result/dispatch tail.
Guest semantics, bytecode, operand fetching, and budget checks are unchanged.
The larger host code footprint can be a tradeoff on other CPUs and workloads;
these LLVM options and the generated topology should be rechecked on compiler
upgrades or opcode-table changes.

Each experiment below was built and measured with the same single-run procedure;
speedups use the fresh 2,468.087 ms baseline, not the preceding experiment.
Every VM run exited 0 with 384,466,918 retired instructions and 1,320 blocks.

| Experiment | VM wall | VM execution | Execution speedup | Indirect jump sites | `tci_run` bytes | ckb-debugger wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline | 2.47 s | 2,468.087 ms | 1.00× | 5 | 2,750 | 2.05 s |
| Placement threshold 128 only | 2.49 s | 2,494.809 ms | 0.99× | 5 | 3,213 | 2.07 s |
| All three instruction thresholds 128, tail merging disabled | 2.33 s | 2,333.022 ms | 1.06× | 1 | 12,392 | 2.07 s |
| Original flags, explicit `Add` dispatch fast path | 2.37 s | 2,374.623 ms | 1.04× | 1 | 2,770 | 2.01 s |
| All five thresholds/limits 128 (retained) | 1.93 s | 1,928.545 ms | 1.28× | 150 | 8,781 | 1.95 s |
| Only indirect-size and predecessor/successor limits 128 | 2.07 s | 2,077.541 ms | 1.19× | 150 | 8,458 | 2.01 s |
| Retained configuration, final rebuild | 2.00 s | 2,002.720 ms | 1.23× | 150 | 8,781 | 1.98 s |

Separate follow-up measurements ran baseline, retained, and reduced settings
sequentially for three rounds. Median execution times were **2,470.002**,
**1,986.822**, and **2,017.289 ms**, respectively: a **1.24×** median speedup for
the retained settings over baseline. These additional runs are separate from the
required single-run comparison and show some host timing variability.

Separate `perf stat` runs (all counters active 100% of the time) measured host
cycles falling from 6,838,753,631 to 5,762,329,251, host instructions from
10,054,070,665 to 9,430,645,665, and retired branch misses from 139,442,113 to
108,003,195. Executed indirect-jump mispredictions fell from 137,781,220 /
388,563,436 (**35.46%**) to 107,536,573 / 389,686,400 (**27.60%**). These
speculative indirect-jump events differ from retired-branch events; they support
improved prediction but do not isolate its exact contribution to elapsed time.
Profiler timings are excluded from the speedups above. The event groups are
`{cycles:u,instructions:u,branches:u,branch-misses:u}` and
`{br_inst_exec.all_indirect_jump_non_call_ret:u,br_misp_exec.all_indirect_jump_non_call_ret:u}`;
run each separately as described in TODO.md.

To inspect the generated topology after a release build:

```sh
objdump -d --disassemble=tci_run target/release/riscv-tci-vm > target/tci-run.asm
rg -c 'jmp.*\*' target/tci-run.asm
nm -S target/release/riscv-tci-vm | rg ' tci_run$'
```

### Safe memory-access paths (TODO item 4)

Measured on 2026-11-03 on the same Xeon E5-2670 v2 host with Debian Clang
19.1.7, Rust 1.95.0, and ckb-debugger 1.1.1. The fresh baseline already includes
the dispatch optimization above. Each experiment was built and measured with
the single-run procedure, sequentially without an extra warmup. Speedups below
are relative to this fresh baseline, not the preceding experiment.

| Experiment | VM wall | VM execution | Wall speedup | Execution speedup | ckb-debugger wall |
| --- | ---: | ---: | ---: | ---: | ---: |
| Fresh baseline | 2.06 s | 2,061.165 ms | 1.00× | 1.00× | 1.97 s |
| Same-page guard | 2.27 s | 2,270.152 ms | 0.91× | 0.91× | 2.01 s |
| Natural-alignment guard | 1.94 s | 1,941.520 ms | 1.06× | 1.06× | 2.05 s |
| Separate last-read/write-page caches | 1.95 s | 1,950.552 ms | 1.06× | 1.06× | 2.13 s |
| Alignment guard with likely-aligned hint (retained) | 2.03 s | 2,034.066 ms | 1.01× | 1.01× | 2.04 s |
| Same-page guard with likely-same-page hint | 2.00 s | 1,998.181 ms | 1.03× | 1.03× | 2.12 s |
| Retained alignment guard, final rebuild | 1.94 s | 1,944.300 ms | **1.06×** | **1.06×** | 2.13 s |

All VM runs exited 0, retired **384,466,918 instructions**, and translated
**1,320 blocks**. Baseline/final VM user CPU times were 2.05 / 1.94 s and system
times were both 0.00 s. Loading took 0.398 / 0.244 ms and throughput was
186.53 / 197.74 MIPS. The baseline/final comparator user CPU times were
1.97 / 2.12 s, with 0.00 s system time; both exited 0 and reported
725,894,181 CKB cycles.

Single-run variation was substantial. An initial three-round baseline/alignment
comparison had median execution of 1,945.552 / 1,955.232 ms (0.995×).
A second three-round comparison rotating baseline, alignment, and cached
variants had medians of 1,947.022 / 1,933.162 / 1,980.756 ms. The extra cache
state did not establish a benefit and was discarded. Finally, five sequential
rounds rotating the runner order compared the branch-layout alternatives:

| Variant | Median execution | Range | Median speedup |
| --- | ---: | ---: | ---: |
| Baseline | 1,973.133 ms | 1,898.557–2,060.890 ms | 1.00× |
| Alignment guard | 2,016.383 ms | 1,914.305–2,074.189 ms | 0.98× |
| Alignment guard with hint (retained) | 1,898.504 ms | 1,863.461–2,002.955 ms | **1.04×** |
| Same-page guard with hint | 2,038.600 ms | 1,896.923–2,053.878 ms | 0.97× |

These additional runs are separate from the required single-run comparison.
The overlapping ranges mean the modest gain should be remeasured on other
hosts/workloads; the first alignment-only timing overstated its repeatability.

The retained helper keeps the subtraction-based RAM bounds check and first-page
permission check. For the supported 1/2/4/8-byte widths, natural alignment proves
the access cannot cross a 4 KiB page, so the second permission lookup is skipped.
The branch hint makes aligned accesses fall through directly in the generated
load/store handlers. Unaligned accesses retain both endpoint checks before the
existing `memcpy`; faulting stores cannot modify memory. No permission cache or
Rust/C ABI change is retained. Disassembly shows 150 indirect dispatch sites
unchanged and `tci_run` growing from 8,781 to 8,812 bytes.

Validation passed: release tests, Clippy with warnings denied, the assembled ISA
test against QEMU, and the README UBSan command. New C-interpreter regression
coverage exercises every load/store width around page and RAM boundaries,
denied endpoint permissions, large/wrapping-range addresses, sign extension,
exact fault PCs/addresses and retirement, prior side effects, whole-memory
comparison for partial stores, and budget stops/resumption at the access.
Local experiment binaries, source snapshots, assembly, and repeat logs are in
ignored `target/memory-paths/`; the results above survive `cargo clean`.
