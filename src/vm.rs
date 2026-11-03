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
mod tests {
    use super::*;
    use crate::memory::{EXEC, PAGE_COUNT, PAGE_SIZE, READ, WRITE};

    // Execute trusted test bytecode directly so the unfused interpreter is an
    // independent oracle, including its precise exit state and memory effects.
    fn execute_pair(
        code: &[Insn],
        regs: [u64; 33],
        start: u64,
        budget: u64,
        memory: &mut Memory,
    ) -> (i32, [u64; 33], u64, u64, u64) {
        let mut blocks = vec![ptr::null(); code.last().unwrap().pc as usize / 2 + 1];
        for (i, insn) in code.iter().enumerate() {
            blocks[insn.pc as usize / 2] = code[i..].as_ptr();
        }
        let mut state = State {
            regs,
            pc: code[0].pc as u64,
            steps: start,
            limit: start.saturating_add(budget),
            fault: 0,
            memory: memory.bytes.as_mut_ptr(),
            permissions: memory.permissions.as_ptr(),
            blocks: blocks.as_ptr(),
        };
        // SAFETY: Live allocations, checked ABI, valid operands and a terminal
        // Break instruction; the block table covers every reachable test PC.
        let status = unsafe { tci_run(&mut state) };
        (status, state.regs, state.pc, state.steps, state.fault)
    }

    #[test]
    fn arithmetic_superinstructions_match_unfused_at_every_budget_boundary() {
        let mut memory = Memory {
            bytes: vec![0; MEMORY_SIZE].into_boxed_slice(),
            permissions: Box::new([READ | EXEC; PAGE_COUNT]),
        };
        for (x, y) in [
            (0, 0),
            (0, u64::MAX),
            (1, u64::MAX),
            (u64::MAX, u64::MAX),
            (1 << 63, 3),
            (0x123456789abcdef0, 0xfedcba9876543210),
        ] {
            for rd in 0..4 {
                for second_rd in 0..4 {
                    for rs1 in 0..4 {
                        for rs2 in 0..4 {
                            // Exercise both immediate forms of ADD/SLTU and
                            // both operand orders and rejected MUL aliases.
                            for mode in 0..5 {
                                let (first, second) = if mode == 4 {
                                    (Opcode::Mulhu, Opcode::Mul)
                                } else {
                                    (Opcode::Add, Opcode::Ltu)
                                };
                                let mut code = [
                                    Insn::new(first, 0x1000, 2, rd, 1, 2),
                                    Insn::new(second, 0x1002, 4, second_rd, rs1, rs2),
                                    Insn::new(Opcode::Break, 0x1006, 4, 0, 0, 0),
                                ];
                                if mode < 4 {
                                    if mode & 1 != 0 {
                                        code[0] = code[0].imm(y);
                                    }
                                    if mode & 2 != 0 {
                                        code[1] = code[1].imm(y);
                                    }
                                }
                                let mut fused = code;
                                crate::ir::fuse(&mut fused);
                                let mut regs = [0; 33];
                                regs[1] = x;
                                regs[2] = y;
                                regs[3] = 17;
                                for budget in 0..=3 {
                                    let plain = execute_pair(&code, regs, 7, budget, &mut memory);
                                    let combined =
                                        execute_pair(&fused, regs, 7, budget, &mut memory);
                                    assert_eq!(combined, plain, "{code:?}, budget={budget}");
                                }
                            }
                        }
                    }
                }
            }
        }
        // A saturated limit still allows just the first operation to retire.
        let mut code = [
            Insn::new(Opcode::Add, 0x1000, 4, 1, 0, 0).imm(42),
            Insn::new(Opcode::Ltu, 0x1004, 4, 2, 0, 1),
            Insn::new(Opcode::Break, 0x1008, 4, 0, 0, 0),
        ];
        crate::ir::fuse(&mut code);
        let result = execute_pair(&code, [0; 33], u64::MAX - 1, 10, &mut memory);
        assert_eq!(result.0, 2);
        assert_eq!((result.1[1], result.1[2]), (42, 0));
        assert_eq!((result.2, result.3), (0x1004, u64::MAX));
    }

    #[test]
    fn memory_superinstructions_preserve_order_faults_and_budget_stops() {
        const PAGE: usize = 0x20000;
        let mut memory = Memory {
            bytes: vec![0x81; MEMORY_SIZE].into_boxed_slice(),
            permissions: Box::new([READ | WRITE; PAGE_COUNT]),
        };
        memory.permissions[0] = 0;
        memory.permissions[1] = READ | EXEC;
        let addresses = [
            0,
            0x1000,
            PAGE as u64,
            PAGE as u64 + 1,
            (PAGE + PAGE_SIZE - 7) as u64,
            (PAGE + PAGE_SIZE - 8) as u64,
            (PAGE + PAGE_SIZE) as u64,
            (MEMORY_SIZE - 7) as u64,
            u64::MAX - 3,
        ];
        for op in [Opcode::Ld, Opcode::Sd] {
            for permission in [0, READ, WRITE, READ | WRITE, EXEC] {
                memory.permissions[PAGE / PAGE_SIZE + 1] = permission;
                for &first_address in &addresses {
                    for &second_address in &addresses {
                        // rd=1 feeds the loaded value into the second base;
                        // rd=0 verifies that discarded loads still fault.
                        for rd in [0, 1, 3] {
                            let code = [
                                Insn::new(op, 0x1000, 2, rd, 1, 3),
                                Insn::new(op, 0x1002, 4, 0, 1, 2)
                                    .imm(second_address.wrapping_sub(first_address)),
                                Insn::new(Opcode::Break, 0x1006, 4, 0, 0, 0),
                            ];
                            let mut fused = code;
                            crate::ir::fuse(&mut fused);
                            let mut regs = [0; 33];
                            regs[1] = first_address;
                            regs[2] = 0xfedcba9876543210;
                            regs[3] = 0x123456789abcdef0;
                            for budget in 0..=3 {
                                let plain = execute_pair(&code, regs, 11, budget, &mut memory);
                                let expected = memory.bytes.clone();
                                memory.bytes.fill(0x81);
                                let combined = execute_pair(&fused, regs, 11, budget, &mut memory);
                                assert_eq!(combined, plain, "{code:?}, budget={budget}");
                                // Whole-memory comparison detects partial stores
                                // and accidentally reordered overlapping writes.
                                assert_eq!(memory.bytes, expected);
                                memory.bytes.fill(0x81);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn memory_paths_preserve_bounds_permissions_and_precise_state() {
        const PC: usize = 0x1000;
        const PAGE: usize = 0x20000;
        const PRIOR_STORE: usize = 0x30000;
        const VALUE: u64 = 0x8877_6655_4433_2255;
        const UNCHANGED: u64 = 0xdead_beef;
        let mut memory = Memory {
            bytes: vec![0x81; MEMORY_SIZE].into_boxed_slice(),
            permissions: Box::new([READ | WRITE; PAGE_COUNT]),
        };
        memory.permissions[0] = 0;
        memory.permissions[PC / PAGE_SIZE] = READ | EXEC;
        let mut expected = memory.bytes.clone();
        expected[PRIOR_STORE] = VALUE as u8;
        let mut blocks = vec![ptr::null(); PC / 2 + 1];

        // Cover every alignment near a boundary, permissions denied at either
        // endpoint, the last RAM bytes, and addresses that could wrap/truncate.
        let mut cases = Vec::new();
        for (first, second) in [
            (READ | WRITE, READ | WRITE),
            (READ, WRITE),
            (WRITE, READ),
            (0, READ | WRITE),
            (READ | WRITE, 0),
            (EXEC, READ | WRITE),
            (READ | WRITE, EXEC),
        ] {
            for address in PAGE + PAGE_SIZE - 8..=PAGE + PAGE_SIZE + 8 {
                cases.push((address as u64, first, second));
            }
        }
        for address in (MEMORY_SIZE - 8..=MEMORY_SIZE + 1).chain([
            0,
            1,
            PAGE_SIZE - 1,
            PAGE_SIZE,
            PAGE_SIZE + 1,
            PAGE + 1,
        ]) {
            cases.push((address as u64, READ | WRITE, READ | WRITE));
        }
        for address in [u64::MAX - 7, u64::MAX, (1u64 << 32) + PAGE as u64] {
            cases.push((address, READ | WRITE, READ | WRITE));
        }

        for (op, size, permission, signed) in [
            (Opcode::Lb, 1, READ, true),
            (Opcode::Lh, 2, READ, true),
            (Opcode::Lw, 4, READ, true),
            (Opcode::Ld, 8, READ, false),
            (Opcode::Lbu, 1, READ, false),
            (Opcode::Lhu, 2, READ, false),
            (Opcode::Lwu, 4, READ, false),
            (Opcode::Sb, 1, WRITE, false),
            (Opcode::Sh, 2, WRITE, false),
            (Opcode::Sw, 4, WRITE, false),
            (Opcode::Sd, 8, WRITE, false),
        ] {
            let code = [
                Insn::new(Opcode::Mov, PC as u32, 4, 8, 0, 0).imm(42),
                Insn::new(Opcode::Sb, PC as u32 + 4, 4, 0, 9, 2),
                Insn::new(op, PC as u32 + 8, 4, 3, 1, 2),
                Insn::new(Opcode::Break, PC as u32 + 12, 4, 0, 0, 0),
            ];
            blocks[PC / 2] = code.as_ptr();
            // Resuming at the access must behave identically to reaching it
            // in one call; a budget stop must precede even an invalid access.
            blocks.resize((PC + 8) / 2 + 1, ptr::null());
            blocks[(PC + 8) / 2] = code[2..].as_ptr();
            for &(address, first, second) in &cases {
                memory.permissions[PAGE / PAGE_SIZE] = first;
                memory.permissions[PAGE / PAGE_SIZE + 1] = second;
                let allowed = memory.accessible(address, size, permission);
                for stop_before_access in [false, true] {
                    let mut state = State {
                        regs: [0; 33],
                        pc: PC as u64,
                        steps: 0,
                        limit: if stop_before_access { 2 } else { 3 },
                        fault: 0,
                        memory: memory.bytes.as_mut_ptr(),
                        permissions: memory.permissions.as_ptr(),
                        blocks: blocks.as_ptr(),
                    };
                    state.regs[1] = address;
                    state.regs[2] = VALUE;
                    state.regs[3] = UNCHANGED;
                    state.regs[9] = PRIOR_STORE as u64;
                    // SAFETY: The checked ABI, live buffers, valid register
                    // indices and terminating instruction match Vm::run.
                    let mut status = unsafe { tci_run(&mut state) };
                    if stop_before_access {
                        assert_eq!(status, 2);
                        assert_eq!(state.steps, 2);
                        assert_eq!(state.pc, PC as u64 + 8);
                        assert_eq!(state.regs[3], UNCHANGED);
                        assert_eq!(memory.bytes, expected);
                        state.limit = 3;
                        // SAFETY: Same live buffers and valid resume block.
                        status = unsafe { tci_run(&mut state) };
                    }
                    let context = (op, address, first, second, stop_before_access);
                    assert_eq!(state.regs[8], 42, "{context:?}");
                    if allowed {
                        assert_eq!(status, 2, "{context:?}");
                        assert_eq!(state.steps, 3, "{context:?}");
                        assert_eq!(state.pc, PC as u64 + 12, "{context:?}");
                        if permission == WRITE {
                            expected[address as usize..address as usize + size]
                                .copy_from_slice(&VALUE.to_le_bytes()[..size]);
                            assert_eq!(state.regs[3], UNCHANGED, "{context:?}");
                        } else {
                            let shift = 64 - size * 8;
                            let raw = u64::from_le_bytes([0x81; 8]);
                            let loaded = if signed {
                                ((raw << shift) as i64 >> shift) as u64
                            } else {
                                raw << shift >> shift
                            };
                            assert_eq!(state.regs[3], loaded, "{context:?}");
                        }
                    } else {
                        assert_eq!(
                            status,
                            if permission == READ { 3 } else { 4 },
                            "{context:?}"
                        );
                        assert_eq!(state.steps, 2, "{context:?}");
                        assert_eq!(state.pc, PC as u64 + 8, "{context:?}");
                        assert_eq!(state.fault, address, "{context:?}");
                        assert_eq!(state.regs[3], UNCHANGED, "{context:?}");
                    }
                    // Comparing all RAM catches partial faulting stores as
                    // well as writes outside a successful store's width.
                    assert_eq!(memory.bytes, expected, "{context:?}");
                    if allowed && permission == WRITE {
                        expected[address as usize..address as usize + size].fill(0x81);
                        memory.bytes[address as usize..address as usize + size].fill(0x81);
                    }
                    memory.bytes[PRIOR_STORE] = 0x81;
                }
            }
        }
    }
}
