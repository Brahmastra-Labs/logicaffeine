//! The hart's RV64C decompressor against an independent decoder: GNU binutils. Every one of the 49,152 16-bit
//! encodings is disassembled twice — as the compressed instruction, and as the 32-bit instruction
//! `hart::rvc::expand` turns it into, at the same address so pc-relative targets print alike — and the two must say
//! the same thing once the compressed mnemonic is read through the specification's own table of what each `c.`
//! instruction stands for. An encoding the decompressor refuses must be one binutils refuses too, save the single
//! encoding the specification reserves that binutils still decodes.
//!
//! Tool-gated: `riscv64-linux-gnu-objdump`.

mod common;

use std::collections::BTreeMap;

use logicaffeine_compile::codegen_sva::hart::rvc::expand;

/// `C.ADDI16SP` with a zero immediate: the specification reserves it, and binutils decodes it as `c.addi16sp sp,0`.
const RESERVED_BUT_DECODED_BY_BINUTILS: [u16; 1] = [0x6101];

/// Disassemble `bytes` with no aliases: address → (mnemonic, operands).
fn disassemble(bytes: &[u8], label: &str) -> BTreeMap<u64, (String, String)> {
    let dir = std::env::temp_dir().join(format!("hart_rvc_lock_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let blob = dir.join("blob.bin");
    std::fs::write(&blob, bytes).unwrap();
    let out = std::process::Command::new("riscv64-linux-gnu-objdump")
        .args(["-D", "-b", "binary", "-m", "riscv:rv64", "-M", "no-aliases"])
        .arg(&blob)
        .output()
        .expect("run riscv64-linux-gnu-objdump");
    assert!(out.status.success(), "objdump failed: {}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let addr = cols.next()?.trim().strip_suffix(':')?;
            let addr = u64::from_str_radix(addr, 16).ok()?;
            let _hex = cols.next()?;
            let mnemonic = cols.next()?.trim().to_string();
            // binutils follows register values across the sweep and annotates computed addresses (`# 0x1008`).
            let operands = cols.next().unwrap_or("").split('#').next().unwrap_or("").trim().to_string();
            Some((addr, (mnemonic, operands)))
        })
        .collect()
}

/// What a compressed instruction stands for, written as the base instruction binutils prints, from the
/// specification's table: each `c.` mnemonic names one base instruction and how its operands map.
fn as_base(mnemonic: &str, operands: &str) -> Option<(String, String)> {
    let ops: Vec<&str> = if operands.is_empty() { vec![] } else { operands.split(',').collect() };
    let (m, o): (&str, Vec<String>) = match (mnemonic, ops.as_slice()) {
        ("c.addi4spn", [rd, sp, imm]) => ("addi", vec![rd.to_string(), sp.to_string(), imm.to_string()]),
        ("c.fld" | "c.fldsp", [r, mem]) => ("fld", vec![r.to_string(), mem.to_string()]),
        ("c.lw" | "c.lwsp", [r, mem]) => ("lw", vec![r.to_string(), mem.to_string()]),
        ("c.ld" | "c.ldsp", [r, mem]) => ("ld", vec![r.to_string(), mem.to_string()]),
        ("c.fsd" | "c.fsdsp", [r, mem]) => ("fsd", vec![r.to_string(), mem.to_string()]),
        ("c.sw" | "c.swsp", [r, mem]) => ("sw", vec![r.to_string(), mem.to_string()]),
        ("c.sd" | "c.sdsp", [r, mem]) => ("sd", vec![r.to_string(), mem.to_string()]),
        ("c.addi", [rd, imm]) => ("addi", vec![rd.to_string(), rd.to_string(), imm.to_string()]),
        ("c.addiw", [rd, imm]) => ("addiw", vec![rd.to_string(), rd.to_string(), imm.to_string()]),
        ("c.li", [rd, imm]) => ("addi", vec![rd.to_string(), "zero".into(), imm.to_string()]),
        ("c.addi16sp", [sp, imm]) => ("addi", vec![sp.to_string(), sp.to_string(), imm.to_string()]),
        ("c.lui", [rd, imm]) => ("lui", vec![rd.to_string(), imm.to_string()]),
        ("c.srli", [rd, sh]) => ("srli", vec![rd.to_string(), rd.to_string(), sh.to_string()]),
        ("c.srai", [rd, sh]) => ("srai", vec![rd.to_string(), rd.to_string(), sh.to_string()]),
        ("c.slli", [rd, sh]) => ("slli", vec![rd.to_string(), rd.to_string(), sh.to_string()]),
        ("c.srli64", [rd]) => ("srli", vec![rd.to_string(), rd.to_string(), "0x0".into()]),
        ("c.srai64", [rd]) => ("srai", vec![rd.to_string(), rd.to_string(), "0x0".into()]),
        ("c.slli64", [rd]) => ("slli", vec![rd.to_string(), rd.to_string(), "0x0".into()]),
        ("c.andi", [rd, imm]) => ("andi", vec![rd.to_string(), rd.to_string(), imm.to_string()]),
        ("c.sub", [rd, rs]) => ("sub", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.xor", [rd, rs]) => ("xor", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.or", [rd, rs]) => ("or", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.and", [rd, rs]) => ("and", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.subw", [rd, rs]) => ("subw", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.addw", [rd, rs]) => ("addw", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.j", [off]) => ("jal", vec!["zero".into(), off.to_string()]),
        ("c.beqz", [rs, off]) => ("beq", vec![rs.to_string(), "zero".into(), off.to_string()]),
        ("c.bnez", [rs, off]) => ("bne", vec![rs.to_string(), "zero".into(), off.to_string()]),
        ("c.jr", [rs]) => ("jalr", vec!["zero".into(), format!("0({rs})")]),
        ("c.jalr", [rs]) => ("jalr", vec!["ra".into(), format!("0({rs})")]),
        ("c.mv", [rd, rs]) => ("add", vec![rd.to_string(), "zero".into(), rs.to_string()]),
        ("c.add", [rd, rs]) => ("add", vec![rd.to_string(), rd.to_string(), rs.to_string()]),
        ("c.ebreak", []) => ("ebreak", vec![]),
        _ => return None,
    };
    Some((m.to_string(), o.join(",")))
}

#[test]
fn every_compressed_encoding_expands_to_what_binutils_decodes_it_as() {
    if !common::require_tool("riscv64-linux-gnu-objdump") {
        return;
    }
    let encodings: Vec<u16> = (0..=u16::MAX).filter(|c| c & 3 != 3).collect();
    let mut compressed = Vec::with_capacity(encodings.len() * 4);
    let mut expanded = Vec::with_capacity(encodings.len() * 4);
    for &c in &encodings {
        compressed.extend_from_slice(&c.to_le_bytes());
        compressed.extend_from_slice(&0x0001u16.to_le_bytes());
        expanded.extend_from_slice(&expand(c).unwrap_or(0x0000_0013).to_le_bytes());
    }
    let dc = disassemble(&compressed, "c");
    let de = disassemble(&expanded, "e");
    let (mut agreed, mut refused, mut wrong) = (0usize, 0usize, Vec::new());
    for (i, &c) in encodings.iter().enumerate() {
        let addr = 4 * i as u64;
        let (cm, co) = dc.get(&addr).unwrap_or_else(|| panic!("binutils printed nothing at {addr:#x} for {c:#06x}"));
        match expand(c) {
            None => {
                let binutils_refuses = cm == ".insn" || cm == "c.unimp";
                if !binutils_refuses && !RESERVED_BUT_DECODED_BY_BINUTILS.contains(&c) {
                    wrong.push(format!("{c:#06x}: refused, binutils decodes `{cm} {co}`"));
                }
                refused += 1;
            }
            Some(e) => {
                let (em, eo) = de.get(&addr).unwrap_or_else(|| panic!("binutils printed nothing for {e:#010x}"));
                match as_base(cm, co) {
                    Some((bm, bo)) if &bm == em && &bo == eo => agreed += 1,
                    Some((bm, bo)) => wrong.push(format!(
                        "{c:#06x} `{cm} {co}` stands for `{bm} {bo}`, expanded to {e:#010x} = `{em} {eo}`"
                    )),
                    None => wrong.push(format!("{c:#06x}: expanded to {e:#010x}, binutils decodes `{cm} {co}`")),
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} encodings disagree with binutils:\n{}",
        wrong.len(),
        encodings.len(),
        wrong.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
    assert_eq!(agreed + refused, encodings.len());
    assert!(agreed > 40_000, "only {agreed} encodings expanded; the decompressor refuses far too much");
}

#[test]
fn the_reserved_encodings_are_refused() {
    for (c, what) in [
        (0x0000u16, "the all-zero instruction"),
        (0x8000, "quadrant 0, funct3 100"),
        (0x6101, "C.ADDI16SP with a zero immediate"),
        (0x6501, "C.LUI with a zero immediate"),
        (0x2001, "C.ADDIW into x0"),
        (0x4002, "C.LWSP into x0"),
        (0x6002, "C.LDSP into x0"),
        (0x8002, "C.JR through x0"),
        (0x9c41, "quadrant 1 arithmetic 10 with bit 12 set"),
        (0x9c61, "quadrant 1 arithmetic 11 with bit 12 set"),
    ] {
        assert_eq!(expand(c), None, "{what} ({c:#06x}) must be an illegal instruction");
    }
}
