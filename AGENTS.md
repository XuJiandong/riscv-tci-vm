

## Performance

Use the steps in the "Performance" section of `README.md` to measure performance. Before optimizing, record a
baseline. After each optimization, build and run the benchmark, then report the before/after numbers and the
resulting speedup. And finally update README.md's following table only:
```
| Runner | Wall time | User CPU time | System CPU time | Guest result |
| --- | ---: | ---: | ---: | ---: |
| riscv-tci-vm (current: dispatch and memory-access optimizations) | 1.94 s | 1.94 s | 0.00 s | 0 |
| ckb-debugger (`--mode fast`) | 2.13 s | 2.12 s | 0.00 s | 0 |
```
