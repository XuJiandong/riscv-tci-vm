include!(concat!(env!("OUT_DIR"), "/opcodes.rs"));

/// TCI register bytecode. All pointers stay in the host's non-executable heap.
/// x0 is read-only; destination x0 is redirected to the private discard slot 32.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct Insn {
    pub op: Opcode,
    pub rd: u8,
    pub rs1: u8,
    pub rs2: u8,
    pub immediate: u8,
    pub len: u8,
    // Opcode plus the immediate bit, using the former padding bytes.
    dispatch_index: u16,
    pub pc: u32,
    pub aux: u32,
    pub imm: u64,
}

impl Insn {
    pub fn new(op: Opcode, pc: u32, len: u8, rd: u8, rs1: u8, rs2: u8) -> Self {
        Self {
            op,
            pc,
            len,
            rd: if rd == 0 { 32 } else { rd },
            rs1,
            rs2,
            immediate: 0,
            dispatch_index: op as u16,
            aux: 0,
            imm: 0,
        }
    }
    pub fn imm(mut self, value: u64) -> Self {
        self.imm = value;
        self.immediate = 1;
        self.dispatch_index |= 128;
        self
    }
    fn set_opcode(&mut self, op: Opcode) {
        self.op = op;
        self.dispatch_index = op as u16 | u16::from(self.immediate) << 7;
    }
    pub fn ends_block(&self) -> bool {
        use Opcode::*;
        matches!(
            self.op,
            Beq | Bne | Blt | Bge | Bltu | Bgeu | Jump | JumpReg | Ecall | Break | Trap
        )
    }
}

const _: () = assert!(size_of::<Insn>() == 24);

/// Keep both records (including the second operation's PC and opcode). Only
/// replace the first opcode, and never form overlapping pairs. The interpreter
/// can execute the original first operation when only one budget slot remains.
pub(crate) fn fuse(code: &mut [Insn]) {
    let mut i = 0;
    while i + 1 < code.len() {
        let (first, second) = (code[i], code[i + 1]);
        let fused = match (first.op, second.op) {
            (Opcode::Add, Opcode::Ltu) => Some(Opcode::AddLtu),
            (Opcode::Ld, Opcode::Ld) => Some(Opcode::LdLd),
            (Opcode::Sd, Opcode::Sd) => Some(Opcode::SdSd),
            // Both halves must use the same original operands. The high-half
            // destination must not overwrite either input of the low multiply.
            (Opcode::Mulhu, Opcode::Mul)
                if first.immediate == 0
                    && second.immediate == 0
                    && first.rd != second.rs1
                    && first.rd != second.rs2
                    && ((first.rs1 == second.rs1 && first.rs2 == second.rs2)
                        || (first.rs1 == second.rs2 && first.rs2 == second.rs1)) =>
            {
                Some(Opcode::MulhuMul)
            }
            _ => None,
        };
        if let Some(op) = fused {
            code[i].set_opcode(op);
            i += 2;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
#[path = "tests/ir.rs"]
mod tests;
