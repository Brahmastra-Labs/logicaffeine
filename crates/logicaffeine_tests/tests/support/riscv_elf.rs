//! Loading a statically linked RISC-V ELF into memory for the hart's model: its `PT_LOAD` segments (the bytes the file
//! holds, then zeros to the segment's memory size), its entry point, and its symbols by name.
#![allow(dead_code)]

use std::collections::BTreeMap;

/// A loadable segment: where it goes and the bytes it holds there, zero fill included.
pub struct Segment {
    pub addr: u64,
    pub bytes: Vec<u8>,
}

pub struct Elf {
    pub entry: u64,
    pub segments: Vec<Segment>,
    pub symbols: BTreeMap<String, u64>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Parse a little-endian ELF64 RISC-V executable; anything else panics, naming what it is not.
pub fn parse(b: &[u8]) -> Elf {
    assert!(b.len() > 64 && &b[0..4] == b"\x7fELF", "not an ELF file");
    assert_eq!(b[4], 2, "not a 64-bit ELF");
    assert_eq!(b[5], 1, "not a little-endian ELF");
    assert_eq!(u16_at(b, 18), 243, "not a RISC-V ELF");
    let entry = u64_at(b, 24);
    let phoff = u64_at(b, 32) as usize;
    let shoff = u64_at(b, 40) as usize;
    let phentsize = u16_at(b, 54) as usize;
    let phnum = u16_at(b, 56) as usize;
    let shentsize = u16_at(b, 58) as usize;
    let shnum = u16_at(b, 60) as usize;
    let mut segments = Vec::new();
    for i in 0..phnum {
        let p = phoff + i * phentsize;
        if u32_at(b, p) != 1 {
            continue;
        }
        let offset = u64_at(b, p + 8) as usize;
        let vaddr = u64_at(b, p + 16);
        let filesz = u64_at(b, p + 32) as usize;
        let memsz = u64_at(b, p + 40) as usize;
        let mut bytes = b[offset..offset + filesz].to_vec();
        bytes.resize(memsz, 0);
        segments.push(Segment { addr: vaddr, bytes });
    }
    let mut symbols = BTreeMap::new();
    for i in 0..shnum {
        let s = shoff + i * shentsize;
        if u32_at(b, s + 4) != 2 {
            continue;
        }
        let symoff = u64_at(b, s + 24) as usize;
        let symsize = u64_at(b, s + 32) as usize;
        let strtab = shoff + u32_at(b, s + 40) as usize * shentsize;
        let stroff = u64_at(b, strtab + 24) as usize;
        for k in 0..symsize / 24 {
            let e = symoff + k * 24;
            let name_off = stroff + u32_at(b, e) as usize;
            let end = b[name_off..].iter().position(|&c| c == 0).unwrap() + name_off;
            let name = String::from_utf8_lossy(&b[name_off..end]).to_string();
            if !name.is_empty() {
                symbols.insert(name, u64_at(b, e + 8));
            }
        }
    }
    Elf {
        entry,
        segments,
        symbols,
    }
}
