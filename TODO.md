# Benchmark bottlenecks and optimization backlog

Analysis date: 2026-11-03. Source revision: `cfac7a872eba7f71c39dfb681205bf2d160edbda`.
Scope: the bundled `k256_ecdsa_ckbvm` workload with guest argument `100`.
This began as an analysis and implementation backlog without runtime or build
changes. Item 1 is now implemented; its results are recorded below and in
README.md. Diagnostic instrumentation exists only in an ignored copy.
Priorities reflect this workload, expected reach, effort, and correctness risk.
Proposed benefits are hypotheses, not measured speedups.

## Baseline

Followed README.md's Performance procedure: one release build, then one run of
each runner, sequentially, without an extra warmup. Profiling happened afterward.
Did not use `scripts/bench.py`, which uses a different warmup/repetition policy.

```sh
cargo build --release --locked
/usr/bin/time -f 'wall=%e s user=%U s system=%S s' \
  target/release/riscv-tci-vm --stats tests/fixtures/k256_ecdsa_ckbvm 100
/usr/bin/time -f 'wall=%e s user=%U s system=%S s' \
  ckb-debugger --mode fast --bin tests/fixtures/k256_ecdsa_ckbvm -- 100
```

Host: Intel Xeon E5-2670 v2 @ 2.50 GHz, x86_64 Linux. C compiler:
Debian Clang 19.1.7. Installed comparator: **ckb-debugger 1.1.1**, whereas the
existing README measurement used 1.0.0; these are separate observations.

| Runner | Wall | User | System | Exit |
| --- | ---: | ---: | ---: | ---: |
| riscv-tci-vm release | 2.44 s | 2.43 s | 0.00 s | 0 |
| ckb-debugger fast | 2.08 s | 2.08 s | 0.00 s | 0 |

VM statistics: **384,466,918** retired instructions, **1,320** translated blocks,
**1.038 ms** loading, **2,444.145 ms** execution, **157.30 MIPS**. Comparator:
**725,894,181** CKB cycles. The VM's measured wall time was approximately **17.3%
longer**; CKB cycles are not retired-instruction counts. Single-run timings are
not statistical estimates. No optimization was applied, so before/after speedup
is **not applicable**.

## Flamegraph evidence and limitations

P0 hardware profiling completed after the host's `perf_event_paranoid` was changed
from 3 to 2. Recording still needed **`--user-callchains`** in addition to
`cycles:u`: the default DWARF recording requested kernel call chains, which the
host policy rejects. Recording and counter collection ran outside the sandbox,
without sudo or further host-policy changes.

The unchanged release executable completed the `100`-iteration guest with exit 0,
384,466,918 retired instructions, and 1,320 translated blocks. The hardware capture
contains **2,461 samples, zero lost samples**, and **6,847,557,773 sampled cycles**
(the sum of sample periods). `tci_run` accounts for **99.96% of sampled cycles**
and 2,458 samples. Profiled execution was 2,450.704 ms; this is not a replacement
for the unprofiled baseline above.

Local artifacts are in the ignored `target/profiling/` directory:

- [Hardware stack flamegraph](target/profiling/perf-flamegraph.svg), whose widths
  count samples, and [cycle-weighted PC-region flamegraph](target/profiling/perf-flamegraph-regions.svg).
- [Raw perf capture](target/profiling/perf.data),
  [perf report](target/profiling/perf-report.txt),
  [sampled stacks](target/profiling/perf-stacks.txt), and
  [sampled leaf PCs/periods](target/profiling/perf-leaves.txt).
- [Annotated disassembly](target/profiling/perf-annotated-disassembly.tsv),
  [aggregation script](target/profiling/summarize-perf.py), and
  [region summary](target/profiling/perf-summary.txt).
- [Diagnostic-copy builder](target/profiling/build-diagnostic.py),
  [dynamic counters](target/profiling/diagnostic.tsv),
  [diagnostic aggregation script](target/profiling/summarize-diagnostic.py), and
  [dynamic-count summary](target/profiling/diagnostic-summary.txt).

These artifacts are local, not committed, and disappear on `cargo clean`. The
measurements below preserve the findings independently of those files.

The C object normally has no debug information. For address attribution only,
`src/tci.c` was compiled to a temporary object using the existing build flags plus
`-g`; it was **not linked into the benchmark**. Its `.text` was byte-for-byte
identical to the release C object's `.text` (SHA-256
`2588b50dbb96f0e48b68281ffd38168605b75a9d56f0c09e056997c7640d84fe`).
`perf annotate` crashed locally, including with source display disabled, so the
annotation artifact joins `perf script` symbol offsets/sample periods with this
matching object's disassembly and DWARF locations. Shared macro-generated tails
often have source line zero. PC-region leaves are synthetic classifications, not
additional function calls; offset ranges are in `summarize-perf.py`.

| Sampled PC region | Samples | Share of sampled cycles | Interpretation |
| --- | ---: | ---: | --- |
| Shared dispatch, operand fetch, register writeback, budget/accounting | 1,100 | 44.95% | Largest candidate for reducing overhead per guest instruction |
| Arithmetic, bit operations, and other interpreter code | 590 | 23.89% | Includes handler branches/padding, not just arithmetic instructions |
| Load/store bounds and permission checks | 404 | 16.37% | Repeated checks in `accessible()` |
| Load/store address calculation, data access, handler branches | 171 | 6.95% | Together with checks, memory paths account for 23.32% |
| Guest control flow and block lookup | 193 | 7.80% | Branch/jump handling and cached-successor validation/lookup |
| Outside `tci_run` | 3 | 0.04% | Startup/loading/translation are small in this capture |

Percentages weight each sample by its recorded cycle period; raw sample counts
alone produce slightly different percentages. Ordinary `cycles:u` sampling has
instruction-pointer skid and limited statistical resolution: for example, a NOP
near the shared writeback tail receives samples. These are approximate region
costs, not independently removable costs or proof that individual instructions
consume the reported cycles. Aggregate hardware counters and exact dynamic counts
below provide complementary evidence. A function flamegraph alone cannot separate
the many inlined opcode handlers inside `tci_run`.

The earlier GDB capture remains available in
[the original flamegraph](target/profiling/flamegraph.svg). Its 5,714 samples put
45.13% in shared dispatch/bookkeeping and 23.67% in memory paths, consistent with
the hardware profile, but its debugger-induced overhead and sampling bias make it
secondary evidence now.

Disassembly supports several concrete observations:

- Many handlers converge on shared result/dispatch code around `tci_run+0x8f9`.
  The common indirect branches at `+0x932` and `+0x952` mean source-level
  computed gotos do not guarantee a distinct dispatch branch per opcode.
- Dispatch selects immediate versus register operands before entering handlers,
  including handlers that do not need both generic operands.
- Ordinary instruction tails increment `steps`, test the `Link` exception, and
  check the limit. Block transitions perform additional budget checks.
- Loads and stores calculate addresses, check the full RAM range, and check
  permissions of both endpoint pages; ordinary same-page accesses repeat the
  page lookup. The constant-size `memcpy` operations already become native
  scalar loads/stores, not calls to a copying routine.
- Successor lookup repeats alignment, range, X-permission, and cache-entry checks.
  It already chains cached blocks within C; this is not a Rust round trip on
  every guest branch.
- `Mul`/`Mulhu` already use native multiply instructions. A cryptography workload
  does not by itself establish carry-less multiplication or division as a hotspot.

## Prioritized implementation candidates

Items are ordered within each priority. Several are alternative or overlapping
experiments; their possible gains must not be added together.

### P0 — Measurement completed

- [x] **0. Capture hardware profiles and diagnostic execution counts.** Completed
  userspace cycle sampling, stack-flamegraph generation, annotated hot-PC analysis,
  branch/cache/TLB counting, and a separate instrumented-copy run. All retained
  counter groups ran for **100.00%** of enabled time, with no multiplex scaling.
  The initial generic four-event cache and TLB groups returned `<not counted>`;
  those are failed measurements, not zero misses. Smaller groups and supported
  CPU-specific events provided the results below. Every successful benchmark run
  exited 0 and retired 384,466,918 guest instructions.

#### Hardware counters

Counter sets were collected in separate sequential invocations of the unchanged
release executable. Cross-run ratios are approximate. `:u` was applied to every
hardware event; event names below omit it for readability.

| Measurement | Result | Meaning |
| --- | ---: | --- |
| Host cycles | 6,835,462,489 | 17.78 host cycles per retired guest instruction |
| Host instructions | 10,054,070,739 | 26.15 host instructions per guest instruction; IPC 1.47 |
| Retired branches / branch misses | 2,485,243,368 / 139,568,763 | 5.62% misprediction rate overall |
| Executed indirect jumps / mispredictions | 388,582,570 / 137,567,253 | 35.40% for non-call/non-return indirect jumps; these are speculative execution events |
| Retired conditional branch misses | 2,207,815 | Far fewer than the indirect-jump misprediction count; different event semantics and runs prevent an exact partition |
| Recovery cycles | 660,940,250 | 9.66% of 6,844,358,636 cycles in that same run; not the full cost of misprediction |
| Retired load uops / L1-missing load uops | 3,534,628,915 / 8,315,214 | Approximately 0.235% miss ratio |
| LLC references / misses | 294,650 / 837 | 0.284% of LLC references missed; not a ratio of all loads |
| Instruction-cache misses | 27,571 | Low absolute count relative to about 10 billion host instructions |
| Completed data-TLB load/store walks | 5,647 / 720 | Few accesses require page-table walks |
| Data-TLB load/store STLB hits | 1,062,193 / 5,288 | First-level misses resolved in the second-level TLB |
| Completed instruction-TLB walks / STLB hits | 663,647 / 131,342 | Collected in separate runs |
| Instruction-TLB walk-duration cycles | 13,170,718 | 0.192% of cycles in the same run; not a direct stall-time percentage |

Raw results:
[core](target/profiling/perf-core.csv),
[indirect branches](target/profiling/perf-indirect.csv),
[recovery and walk duration](target/profiling/perf-recovery.csv),
[L1 loads](target/profiling/perf-l1d.csv),
[LLC](target/profiling/perf-llc.csv),
[instruction cache and completed walks](target/profiling/perf-icache.csv), and
[STLB hits](target/profiling/perf-tlb-detail.csv).

**Decision:** dispatch prediction and the volume of interpreter instructions
remain the leading targets. Indirect-jump mispredictions are comparable in
magnitude to all retired branch misses, consistent with shared computed-goto
dispatch being the main prediction problem. The counters do not establish its
exact share of stall time. Cache/TLB results give little support to prioritizing
DRAM bandwidth, huge pages, or generic prefetching. The 23.32% sampled memory
region includes address/check/dispatch work; it must not be read as 23.32% waiting
for RAM. Separate assembly analysis is still required to isolate dependency-chain
costs from other execution overhead.

#### Exact dynamic counts from an isolated diagnostic copy

Only copied files under `target/profiling/diagnostic/` were instrumented; the
project's `src/`, build files, lockfile, and release executable retain their
original SHA-256 hashes. The copied run uses the same guest and argument. Its
3,701.539 ms instrumented execution is excluded from all performance comparisons.
Counters cover executed opcode forms/pairs, block visits, branch target edges,
load/store page/alignment properties, C invocations, translations, and flushes.
The guest completed without faults or budget stops, so dispatched guest-op counts
match retirement. Synthetic `Link` operations are counted separately.

| Measurement | Result | Implication |
| --- | ---: | --- |
| Guest instructions | 384,466,918 | Matches release execution exactly |
| Dynamic block visits | 34,380,247 | Mean 11.18 guest instructions per block |
| Three- or four-instruction block visits | 17,832,517 (51.87%) | Many transitions come from short real blocks |
| Synthetic `Link` executions | 3,307,901 | 9.62% of block exits; 0.86% extra operations relative to guest instructions |
| Translated blocks / cache flushes | 1,320 / 0 | No cache-cap churn |
| Cached IR records / bytes | 18,461 / 443,064 | Excludes block metadata and the dense lookup table |
| C invocations / translation misses | 1,321 / 1,320 | Cached successors stay inside C |
| Guest loads / stores | 64,018,645 / 56,752,648 | 120,771,293 total memory accesses |
| Same-page / naturally aligned accesses | 120,771,293 / 120,771,293 | Both properties hold for 100% of this workload's accesses; guards remain necessary for other guests |
| Observed branch/link source-target edges | 2,126 from 1,289 source PCs | Full edge counts are in `diagnostic.tsv` |
| Indirect-jump sites | 881, of which 854 have one observed target | Static site counts alone overstate inline-cache hit potential |
| Per-site dominant-target coverage for indirect jumps | 54.45% of 7,063,908 executions | A few polymorphic sites dominate; one site has 239 distinct targets |

Dominant guest opcodes are `Add` (23.19%), `Ld` (14.50%), `Sd` (12.15%), `Shr`
(7.01%), `Mul` (6.30%), `Ltu` (5.82%), and `Mulhu` (5.65%). No division, remainder,
carry-less multiplication, population-count, or `Orcb` operations executed.

| Frequent adjacent pair within one translated block | Executions |
| --- | ---: |
| `Sd → Sd` | 24,069,727 |
| `Ld → Ld` | 21,684,125 |
| `Ld → Add` | 20,762,305 |
| `Add → Add` | 19,474,433 |
| `Add → Ltu` | 17,026,200 |
| `Ltu → Add` | 16,349,000 |
| `Add → Bltu` | 8,953,400 |
| `Mulhu → Mul` | 7,821,400 |

Pairs overlap and counts do not establish operand compatibility or the number of
safely fusible operations. `Mulhu → Mul` requires matching inputs and correct
alias handling; paired memory operations must preserve ordered side effects and
precise second-access faults. The counts identify which sequences to inspect.
For example, the branch edge `0x1dc4e → 0x1dc52` executes 1,210,200 times; the raw
edge table can guide hot-path formation. Per-site dominant-target coverage is
not a measured last-target-cache hit rate.

Counter checks reconcile opcode totals with retirement, weighted block lengths
with retirement, within-block pair counts with instructions minus block visits,
edge counts with visits minus the final exit block, and memory counters with
load/store opcodes. No semantic optimizations were made.

**Priority refinements:** keep dispatch first; specialize the frequent `Add`,
`Ld`, and `Sd` forms; test guarded same-page/aligned memory paths; select fusion
candidates from the pair table. Direct successor caching is better supported
than assuming an indirect-target cache will hit reliably. A larger block cap can
remove synthetic links but cannot address most real branch transitions. Defer
rare-ISA acceleration, eviction, loader work, and broad cache/TLB tuning for this
benchmark.

#### Reproduction

Run from the repository root. Hardware commands need execution outside the
sandbox with `perf_event_paranoid=2`; kernel profiling is not required.

```sh
cargo build --release --locked
mkdir -p target/profiling
perf record -F 999 -e cycles:u --user-callchains --call-graph dwarf,16384 \
  --no-buildid-cache -o target/profiling/perf.data -- \
  target/release/riscv-tci-vm --stats tests/fixtures/k256_ecdsa_ckbvm 100
perf script -i target/profiling/perf.data > target/profiling/perf-stacks.txt
inferno-collapse-perf target/profiling/perf-stacks.txt > target/profiling/perf.folded
inferno-flamegraph target/profiling/perf.folded > target/profiling/perf-flamegraph.svg
perf script -i target/profiling/perf.data --hide-call-graph \
  -F ip,sym,symoff,period > target/profiling/perf-leaves.txt
```

Collect each following event group in its own sequential run, substituting it
for `GROUP` below, and retain the running-time percentage:

```sh
perf stat -x , -e 'GROUP' -- target/release/riscv-tci-vm --stats \
  tests/fixtures/k256_ecdsa_ckbvm 100
```

```text
{cycles:u,instructions:u,branches:u,branch-misses:u}
{br_inst_exec.all_indirect_jump_non_call_ret:u,br_misp_exec.all_indirect_jump_non_call_ret:u}
{cycles:u,br_misp_retired.conditional:u,int_misc.recovery_cycles:u,itlb_misses.walk_duration:u}
{mem_uops_retired.all_loads:u,mem_load_uops_retired.l1_miss:u}
{cache-references:u,cache-misses:u}
{icache.misses:u,itlb_misses.walk_completed:u,dtlb_load_misses.walk_completed:u,dtlb_store_misses.walk_completed:u}
{dtlb_load_misses.stlb_hit:u,dtlb_store_misses.stlb_hit:u,itlb_misses.stlb_hit:u}
```

CPU-specific events must be checked against `perf list` on a different host.
With the saved local diagnostic scripts available, regenerate the dynamic counts:

```sh
python3 target/profiling/build-diagnostic.py
cargo build --release --locked --offline \
  --manifest-path target/profiling/diagnostic/Cargo.toml
target/profiling/diagnostic/target/release/riscv-tci-vm --stats \
  tests/fixtures/k256_ecdsa_ckbvm 100 2> target/profiling/diagnostic.tsv
python3 target/profiling/summarize-diagnostic.py
```

### P1 — Reduce costs paid throughout the execution loop

- [x] **1. Improve the generated dispatch topology.** Completed 2026-11-03.
  `build.rs` raises Clang 19's tail-duplication instruction thresholds and
  predecessor/successor limits to 128. Increasing instruction thresholds alone
  did not produce per-handler dispatch: the default graph limits of 16 also
  constrained the 86-opcode table. The retained configuration has 150 indirect
  jump sites versus 5 originally; `tci_run` grows from 2,750 to 8,781 bytes.
  Disassembly confirms separate dispatch tails for arithmetic and memory
  handlers, including `Add`. No interpreter semantic changes were needed.
  Placement-only, aggressive duplication without raised graph limits, an `Add`
  fast path, and reduced compiler settings were also built, benchmarked, and
  inspected; their results are in [README.md](README.md#performance).

  Fresh README-procedure baseline: **2.47 s wall / 2,468.087 ms execution**.
  Final retained rebuild: **2.00 s / 2,002.720 ms**, or **1.24× wall / 1.23×
  execution speedup**. Both retire **384,466,918** instructions, translate
  **1,320** blocks, and exit 0. The first retained-configuration measurement was
  1.93 s / 1,928.545 ms (1.28× execution speedup); separate three-round sequential
  measurements gave median execution of 2,470.002 ms baseline versus 1,986.822 ms
  retained (**1.24×**). All speedups use unprofiled execution. The comparator was
  `ckb-debugger 1.1.1` (2.05 s baseline comparison, 1.98 s final comparison).

  Separate hardware counters show indirect-jump misprediction rates declining
  from **35.46% to 27.60%** and host instructions from **10.054 billion to
  9.431 billion**. All counters ran for 100% of enabled time. Artifacts are in
  ignored `target/dispatch-topology/`; README.md preserves results and assembly
  inspection commands. Larger host code and LLVM-version-dependent layout remain
  tradeoffs; remeasure on compiler/opcode changes and other CPUs/workloads.
  Validation passed: `cargo test --release --locked`, Clippy with warnings denied,
  `scripts/check-isa.sh` against QEMU, and the README UBSan test command.

- [x] **2. Specialize operand fetch and common opcode forms.** Split frequently
  used immediate and register forms and move operand fetch into handlers that
  need it. Loads require a base plus their encoded displacement; stores can
  fetch their value once; moves and control operations need fewer inputs than
  generic binary operations. Specialize constant shift counts, x0 inputs, and
  discarded pure results where worthwhile. This targets the generic immediate
  branch and unnecessary register reads within the 44.95% region. Changes span
  `src/tci.c`, `src/ops.def`, `src/ir.rs`, and `src/decode.rs`. Watch code/IR growth;
  discarded loads must still check permissions and fault, and all guest
  instructions must still count toward the budget.

- [x] **3. Amortize instruction-budget bookkeeping over blocks.** Add block
  instruction counts and use a fast path when the remaining budget covers the
  block, with the existing precise path for a partial block. Remove repeated
  `Link` tests where the IR layout makes them unnecessary. Explore deriving
  retired counts from block-entry count plus instruction index instead of an
  increment after every operation. Faults, traps, syscalls, branches, and budget
  exits must materialize the exact count and guest PC; never count an entire
  block before its potentially faulting operations execute. Preserve resumability
  and saturating limits. Check budgets of zero, one, and every boundary within a
  block, especially blocks ending in synthetic `Link` operations.

- [x] **4. Add cheaper safe memory-access paths.** Completed 2026-11-03.
  `accessible()` skips the second permission lookup for naturally aligned
  1/2/4/8-byte accesses, which cannot cross a 4 KiB page. A likely-aligned hint
  keeps the common path falling through in the generated handlers. Bounds
  checks still use subtraction; unaligned accesses check both endpoints before
  the existing `memcpy`. No partially committed faulting store is allowed.
  Same-page guards, alignment without a hint, and separate last-read/write-page
  caches were measured but not retained. Stack special cases and cross-access
  range hoisting remain possible future experiments, without current evidence
  justifying their complexity.

  Fresh baseline: **2.06 s wall / 2,061.165 ms execution**. Final retained build:
  **1.94 s / 1,944.300 ms**, or **1.06× wall and execution speedup**. Both exit 0
  with **384,466,918 instructions** and **1,320 blocks**. Five additional rounds
  rotating variant order gave median execution of **1,973.133 ms baseline /
  1,898.504 ms retained (1.04×)**; ranges overlap, so the gain is modest and
  host/workload dependent. [README.md](README.md#safe-memory-access-paths-todo-item-4)
  records every experiment and repeat comparison. Release tests, Clippy, the
  QEMU ISA check, and UBSan passed, including new width/boundary/permission,
  precise-fault, partial-store, and budget/resume regression coverage.

- [x] **5. Introduce a small measured set of superinstructions.** Completed
  2026-11-03. Translation selects non-overlapping `Add → Ltu`, `Mulhu → Mul`,
  `Ld → Ld`, and `Sd → Sd` pairs. Both 24-byte records remain in the cache;
  only the first opcode changes. The second PC, operands, and original opcode
  remain available for precise faults and ordinary execution when the budget
  stops inside a pair. Register writeback precedes dependent reads. Memory
  accesses remain ordered and independently checked, including discarded loads;
  no permission/range check is hoisted across an access. Multiplication shares
  one native unsigned multiply only for matching (possibly swapped) inputs when
  the high-half destination does not clobber a low-half input. Disassembly
  confirms both halves come from one `mul`. `tci_run` grows from **8,812 to
  9,240 bytes (+4.9%)**; bytecode size and the cache bound are unchanged.

  Experiments used the README Performance procedure: release build, then one
  run of each runner in sequence with argument `100`, without a warmup. Each
  row adds to the previous implementation. Times are unprofiled; speedups are
  previous wall/execution time divided by the new time.

  | Implementation | VM wall / user / system | VM execution | Incremental wall / execution speedup | ckb-debugger wall / user / system |
  | --- | --- | ---: | ---: | --- |
  | Fresh baseline | 1.92 / 1.92 / 0.00 s | 1,925.482 ms | — | 2.01 / 2.01 / 0.00 s |
  | Add → Ltu | 1.89 / 1.88 / 0.00 s | 1,892.584 ms | 1.02× / 1.02× | 2.09 / 2.08 / 0.00 s |
  | Add guarded Mulhu → Mul | 1.82 / 1.82 / 0.00 s | 1,821.111 ms | 1.04× / 1.04× | 2.16 / 2.16 / 0.00 s |
  | Add paired Ld and Sd | 1.95 / 1.94 / 0.00 s | 1,949.648 ms | 0.93× / 0.93× | 2.06 / 2.05 / 0.00 s |

  The first all-four run regressed, so five additional sequential rounds rotated
  the order of saved binaries, without warmups. Median execution (range) was
  **2,007.888 ms (1,886.103–2,020.699)** baseline,
  **1,943.470 ms (1,858.929–2,003.811)** Add/Ltu,
  **1,959.886 ms (1,873.036–1,970.340)** both arithmetic pairs, and
  **1,889.708 ms (1,832.759–1,929.952)** all four. The retained set gives a
  **1.06× median speedup** over baseline. Individual contributions are noisy,
  ranges overlap, and these results do not establish a portable improvement.
  All runs exit **0**, retire **384,466,918** instructions, and translate
  **1,320** blocks. Comparator runs exit **0** with **725,894,181 CKB cycles**.

  A separate instrumented source copy counted **17,026,200 Add/Ltu**,
  **4,759,600 Mulhu/Mul**, **14,875,014 Ld/Ld**, and **13,191,916 Sd/Sd**
  executions: **49,852,730 fewer dispatches**, equivalent to **12.97%** of
  retired guest instructions. These are actual non-overlapping compatible
  fusions, unlike the original adjacent-pair counts. Instrumented timings are
  excluded from comparisons. Local binaries, repeat results, diagnostic source,
  and disassembly are in ignored `target/superinstructions/`.

  Final unchanged release build, measured after validation using the same
  README procedure: **1.98 s wall / 1.97 s user / 0.00 s system**, with
  **1,978.806 ms execution**, **0.263 ms loading**, and **194.29 MIPS**.
  This is **0.97× wall and execution speedup** versus the fresh baseline
  (a single-run regression); the repeated-run median above is the evidence for
  retention. The final comparator took **1.97 / 1.97 / 0.00 s**. README's table
  reports this final run, without replacing it with a faster sample. Environment:
  the development Intel Xeon E5-2670 v2 host, Debian Clang **19.1.7**, Rust
  **1.95.0**, and ckb-debugger **1.1.1**.

  Validation passed: `cargo test --release --locked`,
  `cargo clippy --all-targets --locked -- -D warnings`,
  `sh scripts/check-isa.sh` against QEMU, and
  `TCI_SANITIZE=1 CARGO_TARGET_DIR=target/ubsan cargo test --locked`.
  New differential tests compare fused/unfused register, memory, fault, PC and
  retirement state at every internal budget boundary, including aliasing,
  discarded results, saturating limits, cross-page permissions and overlapping
  stores. End-to-end tests cover repeated resumes, compressed instruction PCs,
  pairs at the block cap, and branch entry into a previously fused pair's second
  instruction. README changes are limited to the requested performance table.

  Broader constant/copy propagation, move/dead-result elimination, Add/Add and
  Add/branch fusion remain deferred to separately measured experiments. This
  implementation keeps intermediate architectural state and all guest retirement
  points rather than extending into a general local optimizer.

### P2 — Reduce block-transition and data-layout overhead

- [ ] **6. Cache validated successor pointers.** For direct jumps, conditional
  outcomes, and synthetic links, keep host bytecode successor pointers after
  successful translation/validation. Use a miss path to resolve them. Consider
  a small target-checked inline cache for indirect branches and returns, rather
  than assuming the last target is still correct. This targets the 7.80% control
  region and redundant immutable-code checks. Preserve fetch-fault and
  budget-before-fetch ordering. Invalidate every pointer on translation-cache
  flush; no pointer may outlive its owning block. Patch bytecode metadata only,
  never host executable code.

- [ ] **7. Compact and reorganize the 24-byte IR.** Use the measured 443,064-byte
  IR footprint and low cache/TLB miss counts to assess likely benefit. Try a
  compact common record, opcode-specific
  operands, smaller immediates with an extension form, or cold side tables for
  PCs/rare auxiliary fields. Preserve constant-time access to precise PCs on
  faults and budget exits. Consider precomputed register offsets or threaded
  handler addresses as alternatives, recognizing that pointers can increase
  record size. The tradeoff is fewer bytes fetched versus more decoding/loads;
  no data-cache bottleneck is established by the current capture. Maintain
  checked Rust/C layouts and bounded translation storage.

- [ ] **8. Keep selected guest values in host registers across operations.**
  Explore a small register cache, local temporary forwarding within fused
  sequences, or block-level register assignment. The current interpreter keeps
  the register-file pointer local, but guest values are repeatedly loaded and
  written back. Limit register pressure so memory, permissions, bytecode, and
  dispatch state do not spill. All exits must reconstruct exact guest state;
  source/destination aliasing, x0, and `JALR rd == rs1` require explicit coverage.
  Prefer limited forwarding before a general allocator or an assembly loop.

- [ ] **9. Use longer hot paths selectively.** Use the measured mean of 11.18 guest
  instructions per block and 9.62% synthetic-link exit share when evaluating the
  64-instruction cap. Try a larger cap where it removes links, or interpreter superblocks/traces
  with guarded side exits where branches dominate. Raising the cap alone cannot
  remove blocks terminated by real branches. Avoid excessive duplicated IR,
  translation latency, or cache flushes; preserve independently addressable guest
  entry points and precise fault/budget behavior. This overlaps with successor
  caching and superinstructions and should be evaluated after them.

- [ ] **10. Evaluate profile-guided compilation and hot/cold layout.** Train C
  interpreter PGO on representative workloads, verify on held-out workloads, and
  compare handler placement, branch layout, alignment, and cold fault/helper
  outlining. Test code-size-oriented alternatives to `-O3` where useful. Rust's
  current ThinLTO does not automatically optimize the separately compiled native
  `tci.o`; C/Rust cross-language LTO is a separate build experiment with likely
  limited benefit if time remains inside C. Benchmark explicit host tuning
  separately from portable builds. Do not assume `-march=native` is portable or
  sufficient to fix dispatch, and do not assume this Ivy Bridge host has AVX2,
  BMI2, or ADX. PGO and dispatch-tail experiments may interact.

### P3 — Conditional opportunities with little current evidence of payoff

- [ ] **11. Improve translation allocation and reuse.** If shorter runs or a
  larger corpus show translation cost, use bounded arenas/slabs for block IR,
  reduce `Vec`/boxed-slice allocation churn, avoid duplicate overlapping decoded
  blocks where practical, or reuse decoded instruction metadata. Consider
  immutable-ELF translation reuse across VM instances only with correct code,
  permission, and lifetime keys. Hardware sampling assigns only 0.04% to all work
  outside `tci_run`, and the diagnostic run confirms just 1,320 misses, so
  optimizing decoding first is unlikely to change this benchmark materially.
  Preserve lazy error timing and stable C-visible addresses.

- [ ] **12. Revisit dense lookup storage and eviction only with footprint data.**
  The table occupies 16 MiB of virtual address space; that alone does not prove
  16 MiB is resident or hot. Compare executable-range tables, two-level page
  tables, or compact block IDs against today's single lookup. These alternatives
  add indirections. Measure page faults, resident memory, and TLB misses. If the
  one-million-operation cap causes churn on other workloads, evaluate selective
  eviction or generations instead of full flushes, including successor-pointer
  invalidation. This run cached 18,461 IR records and performed zero flushes, so
  eviction is not a bottleneck here.

- [ ] **13. Reduce Rust/C exit and state-copy costs if miss/syscall rates rise.**
  `Vm::run` constructs a state and copies 33 registers into and out of it per C
  invocation. Consider persistent shared state or narrower synchronization at
  exits, while maintaining a clear ABI and public register visibility. Cached
  branches already stay in C, so these copies are not paid per block execution.
  There is little evidence for prioritizing this on the current workload.

- [ ] **14. Accelerate expensive rare ISA operations when profiles justify it.**
  Candidates include CPUID-guarded PCLMULQDQ for `Clmul`/`Clmulh`/`Clmulr`, hardware
  POPCNT, efficient `Orcb`, and width-specific signed division/remainder paths.
  Keep portable fallbacks and all divide-by-zero/overflow semantics. The host
  advertises PCLMULQDQ and POPCNT, but the diagnostic run executed none of the
  operations listed in this item. Avoid optimizing operations merely because their
  source implementations look expensive. Ordinary multiplication already maps
  to hardware instructions.

- [ ] **15. Optimize startup and host memory initialization for short jobs.**
  If single-iteration latency matters, measure ELF parsing, image/stack copying,
  RAM/table initialization, and teardown separately. Reuse immutable loaded
  images or lazily provision metadata where safe. Loading was only 1.038 ms
  versus 2,444.145 ms execution here; it is a low-priority throughput target.
  Do not trade correct zeroed memory or permission isolation for startup savings.

- [ ] **16. Test prefetching, alignment, and host-placement changes only against
  evidence.** If future profiles show cache pressure, selectively test bytecode
  or successor prefetching, hot-handler alignment, allocation locality, and
  suitable CPU placement. Huge pages or NUMA-specific allocation are possible
  deployment experiments only if TLB/locality measurements justify them. They
  can increase overhead or reduce portability. Keep diagnostic affinity changes
  separate from the standard README baseline and apply comparison conditions
  consistently to both runners.

## Constraints and acceptance criteria for future work

- Keep the TCI-style interpreter, 4 MiB guest RAM, immutable executable mappings,
  W^X, and bounded translation cache. JIT/native guest execution, bypassing guest
  crypto with a host implementation, removing required checks, or changing the
  workload are outside this backlog. Parallelizing a single architectural hart
  is not an established optimization for this workload.
- Preserve exact retirement counts, x0 behavior, fault PCs/addresses, partial
  execution state, unaligned/cross-page memory semantics, ISA corner cases,
  syscall behavior, and budget/resume behavior. Fast paths must have valid guards
  and precise fallback paths; in particular, no partially committed faulting
  store or stale cached block pointer is acceptable.
- Implement one independently measurable experiment at a time. Before optimizing,
  record a fresh baseline using README.md's Performance commands. After **each**
  optimization, build and run the same benchmark and report before/after wall,
  execution time, retired count, and speedup (`before / after`). Keep profiler
  timings separate. Additional repeated measurements can establish whether a
  small gain exceeds noise, but label them separately from the required
  single-run comparison. Reprofile when a change shifts the bottleneck.
- Run `cargo test --release --locked`,
  `cargo clippy --all-targets --locked -- -D warnings`, and the README ISA/UBSan
  checks as applicable to the changed paths. Add focused regression coverage for
  new fused, cached, or budget-fast-path behavior, including every internal stop
  boundary. Confirm this benchmark still retires **384,466,918** instructions
  and exits 0; changed block counts may be legitimate.
- Finally update README.md with measured results, environment/version details,
  and any architecture changes for retained optimizations. README.md is unchanged
  by this analysis because no optimization has been implemented.
