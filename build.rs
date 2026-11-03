use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").unwrap(),
        "x86_64",
        "x86_64 host required"
    );
    assert_eq!(env::var("CARGO_CFG_TARGET_ENDIAN").unwrap(), "little");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let definitions = fs::read_to_string("src/ops.def").unwrap();
    let names: Vec<_> = definitions
        .lines()
        .filter_map(|l| l.strip_prefix("OP(")?.strip_suffix(')'))
        .collect();
    assert!(names.len() < 256);
    let mut rust = String::from(
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[repr(u8)]\npub(crate) enum Opcode {\n",
    );
    for name in names {
        rust.push_str(&format!("    {name},\n"));
    }
    rust.push_str("}\n");
    fs::write(out.join("opcodes.rs"), rust).unwrap();
    let mut cc = Command::new("clang-19");
    // Let LLVM duplicate the computed-goto dispatch tails across the opcode
    // table. Its default predecessor/successor limits (16) prevent this for
    // our 86 handlers, even when the instruction thresholds are increased.
    // Keep these Clang 19 settings together; see README.md for measurements
    // and disassembly checks before changing the compiler or opcode table.
    cc.args([
        "-std=gnu11",
        "-O3",
        "-mllvm",
        "-tail-dup-placement-threshold=128",
        "-mllvm",
        "-tail-dup-size=128",
        "-mllvm",
        "-tail-dup-indirect-size=128",
        "-mllvm",
        "-tail-dup-pred-size=128",
        "-mllvm",
        "-tail-dup-succ-size=128",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-fno-strict-aliasing",
        "-fPIC",
        "-c",
        "src/tci.c",
        "-o",
    ])
    .arg(out.join("tci.o"));
    if env::var_os("TCI_SANITIZE").is_some() {
        cc.args(["-fsanitize=undefined", "-fno-sanitize-recover=all"]);
        println!("cargo:rustc-link-lib=ubsan");
    }
    assert!(cc.status().expect("clang-19 is required").success());
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libtci.a"))
            .arg(out.join("tci.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=tci");
    println!("cargo:rerun-if-changed=src/tci.c");
    println!("cargo:rerun-if-changed=src/ops.def");
    println!("cargo:rerun-if-env-changed=TCI_SANITIZE");
}
