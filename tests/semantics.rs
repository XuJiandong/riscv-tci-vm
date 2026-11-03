mod common;
use common::*;
use riscv_tci_vm::{Error, MEMORY_SIZE, Vm};

fn machine(code: &[u32]) -> Vm {
    Vm::from_elf(&program(code), &[]).unwrap()
}

#[test]
fn signed_exit_and_zero_register() {
    let mut code = vec![i(0x13, 0, 0, 0, 7), i(0x13, 0, 5, 0, 42)];
    code.extend(exit(-7));
    let mut vm = machine(&code);
    let result = vm.run(100, &mut Vec::new()).unwrap();
    assert_eq!(result.exit_code, -7);
    assert_eq!(result.instructions, 5);
    assert_eq!(vm.registers()[0], 0);
    assert_eq!(vm.registers()[5], 42);
    assert_eq!(vm.run(0, &mut Vec::new()).unwrap().instructions, 5);
}

#[test]
fn budget_is_exact_and_resumable_through_block_links() {
    let mut code = vec![i(0x13, 0, 5, 5, 1); 150];
    code.extend(exit(0));
    let mut vm = machine(&code);
    for n in 1..=150 {
        assert!(
            matches!(vm.run(1, &mut Vec::new()), Err(Error::InstructionLimit { instructions, .. }) if instructions == n)
        );
        assert_eq!(vm.registers()[5], n);
        assert_eq!(vm.pc(), BASE + n * 4);
    }
    assert_eq!(vm.run(3, &mut Vec::new()).unwrap().exit_code, 0);
}

#[test]
fn faults_do_not_prevent_prior_instructions_or_retire() {
    for invalid in [0xffff_ffff, 0x0200_103b, 0x0200_101b, 0x6030_1013] {
        let mut vm = machine(&[i(0x13, 0, 5, 0, 42), invalid]);
        assert!(
            matches!(vm.run(100, &mut Vec::new()), Err(Error::IllegalInstruction { pc, .. }) if pc == BASE + 4)
        );
        assert_eq!(vm.registers()[5], 42);
        assert_eq!(vm.instructions(), 1);
    }
    let mut vm = machine(&[0x00100073]);
    assert!(matches!(
        vm.run(1, &mut Vec::new()),
        Err(Error::Breakpoint { .. })
    ));
    assert_eq!(vm.instructions(), 0);
}

#[test]
fn branches_and_jalr_read_sources_before_writing_links() {
    let mut code = vec![
        i(0x13, 0, 5, 0, 12),
        i(0x13, 0, 5, 5, -1),
        branch(1, 5, 0, -4),
    ];
    // AUIPC x1, 0; JALR x1, 13(x1): bit 0 cleared, rd aliases rs1.
    code.extend([0x00000097, i(0x67, 0, 1, 1, 13), 0x00100073]);
    code.extend(exit(0));
    let mut vm = machine(&code);
    assert_eq!(vm.run(100, &mut Vec::new()).unwrap().exit_code, 0);
    assert_eq!(vm.registers()[1], BASE + 20);
    assert_eq!(vm.registers()[5], 0);
}

#[test]
fn memory_widths_sign_extension_and_unaligned_access() {
    let mut code = vec![0x000202b7, i(0x13, 0, 6, 0, -128)]; // x5=0x20000
    code.push(s(3, 5, 6, 1)); // deliberately unaligned SD
    for (rd, f3) in [
        (20, 0),
        (21, 1),
        (22, 2),
        (23, 3),
        (24, 4),
        (25, 5),
        (26, 6),
    ] {
        code.push(i(3, f3, rd, 5, 1));
    }
    code.extend(exit(0));
    let mut vm = machine(&code);
    vm.run(100, &mut Vec::new()).unwrap();
    assert_eq!(&vm.registers()[20..24], &[(-128i64) as u64; 4]);
    assert_eq!(&vm.registers()[24..27], &[128, 65408, 4294967168]);
}

#[test]
fn writes_to_code_and_execution_from_data_fault() {
    let mut vm = machine(&[0x000102b7, s(0, 5, 0, 0)]);
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::Memory {
            access: "write",
            address: BASE,
            ..
        })
    ));
    let mut vm = machine(&[0x000202b7, i(0x67, 0, 0, 5, 0)]);
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::Memory {
            access: "execute",
            address: 0x20000,
            ..
        })
    ));
    // A discarded load still has to fault.
    let mut vm = machine(&[i(3, 3, 0, 0, 0)]);
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::Memory { access: "read", .. })
    ));
}

#[test]
fn memory_bounds_and_cross_page_permissions() {
    for address in [
        0,
        (MEMORY_SIZE - 7) as u64,
        MEMORY_SIZE as u64,
        u64::MAX - 3,
    ] {
        let code = [0x000202b7, i(3, 3, 5, 5, 0), i(3, 3, 6, 5, 0)];
        let elf = elf(
            BASE,
            &[
                Segment::new(BASE, 5, words(&code)),
                Segment::new(0x20000, 4, address.to_le_bytes().to_vec()),
            ],
        );
        let mut vm = Vm::from_elf(&elf, &[]).unwrap();
        assert!(matches!(
            vm.run(100, &mut Vec::new()),
            Err(Error::Memory { access: "read", .. })
        ));
    }
    // Store spans writable page 0x20000 and read-only page 0x21000.
    let code = [0x000212b7, i(0x13, 0, 5, 5, -3), s(3, 5, 0, 0)];
    let elf = elf(
        BASE,
        &[
            Segment::new(BASE, 5, words(&code)),
            Segment::new(0x21000, 4, vec![0]),
        ],
    );
    let mut vm = Vm::from_elf(&elf, &[]).unwrap();
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::Memory {
            address: 0x20ffd,
            access: "write",
            ..
        })
    ));
}

#[test]
fn fetch_across_page_boundary_checks_both_pages() {
    let code = words(&[i(0x13, 0, 5, 0, 42)]);
    let elf = elf(
        0x10ffe,
        &[
            Segment::new(0x10ffe, 5, code[..2].to_vec()),
            Segment::new(0x11000, 4, code[2..].to_vec()),
        ],
    );
    let mut vm = Vm::from_elf(&elf, &[]).unwrap();
    assert!(matches!(
        vm.run(1, &mut Vec::new()),
        Err(Error::Memory {
            pc: 0x10ffe,
            address: 0x11000,
            access: "execute"
        })
    ));
    assert_eq!(vm.instructions(), 0);
}

#[test]
fn debug_syscall_checks_strings_and_preserves_a0() {
    let mut code = vec![
        0x00020537,
        i(0x13, 0, 17, 0, 2047),
        i(0x13, 0, 17, 17, 130),
        0x73,
        i(0x13, 0, 5, 10, 0),
    ];
    code.extend(exit(0));
    let image = elf(
        BASE,
        &[
            Segment::new(BASE, 5, words(&code)),
            Segment::new(0x20000, 4, b"debug message\n\0".to_vec()),
        ],
    );
    let mut vm = Vm::from_elf(&image, &[]).unwrap();
    let mut output = Vec::new();
    vm.run(100, &mut output).unwrap();
    assert_eq!(output, b"debug message\n");
    assert_eq!(vm.registers()[5], 0x20000);

    let mut vm = machine(&[
        i(0x13, 0, 10, 0, -1),
        i(0x13, 0, 17, 0, 2047),
        i(0x13, 0, 17, 17, 130),
        0x73,
    ]);
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::Memory {
            access: "read debug string",
            ..
        })
    ));
    let mut vm = machine(&[0x73]);
    assert!(matches!(
        vm.run(100, &mut Vec::new()),
        Err(Error::UnknownSyscall { number: 0, .. })
    ));
}

#[test]
fn elf_validation_bss_and_stack() {
    assert!(Vm::from_elf(b"not an ELF", &[]).is_err());
    let code = words(&exit(0));
    for segments in [
        vec![Segment::new(BASE, 7, code.clone())],
        vec![
            Segment::new(BASE, 5, code.clone()),
            Segment::new(BASE + 0x800, 6, vec![0]),
        ],
        vec![
            Segment::new(BASE, 5, code.clone()),
            Segment::new(BASE + 4, 4, vec![0]),
        ],
        vec![
            Segment::new(BASE, 5, code.clone()),
            Segment::new(u64::MAX - 1, 4, vec![0; 4]),
        ],
        vec![
            Segment::new(BASE, 5, code.clone()),
            Segment {
                address: 0x20000,
                flags: 6,
                bytes: vec![0; 4],
                size: 2,
            },
        ],
    ] {
        assert!(Vm::from_elf(&elf(BASE, &segments), &[]).is_err());
    }
    let mut code = vec![
        0x000202b7,
        i(3, 3, 5, 5, 0),
        i(3, 3, 6, 2, 0),
        i(3, 3, 7, 11, 0),
        i(3, 4, 7, 7, 0),
    ];
    code.extend(exit(0));
    let image = elf(
        BASE,
        &[
            Segment::new(BASE, 5, words(&code)),
            Segment {
                address: 0x20000,
                flags: 6,
                bytes: vec![],
                size: 32,
            },
        ],
    );
    let mut vm = Vm::from_elf(&image, &["abc".into(), "def".into()]).unwrap();
    assert_eq!(vm.registers()[2] & 15, 0);
    vm.run(100, &mut Vec::new()).unwrap();
    assert_eq!(vm.registers()[5], 0); // BSS
    assert_eq!(vm.registers()[6], 2); // stack argc
    assert_eq!(vm.registers()[7], b'a' as u64); // argv[0]
}
