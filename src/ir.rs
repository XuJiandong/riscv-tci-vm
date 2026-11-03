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
    pub padding: u16,
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
            padding: 0,
            aux: 0,
            imm: 0,
        }
    }
    pub fn imm(mut self, value: u64) -> Self {
        self.imm = value;
        self.immediate = 1;
        self
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
            code[i].op = op;
            i += 2;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiply_fusion_requires_unchanged_matching_inputs() {
        for high_rd in 0..5 {
            for low_a in 0..5 {
                for low_b in 0..5 {
                    for immediate in 0..3 {
                        let mut pair = [
                            Insn::new(Opcode::Mulhu, 0x1000, 4, high_rd, 1, 2),
                            Insn::new(Opcode::Mul, 0x1004, 4, 1, low_a, low_b),
                        ];
                        if immediate != 0 {
                            pair[immediate - 1].immediate = 1;
                        }
                        fuse(&mut pair);
                        let expected = immediate == 0
                            && high_rd != 1
                            && high_rd != 2
                            && matches!((low_a, low_b), (1, 2) | (2, 1));
                        assert_eq!(pair[0].op == Opcode::MulhuMul, expected);
                        assert_eq!(pair[1].op, Opcode::Mul);
                    }
                }
            }
        }
    }

    #[test]
    fn fusion_keeps_records_and_does_not_overlap_or_cross_terminators() {
        let mut code = [
            Insn::new(Opcode::Ld, 0x1000, 2, 1, 2, 0).imm(8),
            Insn::new(Opcode::Ld, 0x1002, 4, 2, 1, 0).imm(16),
            Insn::new(Opcode::Ld, 0x1006, 2, 3, 2, 0),
            Insn::new(Opcode::Link, 0x1008, 0, 0, 0, 0),
            Insn::new(Opcode::Ld, 0x1008, 4, 4, 2, 0),
        ];
        let original = code;
        fuse(&mut code);
        assert_eq!(code[0].op, Opcode::LdLd);
        code[0].op = Opcode::Ld;
        for (actual, expected) in code.iter().zip(original) {
            // All metadata, including each original PC and length, survives.
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
        fuse(&mut []);
        fuse(&mut code[..1]);
    }
}
