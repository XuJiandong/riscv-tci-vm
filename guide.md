Your task is to implement a RISC-V VM that runs on the x86_64 architecture. The RISC-V target is rv64imcb.
The performance is quite important. We want a very fast risc-v interpreter.

## Code
Use QEMU's TCI in TCG as the main architecture; see the code at /home/xjd/projects/qemu.
We can't use JIT or other similar technologies, for security reason.

The main and entry parts should be written in Rust. Where performance matters, using C or assembly is acceptable.
Use [goblin](https://crates.io/crates/goblin) to parse ELF files.

The total guest memory on VM should be 4M bytes, 4K per page. The memory must support W ^ X.

Use rust 1.95 and clang 19: they are already installed.

Don't cheat: don't refer any ckb-vm code.

## Tests
The interpreter must successfully run the bundled program at `tests/fixtures/k256_ecdsa_ckbvm` with guest argument `1`. Run `cargo test --release --locked --test k256` to verify it; this test also runs in the default test suite. Set `K256_ELF` to test an alternate ELF.


## Syscalls
In tests above, it might contains syscalls. defined as below:

```C
static inline long
__internal_syscall(long n, long _a0, long _a1, long _a2, long _a3, long _a4, long _a5)
{
  register long a0 asm("a0") = _a0;
  register long a1 asm("a1") = _a1;
  register long a2 asm("a2") = _a2;
  register long a3 asm("a3") = _a3;
  register long a4 asm("a4") = _a4;
  register long a5 asm("a5") = _a5;

  register long syscall_id asm("a7") = n;

  asm volatile ("scall"
		: "+r"(a0) : "r"(a1), "r"(a2), "r"(a3), "r"(a4), "r"(a5), "r"(syscall_id));

  return a0;
}

#define syscall(n, a, b, c, d, e, f) \
        __internal_syscall(n, (long)(a), (long)(b), (long)(c), (long)(d), (long)(e), (long)(f))


void exit(int8_t code)
{
  syscall(93, code, 0, 0, 0, 0, 0);
}
```

Exit syscall don't need a return value since it is not supposed to return from this function. Upon receiving this syscall, VM would terminate execution with the specified return code. This is the only way of correctly exiting.


```C
void debug(const char* s)
{
  syscall(2177, s, 0, 0, 0, 0, 0);
}
```
This syscall accepts a null terminated string and prints it out as debug log. It can be used as a handy way to debug. This syscall has no return value.

