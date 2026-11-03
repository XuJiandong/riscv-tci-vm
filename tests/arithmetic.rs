mod common;
use common::*;
use riscv_tci_vm::Vm;

type Oracle = fn(u64, u64) -> u64;
fn sx(n: u32) -> u64 {
    n as i32 as i64 as u64
}
fn polynomial(a: u64, b: u64) -> u128 {
    (0..64)
        .filter(|bit| (b >> bit) & 1 != 0)
        .fold(0, |v, bit| v ^ ((a as u128) << bit))
}

#[test]
fn integer_and_bitmanipulation_vectors() {
    let mut cases: Vec<(&str, u32, Oracle)> = Vec::new();
    macro_rules! reg {
        ($name:expr, $op:expr, $f7:expr, $f3:expr, $oracle:expr) => {
            cases.push(($name, r($op, $f7, $f3, 5, 1, 2), $oracle));
        };
    }
    macro_rules! imm {
        ($name:expr, $op:expr, $f3:expr, $imm:expr, $oracle:expr) => {
            cases.push(($name, i($op, $f3, 5, 1, $imm), $oracle));
        };
    }
    reg!("add", 0x33, 0, 0, u64::wrapping_add);
    reg!("sub", 0x33, 0x20, 0, u64::wrapping_sub);
    reg!("sll", 0x33, 0, 1, |a, b| a << (b & 63));
    reg!("slt", 0x33, 0, 2, |a, b| ((a as i64) < b as i64) as u64);
    reg!("sltu", 0x33, 0, 3, |a, b| (a < b) as u64);
    reg!("xor", 0x33, 0, 4, |a, b| a ^ b);
    reg!("srl", 0x33, 0, 5, |a, b| a >> (b & 63));
    reg!("sra", 0x33, 0x20, 5, |a, b| ((a as i64) >> (b & 63)) as u64);
    reg!("or", 0x33, 0, 6, |a, b| a | b);
    reg!("and", 0x33, 0, 7, |a, b| a & b);
    reg!("addw", 0x3b, 0, 0, |a, b| sx(a.wrapping_add(b) as u32));
    reg!("subw", 0x3b, 0x20, 0, |a, b| sx(a.wrapping_sub(b) as u32));
    reg!("sllw", 0x3b, 0, 1, |a, b| sx((a as u32) << (b & 31)));
    reg!("srlw", 0x3b, 0, 5, |a, b| sx((a as u32) >> (b & 31)));
    reg!(
        "sraw",
        0x3b,
        0x20,
        5,
        |a, b| ((a as i32) >> (b & 31)) as i64 as u64
    );
    reg!("mul", 0x33, 1, 0, u64::wrapping_mul);
    reg!(
        "mulh",
        0x33,
        1,
        1,
        |a, b| ((a as i64 as i128 * b as i64 as i128) >> 64) as u64
    );
    reg!(
        "mulhsu",
        0x33,
        1,
        2,
        |a, b| ((a as i64 as i128 * b as i128) >> 64) as u64
    );
    reg!("mulhu", 0x33, 1, 3, |a, b| ((a as u128 * b as u128) >> 64)
        as u64);
    reg!("div", 0x33, 1, 4, |a, b| if b == 0 {
        u64::MAX
    } else {
        (a as i64).wrapping_div(b as i64) as u64
    });
    reg!("divu", 0x33, 1, 5, |a, b| a
        .checked_div(b)
        .unwrap_or(u64::MAX));
    reg!("rem", 0x33, 1, 6, |a, b| if b == 0 {
        a
    } else {
        (a as i64).wrapping_rem(b as i64) as u64
    });
    reg!("remu", 0x33, 1, 7, |a, b| if b == 0 { a } else { a % b });
    reg!("mulw", 0x3b, 1, 0, |a, b| sx(a.wrapping_mul(b) as u32));
    reg!("divw", 0x3b, 1, 4, |a, b| if b as u32 == 0 {
        u64::MAX
    } else {
        (a as i32).wrapping_div(b as i32) as i64 as u64
    });
    reg!("divuw", 0x3b, 1, 5, |a, b| sx((a as u32)
        .checked_div(b as u32)
        .unwrap_or(u32::MAX)));
    reg!("remw", 0x3b, 1, 6, |a, b| if b as u32 == 0 {
        sx(a as u32)
    } else {
        (a as i32).wrapping_rem(b as i32) as i64 as u64
    });
    reg!("remuw", 0x3b, 1, 7, |a, b| if b as u32 == 0 {
        sx(a as u32)
    } else {
        sx(a as u32 % b as u32)
    });
    reg!("andn", 0x33, 0x20, 7, |a, b| a & !b);
    reg!("orn", 0x33, 0x20, 6, |a, b| a | !b);
    reg!("xnor", 0x33, 0x20, 4, |a, b| !(a ^ b));
    reg!("rol", 0x33, 0x30, 1, |a, b| a.rotate_left(b as u32));
    reg!("ror", 0x33, 0x30, 5, |a, b| a.rotate_right(b as u32));
    reg!("rolw", 0x3b, 0x30, 1, |a, b| sx(
        (a as u32).rotate_left(b as u32)
    ));
    reg!("rorw", 0x3b, 0x30, 5, |a, b| sx(
        (a as u32).rotate_right(b as u32)
    ));
    reg!("min", 0x33, 5, 4, |a, b| (a as i64).min(b as i64) as u64);
    reg!("max", 0x33, 5, 6, |a, b| (a as i64).max(b as i64) as u64);
    reg!("minu", 0x33, 5, 5, u64::min);
    reg!("maxu", 0x33, 5, 7, u64::max);
    reg!("sh1add", 0x33, 0x10, 2, |a, b| (a << 1).wrapping_add(b));
    reg!("sh2add", 0x33, 0x10, 4, |a, b| (a << 2).wrapping_add(b));
    reg!("sh3add", 0x33, 0x10, 6, |a, b| (a << 3).wrapping_add(b));
    reg!("sh1add.uw", 0x3b, 0x10, 2, |a, b| ((a as u32 as u64) << 1)
        .wrapping_add(b));
    reg!("sh2add.uw", 0x3b, 0x10, 4, |a, b| ((a as u32 as u64) << 2)
        .wrapping_add(b));
    reg!("sh3add.uw", 0x3b, 0x10, 6, |a, b| ((a as u32 as u64) << 3)
        .wrapping_add(b));
    reg!("add.uw", 0x3b, 4, 0, |a, b| (a as u32 as u64)
        .wrapping_add(b));
    reg!("bclr", 0x33, 0x24, 1, |a, b| a & !(1 << (b & 63)));
    reg!("bext", 0x33, 0x24, 5, |a, b| (a >> (b & 63)) & 1);
    reg!("binv", 0x33, 0x34, 1, |a, b| a ^ (1 << (b & 63)));
    reg!("bset", 0x33, 0x14, 1, |a, b| a | (1 << (b & 63)));
    reg!("clmul", 0x33, 5, 1, |a, b| polynomial(a, b) as u64);
    reg!("clmulh", 0x33, 5, 3, |a, b| (polynomial(a, b) >> 64) as u64);
    reg!("clmulr", 0x33, 5, 2, |a, b| (polynomial(a, b) >> 63) as u64);
    cases.push(("zext.h", r(0x3b, 4, 4, 5, 1, 0), |a, _| a as u16 as u64));
    imm!("addi", 0x13, 0, -2048, |a, _| a
        .wrapping_add((-2048i64) as u64));
    imm!("slti", 0x13, 2, -1, |a, _| ((a as i64) < -1) as u64);
    imm!("sltiu", 0x13, 3, -1, |a, _| (a < u64::MAX) as u64);
    imm!("xori", 0x13, 4, -31, |a, _| a ^ (-31i64) as u64);
    imm!("ori", 0x13, 6, -31, |a, _| a | (-31i64) as u64);
    imm!("andi", 0x13, 7, -31, |a, _| a & (-31i64) as u64);
    imm!("slli", 0x13, 1, 63, |a, _| a << 63);
    imm!("srli", 0x13, 5, 63, |a, _| a >> 63);
    imm!("srai", 0x13, 5, 0x43f, |a, _| ((a as i64) >> 63) as u64);
    imm!("addiw", 0x1b, 0, -1, |a, _| sx(a.wrapping_sub(1) as u32));
    imm!("slliw", 0x1b, 1, 31, |a, _| sx((a as u32) << 31));
    imm!("srliw", 0x1b, 5, 31, |a, _| sx((a as u32) >> 31));
    imm!("sraiw", 0x1b, 5, 0x41f, |a, _| ((a as i32) >> 31) as i64
        as u64);
    imm!("clz", 0x13, 1, 0x600, |a, _| a.leading_zeros() as u64);
    imm!("ctz", 0x13, 1, 0x601, |a, _| a.trailing_zeros() as u64);
    imm!("cpop", 0x13, 1, 0x602, |a, _| a.count_ones() as u64);
    imm!("clzw", 0x1b, 1, 0x600, |a, _| (a as u32).leading_zeros()
        as u64);
    imm!("ctzw", 0x1b, 1, 0x601, |a, _| (a as u32).trailing_zeros()
        as u64);
    imm!("cpopw", 0x1b, 1, 0x602, |a, _| (a as u32).count_ones()
        as u64);
    imm!("sext.b", 0x13, 1, 0x604, |a, _| a as i8 as i64 as u64);
    imm!("sext.h", 0x13, 1, 0x605, |a, _| a as i16 as i64 as u64);
    imm!("rev8", 0x13, 5, 0x6b8, |a, _| a.swap_bytes());
    imm!("orc.b", 0x13, 5, 0x287, |a, _| u64::from_le_bytes(
        a.to_le_bytes().map(|b| if b == 0 { 0 } else { 255 })
    ));
    imm!("rori", 0x13, 5, 0x63f, |a, _| a.rotate_right(63));
    imm!("roriw", 0x1b, 5, 0x61f, |a, _| sx(
        (a as u32).rotate_right(31)
    ));
    imm!("slli.uw", 0x1b, 1, 0x0bf, |a, _| (a as u32 as u64) << 63);
    imm!("bclri", 0x13, 1, 0x4bf, |a, _| a & !(1 << 63));
    imm!("bexti", 0x13, 5, 0x4bf, |a, _| a >> 63);
    imm!("binvi", 0x13, 1, 0x6bf, |a, _| a ^ (1 << 63));
    imm!("bseti", 0x13, 1, 0x2bf, |a, _| a | (1 << 63));

    let mut values = vec![
        0,
        1,
        u64::MAX,
        1 << 63,
        0x80000000,
        0x7fffffff,
        32,
        63,
        64,
        0xff00ff0000ff0080,
    ];
    let mut rng = 0x123456789abcdef0u64;
    for _ in 0..6 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        values.push(rng);
    }
    let mut code = vec![0x00100337]; // x6 = 0x100000 (constant table)
    let mut data = Vec::new();
    let mut descriptions = Vec::new();
    for (name, instruction, oracle) in cases {
        for &a in &values {
            for &b in &values {
                let expected = oracle(a, b);
                data.extend(words64(&[a, b, expected]));
                descriptions.push(format!("{name}({a:#x}, {b:#x}) = {expected:#x}"));
                code.extend([
                    i(3, 3, 1, 6, 0),
                    i(3, 3, 2, 6, 8),
                    i(3, 3, 3, 6, 16),
                    instruction,
                    branch(0, 5, 3, 8),
                    0x00100073,
                    i(0x13, 0, 6, 6, 24),
                ]);
            }
        }
    }
    code.extend(exit(0));
    let image = elf(
        BASE,
        &[
            Segment::new(BASE, 5, words(&code)),
            Segment::new(0x100000, 4, data),
        ],
    );
    let mut vm = Vm::from_elf(&image, &[]).unwrap();
    if let Err(error) = vm.run(1_000_000, &mut Vec::new()) {
        let case = ((vm.pc() - BASE - 4) / 28) as usize;
        panic!(
            "{error}: {}; result={:#x}",
            descriptions[case],
            vm.registers()[5]
        );
    }
}

fn words64(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_le_bytes()).collect()
}
