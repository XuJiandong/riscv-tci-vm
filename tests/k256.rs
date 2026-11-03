use riscv_tci_vm::Vm;

#[test]
fn supplied_k256_program_exits_successfully() {
    let path = std::env::var("K256_ELF").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/k256_ecdsa_ckbvm"
        )
        .into()
    });
    let image = std::fs::read(path).unwrap();
    let mut vm = Vm::from_elf(&image, &["1".into()]).unwrap();
    let mut logs = Vec::new();
    let result = vm.run(100_000_000, &mut logs).unwrap();
    assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&logs));
    assert!(
        result.instructions > 1_000_000,
        "must execute the cryptographic workload"
    );
}
