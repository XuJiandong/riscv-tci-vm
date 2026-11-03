use crate::vm::Error;
use goblin::elf::{Elf, header, program_header::*};

pub const MEMORY_SIZE: usize = 4 * 1024 * 1024;
pub const PAGE_SIZE: usize = 4096;
pub(crate) const PAGE_COUNT: usize = MEMORY_SIZE / PAGE_SIZE;
pub(crate) const READ: u8 = 1;
pub(crate) const WRITE: u8 = 2;
pub(crate) const EXEC: u8 = 4;
const STACK_SIZE: usize = 256 * 1024;

pub(crate) struct Memory {
    pub bytes: Box<[u8]>,
    pub permissions: Box<[u8; PAGE_COUNT]>,
}

impl Memory {
    pub fn load_elf(data: &[u8]) -> Result<(Self, u64), Error> {
        let elf = Elf::parse(data).map_err(|e| Error::Elf(e.to_string()))?;
        let invalid = |message: &str| Error::Elf(message.into());
        if !elf.is_64
            || !elf.little_endian
            || elf.header.e_machine != header::EM_RISCV
            || elf.header.e_type != header::ET_EXEC
        {
            return Err(invalid(
                "expected a static little-endian ELF64 RISC-V executable",
            ));
        }
        if elf.header.e_flags & 0x0e != 0 {
            return Err(invalid("floating-point and RV32E ELF ABIs are unsupported"));
        }
        let mut memory = Self {
            bytes: vec![0; MEMORY_SIZE].into_boxed_slice(),
            permissions: Box::new([READ | WRITE; PAGE_COUNT]),
        };
        // The first page is a guard page. The rest is flat RAM, with loaded
        // image pages narrowed to the union of the ELF segment permissions.
        memory.permissions[0] = 0;
        let mut image_pages = [false; PAGE_COUNT];
        let mut ranges = Vec::new();
        for ph in &elf.program_headers {
            if ph.p_type == PT_INTERP || ph.p_type == PT_DYNAMIC {
                return Err(invalid("dynamic loading is unsupported"));
            }
            if ph.p_type == PT_GNU_STACK && ph.p_flags & PF_X != 0 {
                return Err(invalid("executable stack violates W^X"));
            }
            if ph.p_type != PT_LOAD {
                continue;
            }
            if ph.p_filesz > ph.p_memsz {
                return Err(invalid("segment filesz exceeds memsz"));
            }
            if ph.p_align > 1
                && (!ph.p_align.is_power_of_two()
                    || ph.p_vaddr % ph.p_align != ph.p_offset % ph.p_align)
            {
                return Err(invalid("invalid segment alignment"));
            }
            let file_end = ph
                .p_offset
                .checked_add(ph.p_filesz)
                .filter(|&end| end <= data.len() as u64)
                .ok_or_else(|| invalid("segment lies outside the ELF file"))?;
            if ph.p_memsz == 0 {
                continue;
            }
            let end = ph
                .p_vaddr
                .checked_add(ph.p_memsz)
                .filter(|&end| end <= (MEMORY_SIZE - STACK_SIZE) as u64)
                .ok_or_else(|| invalid("segment exceeds RAM or overlaps the reserved stack"))?;
            if ph.p_vaddr < PAGE_SIZE as u64 {
                return Err(invalid("segment overlaps the null guard page"));
            }
            if ranges
                .iter()
                .any(|&(start, stop)| ph.p_vaddr < stop && end > start)
            {
                return Err(invalid("overlapping ELF segment address ranges"));
            }
            ranges.push((ph.p_vaddr, end));
            let flags = ((ph.p_flags & PF_R != 0) as u8 * READ)
                | ((ph.p_flags & PF_W != 0) as u8 * WRITE)
                | ((ph.p_flags & PF_X != 0) as u8 * EXEC);
            for (page, seen) in image_pages
                .iter_mut()
                .enumerate()
                .take((end as usize - 1) / PAGE_SIZE + 1)
                .skip(ph.p_vaddr as usize / PAGE_SIZE)
            {
                let combined = flags | if *seen { memory.permissions[page] } else { 0 };
                if combined & (WRITE | EXEC) == (WRITE | EXEC) {
                    return Err(invalid("ELF segments create a writable executable page"));
                }
                memory.permissions[page] = combined;
                *seen = true;
            }
            let start = ph.p_vaddr as usize;
            memory.bytes[start..start + ph.p_filesz as usize]
                .copy_from_slice(&data[ph.p_offset as usize..file_end as usize]);
            // The allocation starts zeroed, including BSS. Overlapping virtual
            // ranges were rejected so no segment can overwrite another's BSS.
        }
        if elf.entry & 1 != 0 || !memory.accessible(elf.entry, 2, EXEC) {
            return Err(invalid("entry point is not aligned executable memory"));
        }
        Ok((memory, elf.entry))
    }

    pub fn accessible(&self, address: u64, size: usize, permission: u8) -> bool {
        if size == 0 {
            return address <= MEMORY_SIZE as u64;
        }
        if size > MEMORY_SIZE || address > (MEMORY_SIZE - size) as u64 {
            return false;
        }
        let start = address as usize;
        self.permissions[start / PAGE_SIZE..=(start + size - 1) / PAGE_SIZE]
            .iter()
            .all(|p| p & permission == permission)
    }

    pub fn fetch16(&self, address: u64) -> Option<u16> {
        if address & 1 != 0 || !self.accessible(address, 2, EXEC) {
            return None;
        }
        let at = address as usize;
        Some(u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]]))
    }

    pub fn init_stack(&mut self, args: &[String]) -> Result<(u64, u64), Error> {
        let floor = MEMORY_SIZE - STACK_SIZE;
        let mut sp = MEMORY_SIZE;
        let mut pointers = Vec::with_capacity(args.len().min(STACK_SIZE / 8));
        for arg in args.iter().rev() {
            if arg.as_bytes().contains(&0) {
                return Err(Error::Elf("NUL byte in argument".into()));
            }
            sp = sp
                .checked_sub(arg.len().saturating_add(1))
                .filter(|&p| p >= floor)
                .ok_or_else(|| Error::Elf("arguments exceed the guest stack".into()))?;
            self.bytes[sp..sp + arg.len()].copy_from_slice(arg.as_bytes());
            pointers.push(sp as u64);
        }
        pointers.reverse();
        // argc, argv[], argv NULL, envp NULL, and AT_NULL/value. Linux-style
        // process entry stack, aligned to the RISC-V psABI's 16-byte boundary.
        let words = args
            .len()
            .checked_add(5)
            .and_then(|n| n.checked_mul(8))
            .ok_or_else(|| Error::Elf("too many arguments".into()))?;
        sp = sp
            .checked_sub(words)
            .map(|n| n & !15)
            .filter(|&p| p >= floor)
            .ok_or_else(|| Error::Elf("arguments exceed the guest stack".into()))?;
        self.bytes[sp..sp + 8].copy_from_slice(&(args.len() as u64).to_le_bytes());
        for (i, pointer) in pointers.iter().enumerate() {
            self.bytes[sp + (i + 1) * 8..sp + (i + 2) * 8].copy_from_slice(&pointer.to_le_bytes());
        }
        Ok((sp as u64, (sp + 8) as u64))
    }

    pub fn debug_string(&self, address: u64, pc: u64) -> Result<&[u8], Error> {
        let mut at = address;
        loop {
            if !self.accessible(at, 1, READ) {
                return Err(Error::Memory {
                    pc,
                    address: at,
                    access: "read debug string",
                });
            }
            let end = ((at as usize / PAGE_SIZE) + 1) * PAGE_SIZE;
            if let Some(n) = self.bytes[at as usize..end].iter().position(|&b| b == 0) {
                return Ok(&self.bytes[address as usize..at as usize + n]);
            }
            at = end as u64;
        }
    }
}
