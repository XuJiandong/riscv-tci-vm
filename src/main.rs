use riscv_tci_vm::Vm;
use std::{env, fs, io, process::ExitCode, time::Instant};

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut limit = 1_000_000_000u64;
    let mut stats = false;
    let path = loop {
        match args.next().as_deref() {
            Some("--max-instructions") => {
                limit = args
                    .next()
                    .ok_or("--max-instructions needs a number")?
                    .parse()?;
            }
            Some("--stats") => stats = true,
            Some("--") => break args.next().ok_or("missing ELF path")?,
            Some("--help" | "-h") | None => {
                println!(
                    "Usage: riscv-tci-vm [--stats] [--max-instructions N] ELF [ARG ...]\n\
                    RV64IMCB + Zbc; 4 MiB RAM; 4 KiB W^X pages; no JIT.\n\
                    Syscalls: exit (93), debug (2177). Default budget: 1 billion instructions."
                );
                return Ok(0);
            }
            Some(option) if option.starts_with('-') => {
                return Err(format!("unknown option: {option}").into());
            }
            Some(path) => break path.to_owned(),
        }
    };
    // Contract-style programs receive exactly the supplied argument vector;
    // do not inject the host ELF path as argv[0].
    let guest_args: Vec<String> = args.collect();
    let start = Instant::now();
    let mut vm = Vm::from_elf(&fs::read(path)?, &guest_args)?;
    let loaded = Instant::now();
    let result = vm.run(limit, &mut io::stderr().lock())?;
    if stats {
        let elapsed = loaded.elapsed().as_secs_f64();
        eprintln!(
            "exit={} instructions={} blocks={} load={:.3}ms run={:.3}ms rate={:.2} MIPS",
            result.exit_code,
            result.instructions,
            result.translated_blocks,
            loaded.duration_since(start).as_secs_f64() * 1000.0,
            elapsed * 1000.0,
            result.instructions as f64 / elapsed / 1_000_000.0
        );
    }
    Ok(result.exit_code as u8)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("riscv-tci-vm: {error}");
            ExitCode::from(1)
        }
    }
}
