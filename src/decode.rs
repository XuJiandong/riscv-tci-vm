//! RV64IMC + Zba/Zbb/Zbs (B 1.0) and Zbc lowering to register bytecode.
use crate::ir::{Insn, Opcode::*};

fn sext(x: u32, bits: u32) -> u64 {
    ((x << (32 - bits)) as i32 >> (32 - bits)) as i64 as u64
}

pub(crate) fn decode32(w: u32, pc: u32) -> Option<Insn> {
    let rd = ((w >> 7) & 31) as u8;
    let rs1 = ((w >> 15) & 31) as u8;
    let rs2 = ((w >> 20) & 31) as u8;
    let f3 = (w >> 12) & 7;
    let f7 = w >> 25;
    let f6 = w >> 26;
    let i = sext(w >> 20, 12);
    let make = |op| Insn::new(op, pc, 4, rd, rs1, rs2);
    let result = match w & 127 {
        0x37 => make(Mov).imm(sext(w & 0xfffff000, 32)),
        0x17 => make(Mov).imm((pc as u64).wrapping_add(sext(w & 0xfffff000, 32))),
        0x6f => {
            let imm = ((w >> 31) << 20)
                | (((w >> 12) & 255) << 12)
                | (((w >> 20) & 1) << 11)
                | (((w >> 21) & 1023) << 1);
            make(Jump).imm((pc as u64).wrapping_add(sext(imm, 21)))
        }
        0x67 if f3 == 0 => make(JumpReg).imm(i),
        0x63 => {
            let op = match f3 {
                0 => Beq,
                1 => Bne,
                4 => Blt,
                5 => Bge,
                6 => Bltu,
                7 => Bgeu,
                _ => return None,
            };
            let imm = ((w >> 31) << 12)
                | (((w >> 7) & 1) << 11)
                | (((w >> 25) & 63) << 5)
                | (((w >> 8) & 15) << 1);
            let mut insn = make(op);
            insn.imm = (pc as u64).wrapping_add(sext(imm, 13));
            insn
        }
        0x03 => {
            let op = match f3 {
                0 => Lb,
                1 => Lh,
                2 => Lw,
                3 => Ld,
                4 => Lbu,
                5 => Lhu,
                6 => Lwu,
                _ => return None,
            };
            make(op).imm(i)
        }
        0x23 => {
            let op = match f3 {
                0 => Sb,
                1 => Sh,
                2 => Sw,
                3 => Sd,
                _ => return None,
            };
            make(op).imm(sext(((w >> 25) << 5) | ((w >> 7) & 31), 12))
        }
        0x13 => match f3 {
            0 => make(Add).imm(i),
            2 => make(Lt).imm(i),
            3 => make(Ltu).imm(i),
            4 => make(Xor).imm(i),
            6 => make(Or).imm(i),
            7 => make(And).imm(i),
            1 => {
                let op = match w >> 20 {
                    0x600 => Clz,
                    0x601 => Ctz,
                    0x602 => Cpop,
                    0x604 => Sextb,
                    0x605 => Sexth,
                    _ => match f6 {
                        0 => Shl,
                        0x12 => Bclr,
                        0x1a => Binv,
                        0x0a => Bset,
                        _ => return None,
                    },
                };
                make(op).imm(((w >> 20) & 63) as u64)
            }
            5 => {
                let op = match w >> 20 {
                    0x287 => Orcb,
                    0x6b8 => Rev8,
                    _ => match f6 {
                        0 => Shr,
                        0x10 => Sar,
                        0x18 => Ror,
                        0x12 => Bext,
                        _ => return None,
                    },
                };
                make(op).imm(((w >> 20) & 63) as u64)
            }
            _ => return None,
        },
        0x1b => match f3 {
            0 => make(Addw).imm(i),
            1 => {
                let op = match w >> 20 {
                    0x600 => Clzw,
                    0x601 => Ctzw,
                    0x602 => Cpopw,
                    _ if f6 == 0x02 => Shluw,
                    _ if f7 == 0 => Shlw,
                    _ => return None,
                };
                make(op).imm(((w >> 20) & 63) as u64)
            }
            5 => {
                let op = match f7 {
                    0 => Shrw,
                    0x20 => Sarw,
                    0x30 => Rorw,
                    _ => return None,
                };
                make(op).imm(rs2 as u64)
            }
            _ => return None,
        },
        0x33 | 0x3b => {
            let word = w & 127 == 0x3b;
            let op = if word {
                match (f7, f3) {
                    (0, 0) => Addw,
                    (0x20, 0) => Subw,
                    (0, 1) => Shlw,
                    (0, 5) => Shrw,
                    (0x20, 5) => Sarw,
                    (1, 0) => Mulw,
                    (1, 4) => Divw,
                    (1, 5) => Divuw,
                    (1, 6) => Remw,
                    (1, 7) => Remuw,
                    (0x30, 1) => Rolw,
                    (0x30, 5) => Rorw,
                    (4, 4) if rs2 == 0 => Zexth,
                    (4, 0) => Adduw,
                    (0x10, 2 | 4 | 6) => Shadduw,
                    _ => return None,
                }
            } else {
                match (f7, f3) {
                    (0, 0) => Add,
                    (0x20, 0) => Sub,
                    (0, 1) => Shl,
                    (0, 2) => Lt,
                    (0, 3) => Ltu,
                    (0, 4) => Xor,
                    (0, 5) => Shr,
                    (0x20, 5) => Sar,
                    (0, 6) => Or,
                    (0, 7) => And,
                    (1, 0) => Mul,
                    (1, 1) => Mulh,
                    (1, 2) => Mulhsu,
                    (1, 3) => Mulhu,
                    (1, 4) => Div,
                    (1, 5) => Divu,
                    (1, 6) => Rem,
                    (1, 7) => Remu,
                    (0x20, 7) => Andn,
                    (0x20, 6) => Orn,
                    (0x20, 4) => Xnor,
                    (0x30, 1) => Rol,
                    (0x30, 5) => Ror,
                    (5, 4) => Min,
                    (5, 5) => Minu,
                    (5, 6) => Max,
                    (5, 7) => Maxu,
                    (5, 1) => Clmul,
                    (5, 2) => Clmulr,
                    (5, 3) => Clmulh,
                    (0x24, 1) => Bclr,
                    (0x24, 5) => Bext,
                    (0x34, 1) => Binv,
                    (0x14, 1) => Bset,
                    (0x10, 2 | 4 | 6) => Shadd,
                    _ => return None,
                }
            };
            let mut insn = make(op);
            if matches!(op, Shadd | Shadduw) {
                insn.aux = f3 / 2;
            }
            insn
        }
        // FENCE is a no-op in this single-hart interpreter. FENCE.I is also
        // supported: executable pages cannot be modified during execution.
        0x0f if f3 == 0 || f3 == 1 => make(Nop),
        0x73 if w == 0x00000073 => make(Ecall),
        0x73 if w == 0x00100073 => make(Break),
        _ => return None,
    };
    Some(result)
}

pub(crate) fn decode16(raw: u16, pc: u32) -> Option<Insn> {
    let w = raw as u32;
    let rd = ((w >> 7) & 31) as u8;
    let rs2 = ((w >> 2) & 31) as u8;
    let rp1 = 8 + ((w >> 7) & 7) as u8;
    let rp2 = 8 + ((w >> 2) & 7) as u8;
    let ci = sext(((w >> 2) & 31) | (((w >> 12) & 1) << 5), 6);
    let make = |op, d, a, b| Insn::new(op, pc, 2, d, a, b);
    let insn = match (w & 3, w >> 13) {
        (0, 0) => {
            let imm = (((w >> 7) & 15) << 6)
                | (((w >> 11) & 3) << 4)
                | (((w >> 5) & 1) << 3)
                | (((w >> 6) & 1) << 2);
            if imm == 0 {
                return None;
            }
            make(Add, rp2, 2, 0).imm(imm as u64)
        }
        (0, f @ (2 | 3 | 6 | 7)) => {
            let imm = if f & 1 == 0 {
                (((w >> 5) & 1) << 6) | (((w >> 10) & 7) << 3) | (((w >> 6) & 1) << 2)
            } else {
                (((w >> 5) & 3) << 6) | (((w >> 10) & 7) << 3)
            };
            let op = match f {
                2 => Lw,
                3 => Ld,
                6 => Sw,
                _ => Sd,
            };
            make(op, rp2, rp1, rp2).imm(imm as u64)
        }
        (1, 0) => make(Add, rd, rd, 0).imm(ci), // includes C.NOP and HINTs
        (1, 1) if rd != 0 => make(Addw, rd, rd, 0).imm(ci),
        (1, 2) => make(Mov, rd, 0, 0).imm(ci),
        (1, 3) => {
            if ci == 0 {
                return None;
            }
            if rd == 2 {
                let imm = (((w >> 12) & 1) << 9)
                    | (((w >> 3) & 3) << 7)
                    | (((w >> 5) & 1) << 6)
                    | (((w >> 2) & 1) << 5)
                    | (((w >> 6) & 1) << 4);
                make(Add, 2, 2, 0).imm(sext(imm, 10))
            } else {
                make(Mov, rd, 0, 0).imm(ci << 12)
            }
        }
        (1, 4) => match (w >> 10) & 3 {
            0 => make(Shr, rp1, rp1, 0).imm(ci & 63),
            1 => make(Sar, rp1, rp1, 0).imm(ci & 63),
            2 => make(And, rp1, rp1, 0).imm(ci),
            _ => {
                let op = match (((w >> 12) & 1), ((w >> 5) & 3)) {
                    (0, 0) => Sub,
                    (0, 1) => Xor,
                    (0, 2) => Or,
                    (0, 3) => And,
                    (1, 0) => Subw,
                    (1, 1) => Addw,
                    _ => return None,
                };
                make(op, rp1, rp1, rp2)
            }
        },
        (1, 5) => {
            let imm = (((w >> 12) & 1) << 11)
                | (((w >> 8) & 1) << 10)
                | (((w >> 9) & 3) << 8)
                | (((w >> 6) & 1) << 7)
                | (((w >> 7) & 1) << 6)
                | (((w >> 2) & 1) << 5)
                | (((w >> 11) & 1) << 4)
                | (((w >> 3) & 7) << 1);
            make(Jump, 0, 0, 0).imm((pc as u64).wrapping_add(sext(imm, 12)))
        }
        (1, f @ (6 | 7)) => {
            let imm = (((w >> 12) & 1) << 8)
                | (((w >> 5) & 3) << 6)
                | (((w >> 2) & 1) << 5)
                | (((w >> 10) & 3) << 3)
                | (((w >> 3) & 3) << 1);
            let mut insn = make(if f == 6 { Beq } else { Bne }, 0, rp1, 0);
            insn.imm = (pc as u64).wrapping_add(sext(imm, 9));
            insn
        }
        (2, 0) => make(Shl, rd, rd, 0).imm(ci & 63),
        (2, f @ (2 | 3)) if rd != 0 => {
            let imm = if f == 2 {
                (((w >> 2) & 3) << 6) | (((w >> 12) & 1) << 5) | (((w >> 4) & 7) << 2)
            } else {
                (((w >> 2) & 7) << 6) | (((w >> 12) & 1) << 5) | (((w >> 5) & 3) << 3)
            };
            make(if f == 2 { Lw } else { Ld }, rd, 2, 0).imm(imm as u64)
        }
        (2, 4) => match ((w >> 12) & 1, rd, rs2) {
            (0, 0, 0) => return None,
            (0, _, 0) => make(JumpReg, 0, rd, 0),
            (0, _, _) => make(Add, rd, rs2, 0),
            (1, 0, 0) => make(Break, 0, 0, 0),
            (1, _, 0) => make(JumpReg, 1, rd, 0),
            _ => make(Add, rd, rd, rs2),
        },
        (2, f @ (6 | 7)) => {
            let imm = if f == 6 {
                (((w >> 7) & 3) << 6) | (((w >> 9) & 15) << 2)
            } else {
                (((w >> 7) & 7) << 6) | (((w >> 10) & 7) << 3)
            };
            make(if f == 6 { Sw } else { Sd }, 0, 2, rs2).imm(imm as u64)
        }
        _ => return None,
    };
    Some(insn)
}
