mod common;
use common::*;
use riscv_tci_vm::{Error, Vm};

#[test]
fn fused_sequences_resume_at_every_guest_boundary_including_block_links() {
    let mut code = vec![
        i(0x13, 0, 1, 0, -1),
        i(0x13, 0, 2, 0, 3),
        0x000202b7, // LUI x5, 0x20 (data base)
    ];
    for _ in 0..9 {
        code.extend([
            r(0x33, 0, 0, 6, 1, 2), // ADD
            r(0x33, 0, 3, 7, 6, 1), // SLTU consumes sum
            r(0x33, 1, 3, 8, 1, 2), // MULHU
            r(0x33, 1, 0, 9, 2, 1), // MUL with reversed inputs
            s(3, 5, 8, 0),
            s(3, 5, 9, 8),
            i(3, 3, 10, 5, 0),
            i(3, 3, 11, 5, 8),
        ]);
    }
    let mut bytes = words(&code);
    bytes.extend(0x0305u16.to_le_bytes()); // C.ADDI x6, 1
    bytes.extend(words(&[r(0x33, 0, 3, 7, 6, 1)]));
    bytes.extend(words(&exit(0)));
    let image = elf(BASE, &[Segment::new(BASE, 5, bytes)]);
    let count = code.len() as u64 + 5;
    let mut full = Vm::from_elf(&image, &[]).unwrap();
    let expected = full.run(count, &mut Vec::new()).unwrap();
    assert_eq!(expected.exit_code, 0);
    assert_eq!(expected.instructions, count);
    for boundary in 0..count {
        let mut vm = Vm::from_elf(&image, &[]).unwrap();
        assert!(matches!(
            vm.run(boundary, &mut Vec::new()),
            Err(Error::InstructionLimit { instructions, .. }) if instructions == boundary
        ));
        let offset = if boundary <= code.len() as u64 {
            boundary * 4
        } else {
            boundary * 4 - 2
        };
        assert_eq!(vm.pc(), BASE + offset);
        let result = vm.run(count - boundary, &mut Vec::new()).unwrap();
        assert_eq!(result.instructions, count);
        assert_eq!(result.exit_code, 0);
        assert_eq!(vm.registers(), full.registers());
        assert_eq!(vm.pc(), full.pc());
    }
    // Repeated one-instruction resumes exercise the ordinary fallback for
    // each pair and lazy translation starting at its second instruction.
    let mut vm = Vm::from_elf(&image, &[]).unwrap();
    for boundary in 1..count {
        assert!(matches!(
            vm.run(1, &mut Vec::new()),
            Err(Error::InstructionLimit { .. })
        ));
        assert_eq!(vm.instructions(), boundary);
    }
    assert_eq!(vm.run(1, &mut Vec::new()).unwrap().exit_code, 0);
    assert_eq!(vm.registers(), full.registers());
}

#[test]
fn branch_can_enter_the_second_instruction_of_a_previously_fused_pair() {
    let mut code = vec![
        i(0x13, 0, 1, 0, -1),
        i(0x13, 0, 2, 0, 3),
        i(0x13, 0, 4, 0, 2),
        r(0x33, 1, 3, 3, 1, 2), // high half runs once
        r(0x33, 1, 0, 1, 1, 2), // low half runs twice, overwriting an input
        i(0x13, 0, 4, 4, -1),
        branch(1, 4, 0, -8),
    ];
    code.extend(exit(0));
    let mut vm = Vm::from_elf(&program(&code), &[]).unwrap();
    assert_eq!(vm.run(100, &mut Vec::new()).unwrap().exit_code, 0);
    assert_eq!(vm.instructions(), 13);
    assert_eq!(vm.registers()[1], u64::MAX - 8);
    assert_eq!(vm.registers()[3], 2);
}
