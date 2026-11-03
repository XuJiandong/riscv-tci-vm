#![allow(dead_code)]
pub const BASE: u64 = 0x10000;

pub struct Segment {
    pub address: u64,
    pub flags: u32,
    pub bytes: Vec<u8>,
    pub size: u64,
}

impl Segment {
    pub fn new(address: u64, flags: u32, bytes: Vec<u8>) -> Self {
        let size = bytes.len() as u64;
        Self {
            address,
            flags,
            bytes,
            size,
        }
    }
}

pub fn elf(entry: u64, segments: &[Segment]) -> Vec<u8> {
    let mut result = vec![0u8; 64 + 56 * segments.len()];
    result[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut result, 16, 2); // ET_EXEC
    put16(&mut result, 18, 243); // EM_RISCV
    put32(&mut result, 20, 1);
    put64(&mut result, 24, entry);
    put64(&mut result, 32, 64);
    put16(&mut result, 52, 64);
    put16(&mut result, 54, 56);
    put16(&mut result, 56, segments.len() as u16);
    put16(&mut result, 58, 64);
    for (i, segment) in segments.iter().enumerate() {
        let offset = result.len() as u64;
        let at = 64 + 56 * i;
        put32(&mut result, at, 1); // PT_LOAD
        put32(&mut result, at + 4, segment.flags);
        put64(&mut result, at + 8, offset);
        put64(&mut result, at + 16, segment.address);
        put64(&mut result, at + 24, segment.address);
        put64(&mut result, at + 32, segment.bytes.len() as u64);
        put64(&mut result, at + 40, segment.size);
        put64(&mut result, at + 48, 1);
        result.extend_from_slice(&segment.bytes);
    }
    result
}

pub fn put16(data: &mut [u8], at: usize, n: u16) {
    data[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
pub fn put32(data: &mut [u8], at: usize, n: u32) {
    data[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
pub fn put64(data: &mut [u8], at: usize, n: u64) {
    data[at..at + 8].copy_from_slice(&n.to_le_bytes());
}

pub fn words(code: &[u32]) -> Vec<u8> {
    code.iter().flat_map(|n| n.to_le_bytes()).collect()
}
pub fn r(op: u32, f7: u32, f3: u32, rd: u32, a: u32, b: u32) -> u32 {
    f7 << 25 | b << 20 | a << 15 | f3 << 12 | rd << 7 | op
}
pub fn i(op: u32, f3: u32, rd: u32, a: u32, imm: i32) -> u32 {
    ((imm as u32) & 4095) << 20 | a << 15 | f3 << 12 | rd << 7 | op
}
pub fn s(f3: u32, a: u32, b: u32, imm: i32) -> u32 {
    let u = imm as u32 & 4095;
    (u >> 5) << 25 | b << 20 | a << 15 | f3 << 12 | (u & 31) << 7 | 0x23
}
pub fn branch(f3: u32, a: u32, b: u32, offset: i32) -> u32 {
    let u = offset as u32;
    ((u >> 12) & 1) << 31
        | ((u >> 5) & 63) << 25
        | b << 20
        | a << 15
        | f3 << 12
        | ((u >> 1) & 15) << 8
        | ((u >> 11) & 1) << 7
        | 0x63
}
pub fn exit(code: i32) -> [u32; 3] {
    [i(0x13, 0, 10, 0, code), i(0x13, 0, 17, 0, 93), 0x73]
}
pub fn program(code: &[u32]) -> Vec<u8> {
    elf(BASE, &[Segment::new(BASE, 5, words(code))])
}
