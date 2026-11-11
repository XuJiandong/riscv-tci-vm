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
                        pair[immediate - 1] = pair[immediate - 1].imm(0);
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
    assert_eq!(code[0].dispatch_index, 128 + Opcode::LdLd as u16);
    code[0].set_opcode(Opcode::Ld);
    for (actual, expected) in code.iter().zip(original) {
        // All metadata, including each original PC and length, survives.
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    }
    fuse(&mut []);
    fuse(&mut code[..1]);
}
