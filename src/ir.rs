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
