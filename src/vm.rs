use crate::{
    decode::{decode16, decode32},
    ir::{Insn, Opcode},
    memory::{MEMORY_SIZE, Memory},
};
use std::{fmt, io::Write, ptr};

#[derive(Debug)]
pub enum Error {
    Elf(String),
    Memory {
        pc: u64,
        address: u64,
        access: &'static str,
    },
    IllegalInstruction {
        pc: u64,
        instruction: u32,
    },
    Breakpoint {
        pc: u64,
    },
    UnknownSyscall {
        pc: u64,
        number: u64,
    },
    InstructionLimit {
        pc: u64,
        instructions: u64,
    },
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Elf(message) => write!(f, "ELF error: {message}"),
            Self::Memory {
                pc,
                address,
                access,
            } => write!(f, "{access} fault at {address:#x} (pc={pc:#x})"),
            Self::IllegalInstruction { pc, instruction } => {
                write!(f, "illegal instruction {instruction:#010x} at pc={pc:#x}")
            }
            Self::Breakpoint { pc } => write!(f, "breakpoint at pc={pc:#x}"),
            Self::UnknownSyscall { pc, number } => {
                write!(f, "unknown syscall {number} at pc={pc:#x}")
            }
            Self::InstructionLimit { pc, instructions } => write!(
                f,
                "instruction limit reached after {instructions} instructions (pc={pc:#x})"
            ),
            Self::Io(e) => write!(f, "debug output: {e}"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy)]
pub struct RunResult {
    pub exit_code: i8,
    pub instructions: u64,
    pub translated_blocks: u64,
}

#[repr(C)]
struct State {
    regs: [u64; 33],
    pc: u64,
    steps: u64,
    limit: u64,
    fault: u64,
    memory: *mut u8,
    permissions: *const u8,
    blocks: *const *const Insn,
}
const _: () = assert!(size_of::<State>() == 320);
const _: () = assert!(std::mem::offset_of!(State, memory) == 296);

unsafe extern "C" {
    fn tci_run(state: *mut State) -> i32;
}

/// Single-hart RV64 user-mode machine. Memory mappings are immutable after
/// loading, so translation blocks never outlive the code they were built from.
pub struct Vm {
    memory: Memory,
    regs: [u64; 33],
    pc: u64,
    steps: u64,
    // Dense halfword-PC lookup avoids hashing in the interpreter's hot path.
    lookup: Box<[*const Insn]>,
    blocks: Vec<Box<[Insn]>>,
    cached_ops: usize,
    translated_blocks: u64,
    exit_code: Option<i8>,
}

impl Vm {
    /// `args` includes argv[0]. Guest SP is 16-byte aligned and a0/a1 contain
    /// argc/argv as a convenience in addition to the process-entry stack.
    pub fn from_elf(elf: &[u8], args: &[String]) -> Result<Self, Error> {
        let (mut memory, pc) = Memory::load_elf(elf)?;
        let (sp, argv) = memory.init_stack(args)?;
        let mut regs = [0; 33];
        regs[2] = sp;
        regs[10] = args.len() as u64;
        regs[11] = argv;
        Ok(Self {
            memory,
            regs,
            pc,
            steps: 0,
            lookup: vec![ptr::null(); MEMORY_SIZE / 2].into_boxed_slice(),
            blocks: Vec::new(),
            cached_ops: 0,
            translated_blocks: 0,
            exit_code: None,
        })
    }

    pub fn registers(&self) -> &[u64] {
        &self.regs[..32]
    }
    pub fn pc(&self) -> u64 {
        self.pc
    }
    pub fn instructions(&self) -> u64 {
        self.steps
    }

    /// Execute up to `budget` additional retired instructions. A budget error
    /// leaves the VM resumable. Debug syscall bytes are written to `debug`.
    pub fn run(&mut self, budget: u64, debug: &mut impl Write) -> Result<RunResult, Error> {
        let limit = self.steps.saturating_add(budget);
        loop {
            if let Some(exit_code) = self.exit_code {
                return Ok(RunResult {
                    exit_code,
                    instructions: self.steps,
                    translated_blocks: self.translated_blocks,
                });
            }
            let mut state = State {
                regs: self.regs,
                pc: self.pc,
                steps: self.steps,
                limit,
                fault: 0,
                memory: self.memory.bytes.as_mut_ptr(),
                permissions: self.memory.permissions.as_ptr(),
                blocks: self.lookup.as_ptr(),
            };
            // SAFETY: State has a checked C layout. All allocations remain
            // live and unmoved during this synchronous call. Only the trusted
            // translator creates bytecode; C bounds-checks every guest address.
            // C never retains these pointers or writes to bytecode/permissions.
            let status = unsafe { tci_run(&mut state) };
            self.regs = state.regs;
            self.pc = state.pc;
            self.steps = state.steps;
            match status {
                0 => self.translate(),
                1 => match self.regs[17] {
                    93 => self.exit_code = Some(self.regs[10] as i8),
                    2177 => {
                        let message = self.memory.debug_string(self.regs[10], self.pc)?;
                        debug.write_all(message).map_err(Error::Io)?;
                        self.pc += 4; // ECALL has no compressed encoding.
                    }
                    number => {
                        return Err(Error::UnknownSyscall {
                            pc: self.pc,
                            number,
                        });
                    }
                },
                2 => {
                    return Err(Error::InstructionLimit {
                        pc: self.pc,
                        instructions: self.steps,
                    });
                }
                3..=5 => {
                    return Err(Error::Memory {
                        pc: self.pc,
                        address: state.fault,
                        access: match status {
                            3 => "read",
                            4 => "write",
                            _ => "execute",
                        },
                    });
                }
                6 => {
                    return Err(Error::IllegalInstruction {
                        pc: self.pc,
                        instruction: state.fault as u32,
                    });
                }
                7 => return Err(Error::Breakpoint { pc: self.pc }),
                _ => unreachable!("invalid TCI status {status}"),
            }
        }
    }

    fn translate(&mut self) {
        // Bound host cache memory even for adversarial control flow. Clear
        // only between C invocations, when no interpreter holds a block pointer.
        const MAX_CACHED_OPS: usize = 1_048_576;
        if self.cached_ops >= MAX_CACHED_OPS {
            self.lookup.fill(ptr::null());
            self.blocks.clear();
            self.cached_ops = 0;
        }
        let mut pc = self.pc;
        let mut code = Vec::with_capacity(65);
        for _ in 0..64 {
            let mut fault = Insn::new(Opcode::Trap, pc as u32, 0, 0, 0, 0);
            fault.aux = 5;
            fault.imm = pc;
            let Some(low) = self.memory.fetch16(pc) else {
                code.push(fault);
                break;
            };
            let raw = if low & 3 == 3 {
                let Some(high) = self.memory.fetch16(pc + 2) else {
                    fault.imm = pc + 2;
                    code.push(fault);
                    break;
                };
                ((high as u32) << 16) | low as u32
            } else {
                low as u32
            };
            let decoded = if low & 3 == 3 {
                decode32(raw, pc as u32)
            } else {
                decode16(low, pc as u32)
            };
            let Some(insn) = decoded else {
                fault.aux = 6;
                fault.imm = raw as u64;
                code.push(fault);
                break;
            };
            pc += insn.len as u64;
            code.push(insn);
            if insn.ends_block() {
                break;
            }
        }
        if !code.last().unwrap().ends_block() {
            code.push(Insn::new(Opcode::Link, pc as u32, 0, 0, 0, 0).imm(pc));
        }
        crate::ir::fuse(&mut code);
        let block = code.into_boxed_slice();
        self.cached_ops += block.len();
        self.lookup[self.pc as usize / 2] = block.as_ptr();
        self.blocks.push(block);
        self.translated_blocks += 1;
    }
}

#[cfg(test)]
#[path = "tests/vm.rs"]
mod tests;
