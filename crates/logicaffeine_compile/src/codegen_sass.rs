//! codegen_sass — the foundation of our OWN sm_86 SASS assembler (the MaxAs / turingas lever).
//!
//! `ptxas` is the only PTX→SASS path and it is closed-source: it fixes our register allocation and, crucially,
//! the per-instruction CONTROL CODES (stall counts, yields, read/write barriers, the operand reuse cache) that
//! decide how well a hot loop hides latency. That last 5-20% cuBLAS keeps lives there. To take it we must read
//! AND write SASS ourselves. This module is that capability: locate the kernel's machine code in the cubin ELF,
//! decode every 128-bit instruction's control field (verified bit-exact against `nvdisasm` on sm_86), re-encode
//! it, and patch the bytes back. A scheduler/optimizer on top is the follow-on; this is the assembler core.
//!
//! Ampere (sm_80/86) 128-bit instruction = (lo:u64, hi:u64). The control field sits in the high word:
//!
//! ```text
//! stall   bits [41:44]  yield bit [45]  wr_barrier [46:48]  rd_barrier [49:51]  wait_mask [52:57]  reuse [58:61]
//! ```
//!
//! (barrier index 7 = "none".) Verified: S2R/S2UR/memory ops set a write barrier; pure ALU ops do not.

use std::path::Path;
use std::process::Command;

/// One decoded SASS instruction: its byte offset within the .text section and the raw 128-bit encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SassInsn {
    pub offset: usize,
    pub lo: u64,
    pub hi: u64,
}

/// The per-instruction control codes — where the GPU's static scheduling lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    /// Fixed stall before issuing the next instruction (0–15 cycles).
    pub stall: u8,
    /// Yield hint (let another warp issue).
    pub yield_: bool,
    /// Write-barrier scoreboard this instruction sets (7 = none) — for variable-latency results.
    pub wr_barrier: u8,
    /// Read-barrier scoreboard this instruction sets (7 = none) — for operand-still-in-use.
    pub rd_barrier: u8,
    /// Which barrier scoreboards this instruction waits on (6-bit mask).
    pub wait_mask: u8,
    /// Operand reuse-cache flags (4 bits) — avoids re-reading the register file.
    pub reuse: u8,
}

impl Control {
    /// Decode the control field out of the high 64-bit word.
    pub fn decode(hi: u64) -> Control {
        Control {
            stall: ((hi >> 41) & 0xF) as u8,
            yield_: ((hi >> 45) & 0x1) != 0,
            wr_barrier: ((hi >> 46) & 0x7) as u8,
            rd_barrier: ((hi >> 49) & 0x7) as u8,
            wait_mask: ((hi >> 52) & 0x3F) as u8,
            reuse: ((hi >> 58) & 0xF) as u8,
        }
    }

    /// Write this control field back into `hi`, replacing exactly the control bits and leaving the rest intact.
    pub fn apply(&self, hi: u64) -> u64 {
        // Clear bits [41:61] then set them from the fields.
        let clear_mask: u64 = ((1u64 << (61 - 41 + 1)) - 1) << 41;
        let mut h = hi & !clear_mask;
        h |= ((self.stall & 0xF) as u64) << 41;
        h |= ((self.yield_ as u64) & 0x1) << 45;
        h |= ((self.wr_barrier & 0x7) as u64) << 46;
        h |= ((self.rd_barrier & 0x7) as u64) << 49;
        h |= ((self.wait_mask & 0x3F) as u64) << 52;
        h |= ((self.reuse & 0xF) as u64) << 58;
        h
    }
}

/// Locate the `.text.<kernel>` section in a cubin ELF64: returns (file_offset, size_in_bytes).
pub fn find_text_section(cubin: &[u8]) -> Option<(usize, usize)> {
    if cubin.len() < 64 || &cubin[0..4] != b"\x7fELF" {
        return None;
    }
    let rd_u64 = |o: usize| u64::from_le_bytes(cubin[o..o + 8].try_into().unwrap());
    let rd_u16 = |o: usize| u16::from_le_bytes(cubin[o..o + 2].try_into().unwrap());
    let e_shoff = rd_u64(0x28) as usize;
    let e_shentsize = rd_u16(0x3a) as usize;
    let e_shnum = rd_u16(0x3c) as usize;
    let e_shstrndx = rd_u16(0x3e) as usize;
    // section-header string table
    let shstr_hdr = e_shoff + e_shstrndx * e_shentsize;
    let shstr_off = rd_u64(shstr_hdr + 0x18) as usize;
    for i in 0..e_shnum {
        let sh = e_shoff + i * e_shentsize;
        let name_off = u32::from_le_bytes(cubin[sh..sh + 4].try_into().unwrap()) as usize;
        let name_start = shstr_off + name_off;
        let name_end = cubin[name_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| name_start + p)?;
        let name = std::str::from_utf8(&cubin[name_start..name_end]).ok()?;
        if name.starts_with(".text.") {
            let off = rd_u64(sh + 0x18) as usize;
            let size = rd_u64(sh + 0x20) as usize;
            return Some((off, size));
        }
    }
    None
}

/// Parse every 128-bit instruction out of the kernel's .text section of a cubin.
pub fn instructions(cubin: &[u8]) -> Vec<SassInsn> {
    let Some((off, size)) = find_text_section(cubin) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut o = off;
    while o + 16 <= off + size {
        let lo = u64::from_le_bytes(cubin[o..o + 8].try_into().unwrap());
        let hi = u64::from_le_bytes(cubin[o + 8..o + 16].try_into().unwrap());
        out.push(SassInsn {
            offset: o - off,
            lo,
            hi,
        });
        o += 16;
    }
    out
}

/// Patch instruction `idx`'s high word (control codes live here) in-place in the cubin bytes.
pub fn patch_hi(cubin: &mut [u8], idx: usize, new_hi: u64) {
    if let Some((off, _)) = find_text_section(cubin) {
        let o = off + idx * 16 + 8;
        cubin[o..o + 8].copy_from_slice(&new_hi.to_le_bytes());
    }
}

impl SassInsn {
    /// Source operand register Ra (Ampere: low word bits `[24:31]`). 255 = RZ (the zero register).
    pub fn ra(&self) -> u8 {
        ((self.lo >> 24) & 0xFF) as u8
    }
    /// Source operand register Rb (low word bits `[32:39]`) — only a real register for register-form opcodes.
    pub fn rb(&self) -> u8 {
        ((self.lo >> 32) & 0xFF) as u8
    }
    /// Destination register Rd (low word bits `[16:23]`).
    pub fn rd(&self) -> u8 {
        ((self.lo >> 16) & 0xFF) as u8
    }
}

/// Execution-unit class of a SASS opcode — what a scheduler must know to place it and set control codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SassClass {
    /// Fixed-latency integer/logic ALU (IMAD, IADD3, LEA, SHF, LOP3, ISETP, SEL, MOV, PRMT, …).
    Alu,
    /// Fixed-latency FP32 (FADD, FFMA, FMUL, FSETP, …) and FP16x2 (HADD2, HFMA2, …).
    Fp,
    /// Fixed-latency FP64 (DADD, DFMA, DMUL, …) — longer than FP32.
    Fp64,
    /// Multi-function unit (MUFU: rcp/sqrt/sin) — long fixed latency.
    Mufu,
    /// Tensor core matrix-multiply-accumulate (IMMA/HMMA) — fixed throughput (~8 cyc/int8 IMMA).
    Tensor,
    /// Variable-latency memory that SETS A WRITE BARRIER (LDG/LDS/LDSM/LDL/LD/ATOMG/CS2R/S2R/…).
    Memory,
    /// Asynchronous copy global->shared (LDGSTS = cp.async) — completion tracked by the pipeline, not a barrier.
    AsyncCopy,
    /// Store (STG/STS/STL/ST) — no result register.
    Store,
    /// Control-flow / barriers / sync (BRA/BSSY/BSYNC/BAR/WARPSYNC/EXIT/CALL/RET/YIELD/MEMBAR/…).
    Control,
    /// Uniform-datapath (scalar per-warp) ops (ULDC/UIADD3/UMOV/USHF/…).
    Uniform,
    /// Anything not yet classified.
    Other,
}

/// Classify an sm_86 SASS base mnemonic (the token before the first `.`). Covers the full 81-opcode inventory
/// extracted from real machine code (our GEMM + cuBLAS) — this is the ISA a SASS scheduler operates over.
pub fn classify_opcode(mnemonic: &str) -> SassClass {
    use SassClass::*;
    let base = mnemonic.split('.').next().unwrap_or(mnemonic);
    match base {
        // tensor cores
        "IMMA" | "HMMA" | "BMMA" => Tensor,
        // async copy (cp.async)
        "LDGSTS" => AsyncCopy,
        // variable-latency loads (set a write barrier) + special-register reads
        "LDG" | "LDS" | "LDSM" | "LDL" | "LD" | "LDGDEPBAR" | "ATOMG" | "ATOMS" | "RED" | "S2R"
        | "S2UR" | "CS2R" | "CCTL" | "SHFL" | "VOTEU" => Memory,
        // stores
        "STG" | "STS" | "STL" | "ST" => Store,
        // fp64
        "DADD" | "DMUL" | "DFMA" | "DSETP" => Fp64,
        // fp32 + fp16
        "FADD" | "FMUL" | "FFMA" | "FSETP" | "FSEL" | "FMNMX" | "FCHK" | "HADD2" | "HMUL2"
        | "HFMA2" | "HSETP2" | "F2F" | "F2I" | "I2F" | "I2FP" | "F2FP" | "FRND" | "F2IP" => Fp,
        "MUFU" => Mufu,
        // integer/logic ALU
        "IMAD" | "IADD3" | "LEA" | "SHF" | "LOP3" | "PLOP3" | "ISETP" | "SEL" | "MOV" | "PRMT"
        | "IMNMX" | "FLO" | "POPC" | "BFE" | "BFI" | "P2R" | "R2P" | "B2R" | "R2UR" | "NOP" => Alu,
        // uniform datapath
        "ULDC" | "UIADD3" | "UIMAD" | "USHF" | "ULEA" | "UMOV" | "UISETP" | "ULOP3" | "USEL"
        | "UPLOP3" => Uniform,
        // control / barriers
        "BRA" | "BSSY" | "BSYNC" | "BAR" | "WARPSYNC" | "EXIT" | "CALL" | "RET" | "YIELD"
        | "MEMBAR" | "ERRBAR" | "BREAK" | "BMOV" | "NANOSLEEP" | "DEPBAR" | "PBK" | "SSY" => {
            Control
        }
        _ => Other,
    }
}

impl SassClass {
    /// Fixed issue-to-RESULT latency in cycles — the delay before a dependent consumer may read the result.
    /// CALIBRATED by microbenchmark on this sm_86 GPU (clock64 dependent chains), NOT guessed: ALU/FP ~4.2-4.5
    /// (round to 5), int8 IMMA = 24 (MEASURED — a guessed 8 under-stalls the cross-iteration accumulate and
    /// hangs). Variable-latency classes (Memory/AsyncCopy) use write barriers instead of a fixed stall.
    pub fn fixed_latency(&self) -> u8 {
        match self {
            SassClass::Alu | SassClass::Uniform => 5, // measured IMAD 4.48, SHF/LOP3 4.21
            SassClass::Fp => 5,                       // measured FFMA 4.21
            SassClass::Fp64 => 8,
            SassClass::Mufu => 10,
            SassClass::Tensor => 24, // MEASURED int8 m16n8k32 dependent-accumulate latency (was 8 => hung)
            SassClass::Store | SassClass::Control => 1,
            SassClass::Memory | SassClass::AsyncCopy | SassClass::Other => 2,
        }
    }

    /// Functional-unit THROUGHPUT — minimum cycles between consecutive ops on the same pipe even with no
    /// register dependency. CALIBRATED on sm_86: ALU ~1.2, FP32 ~2.7, int8 IMMA ~16 for a single warp (ptxas
    /// uses ~8 assuming >=2 warps interleave the tensor pipe). Memory issue ~2. This is the constraint that
    /// serializes independent IMMAs and independent FFMAs — the register-only model misses it.
    pub fn throughput(&self) -> u8 {
        match self {
            SassClass::Tensor => 16, // single-warp measured; scheduler halves this per extra interleaved warp
            SassClass::Fp => 3,      // measured FFMA throughput 2.65
            SassClass::Fp64 => 4,
            SassClass::Mufu => 4,
            SassClass::Memory | SassClass::AsyncCopy | SassClass::Store => 2,
            _ => 1, // measured IMAD throughput 1.21
        }
    }
    /// Measured VARIABLE-latency result delay in cycles for barrier-tracked ops (L1/shared hit): LDG/LDSM ~45
    /// on sm_86 (pointer-chasing microbenchmark). The scheduler doesn't stall for this (a barrier does), but it
    /// uses it to know these results are FAR from ready — a consumer MUST wait on the scoreboard, never a stall.
    pub fn variable_result_latency(&self) -> u32 {
        match self {
            SassClass::Memory | SassClass::AsyncCopy => 45,
            _ => 0,
        }
    }

    /// Whether this op's result is produced with VARIABLE latency and therefore must set a write barrier
    /// (scoreboard) that dependent consumers wait on — the key fact a scheduler needs for correctness.
    pub fn is_variable_latency(&self) -> bool {
        matches!(self, SassClass::Memory | SassClass::AsyncCopy)
    }
}

impl SassInsn {
    /// The registers this instruction WRITES, given its execution `class` (from the nvdisasm mnemonic), SOUNDLY
    /// over-approximated — never under-count, or the scheduler would drop a real hazard. Multi-register
    /// producers (IMMA's 4-reg accumulator, wide LDSM/LDG) claim a conservative range from Rd; stores/control
    /// write nothing.
    pub fn writes_for(&self, class: SassClass) -> Vec<u8> {
        let rd = self.rd();
        if rd == 255 {
            return Vec::new();
        }
        let width: u8 = match class {
            SassClass::Tensor => 4,
            SassClass::Memory | SassClass::AsyncCopy => 4,
            SassClass::Store | SassClass::Control => 0,
            _ => 1,
        };
        (0..width)
            .map(|k| rd.wrapping_add(k))
            .filter(|&r| r != 255)
            .collect()
    }

    /// The registers this instruction READS, given its `class`, soundly over-approximated. ALU/FP read Ra,Rb;
    /// tensor ops also read accumulator (Rd..) + A-frag (Ra..) + B-frag (Rb..); stores read data Rd + addr Ra.
    pub fn reads_for(&self, class: SassClass) -> Vec<u8> {
        let (ra, rb, rd) = (self.ra(), self.rb(), self.rd());
        let mut r = Vec::new();
        let mut push = |x: u8| {
            if x != 255 && !r.contains(&x) {
                r.push(x);
            }
        };
        match class {
            SassClass::Tensor => {
                for k in 0..4 {
                    push(rd.wrapping_add(k));
                    push(ra.wrapping_add(k));
                }
                push(rb);
                push(rb.wrapping_add(1));
            }
            SassClass::Store => {
                push(rd);
                push(ra);
                push(ra.wrapping_add(1));
            }
            SassClass::Memory | SassClass::AsyncCopy => {
                push(ra);
                push(ra.wrapping_add(1));
            }
            SassClass::Control => {}
            _ => {
                push(ra);
                push(rb);
            }
        }
        r
    }
}

/// PRECISE operand decode from the nvdisasm TEXT of one instruction (everything after the mnemonic, before the
/// `;`). nvdisasm already prints the exact registers, so we parse them rather than reverse-engineer the binary
/// operand fields. Returns (writes, reads) as register numbers (RZ/255 dropped). Rules, verified against real
/// SASS: operand 0 is the WRITE destination UNLESS it is a memory ref `[...]` (a store — then it's a read
/// address and nothing is written); every other operand is a READ; registers inside `[...]` are address reads;
/// a `.64`/`.128` width on an address adds the consecutive address registers; accumulate ops (IMMA/FFMA)
/// naturally read their destination because it reappears as a later operand. Groups `{R8,R9,...}` expand.
pub fn precise_operands(operand_text: &str) -> (Vec<u8>, Vec<u8>) {
    // split the operand list on commas at brace/bracket depth 0.
    let mut parts: Vec<String> = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for ch in operand_text.chars() {
        match ch {
            '{' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            '}' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur.trim().to_string());
    }

    // extract register numbers from a token (handles R12, R12.reuse, {R8, R9}, [R16.64+0x10], etc.).
    fn regs_in(tok: &str) -> Vec<u8> {
        let bytes = tok.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            // a register is 'R' followed by digits, not preceded by a letter (avoid 'UR', 'PR', 'SR').
            if bytes[i] == b'R'
                && (i == 0 || !bytes[i - 1].is_ascii_alphabetic())
                && i + 1 < bytes.len()
                && bytes[i + 1].is_ascii_digit()
            {
                let mut j = i + 1;
                let mut num = 0u32;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    num = num * 10 + (bytes[j] - b'0') as u32;
                    j += 1;
                }
                if num < 255 {
                    out.push(num as u8);
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out
    }
    // widen an address register by .64/.128 (adds the consecutive pair/quad).
    fn addr_regs(tok: &str) -> Vec<u8> {
        let mut base = regs_in(tok);
        let extra = if tok.contains(".128") {
            3
        } else if tok.contains(".64") {
            1
        } else {
            0
        };
        if let Some(&b0) = base.first() {
            for k in 1..=extra {
                let r = b0.wrapping_add(k as u8);
                if r != 255 {
                    base.push(r);
                }
            }
        }
        base
    }

    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for (idx, p) in parts.iter().enumerate() {
        let is_mem = p.starts_with('[') || (p.starts_with("desc") && p.contains('['));
        if idx == 0 && !is_mem {
            // destination register(s).
            for r in regs_in(p) {
                if !writes.contains(&r) {
                    writes.push(r);
                }
            }
        } else if is_mem {
            for r in addr_regs(p) {
                if !reads.contains(&r) {
                    reads.push(r);
                }
            }
        } else {
            for r in regs_in(p) {
                if !reads.contains(&r) {
                    reads.push(r);
                }
            }
        }
    }
    (writes, reads)
}

/// Build a PRECISE dependency DAG from a cubin by parsing nvdisasm's exact operands per instruction (not the
/// conservative range approximation). This gives the scheduler maximal legal freedom — the graph has only the
/// TRUE hazards, so a reschedule can hide latency ptxas left exposed. Returns (insns, classes, mnemonics, DAG).
pub fn precise_dependency_graph(
    cubin_path: &Path,
) -> Option<(Vec<SassInsn>, Vec<SassClass>, DepGraph)> {
    let bytes = std::fs::read(cubin_path).ok()?;
    let insns = instructions(&bytes);
    if insns.is_empty() {
        return None;
    }
    let dis = nvdisasm(cubin_path)?;
    // per-instruction (mnemonic, operand-text) from the disassembly lines, in order.
    let mut ops: Vec<(String, String)> = Vec::new();
    for l in dis.lines() {
        let s = l.trim();
        if s.starts_with("/*") {
            if let Some(rest) = s.split("*/").nth(1) {
                let body = rest.split(';').next().unwrap_or("").trim();
                // strip a leading predicate '@P0' / '@!P0'
                let body = body
                    .strip_prefix('@')
                    .map(|b| b.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
                    .unwrap_or_else(|| body.to_string());
                let mut it = body.splitn(2, char::is_whitespace);
                let mnem = it.next().unwrap_or("").to_string();
                let operands = it.next().unwrap_or("").to_string();
                if !mnem.is_empty() {
                    ops.push((mnem, operands));
                }
            }
        }
    }
    let n = insns.len().min(ops.len());
    let classes: Vec<SassClass> = (0..n).map(|i| classify_opcode(&ops[i].0)).collect();
    let mut reads: Vec<Vec<u8>> = Vec::with_capacity(n);
    let mut writes: Vec<Vec<u8>> = Vec::with_capacity(n);
    for i in 0..n {
        let (w, r) = precise_operands(&ops[i].1);
        writes.push(w);
        reads.push(r);
    }
    let is_barrier: Vec<bool> = classes.iter().map(|c| *c == SassClass::Control).collect();
    let g = build_dependency_graph(&reads, &writes, &is_barrier);
    Some((insns[..n].to_vec(), classes, g))
}

/// Build the dependency DAG directly from a cubin, using nvdisasm to name each opcode (so classification —
/// hence the read/write sets — is precise). Returns the graph paired with the parsed instructions. This is the
/// mnemonic-aware, SOUND path the scheduler uses.
pub fn dependency_graph_from_cubin(
    cubin_path: &Path,
) -> Option<(Vec<SassInsn>, Vec<SassClass>, DepGraph)> {
    let bytes = std::fs::read(cubin_path).ok()?;
    let insns = instructions(&bytes);
    if insns.is_empty() {
        return None;
    }
    let dis = nvdisasm(cubin_path)?;
    let mnems: Vec<String> = dis
        .lines()
        .filter_map(|l| {
            let s = l.trim();
            if s.starts_with("/*") {
                s.split("*/")
                    .nth(1)
                    .and_then(|r| r.trim().split_whitespace().next())
                    .map(|m| m.to_string())
            } else {
                None
            }
        })
        .collect();
    let classes: Vec<SassClass> = (0..insns.len())
        .map(|i| classify_opcode(mnems.get(i).map(|s| s.as_str()).unwrap_or("IMAD")))
        .collect();
    let reads: Vec<Vec<u8>> = insns
        .iter()
        .zip(&classes)
        .map(|(ins, &c)| ins.reads_for(c))
        .collect();
    let writes: Vec<Vec<u8>> = insns
        .iter()
        .zip(&classes)
        .map(|(ins, &c)| ins.writes_for(c))
        .collect();
    let is_barrier: Vec<bool> = classes.iter().map(|c| *c == SassClass::Control).collect();
    let g = build_dependency_graph(&reads, &writes, &is_barrier);
    Some((insns, classes, g))
}

/// A register-dependency DAG over a straight-line SASS region: `deps[i]` lists earlier instructions that `i`
/// must stay after (RAW/WAR/WAW hazards + barrier ordering). Any reordering that respects these edges preserves
/// semantics — this is what the scheduler is free to permute to hide latency.
#[derive(Debug, Clone)]
pub struct DepGraph {
    pub deps: Vec<Vec<usize>>,
}

/// Build the dependency DAG from per-instruction (reads, writes, is_barrier) triples. Barriers (BAR/BSYNC/
/// control) force a total order across themselves: nothing moves past a barrier in either direction.
pub fn build_dependency_graph(
    reads: &[Vec<u8>],
    writes: &[Vec<u8>],
    is_barrier: &[bool],
) -> DepGraph {
    let n = reads.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut last_writer: [Option<usize>; 256] = [None; 256];
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); 256];
    let mut last_barrier: Option<usize> = None;
    let add = |deps: &mut Vec<Vec<usize>>, i: usize, j: usize| {
        if i != j && !deps[i].contains(&j) {
            deps[i].push(j);
        }
    };
    for i in 0..n {
        // barrier ordering: this instruction stays after the previous barrier.
        if let Some(b) = last_barrier {
            add(&mut deps, i, b);
        }
        // RAW: read-after-write on each source register.
        for &r in &reads[i] {
            if let Some(w) = last_writer[r as usize] {
                add(&mut deps, i, w);
            }
        }
        // WAW + WAR on each destination register.
        for &w in &writes[i] {
            if let Some(pw) = last_writer[w as usize] {
                add(&mut deps, i, pw); // WAW
            }
            let rs = std::mem::take(&mut readers[w as usize]);
            for reader in rs {
                add(&mut deps, i, reader); // WAR
            }
        }
        // update state.
        for &r in &reads[i] {
            readers[r as usize].push(i);
        }
        for &w in &writes[i] {
            last_writer[w as usize] = Some(i);
            readers[w as usize].clear();
        }
        if is_barrier[i] {
            // a barrier depends on everything before it, and becomes the new fence.
            for j in 0..i {
                add(&mut deps, i, j);
            }
            last_barrier = Some(i);
        }
    }
    DepGraph { deps }
}

/// CONTROL-CODE ASSIGNER (correctness-first, stalls). Recompute every instruction's STALL count from our own
/// latency model — `stall = fixed_latency(class)` — which is provably >= the cycles any fixed-latency consumer
/// needs (the immediate next instruction already issues `stall` cycles later, so later consumers have even
/// more). We KEEP ptxas's write/read barriers + wait masks (the variable-latency memory / cp.async tracking,
/// the fragile part) untouched. If the kernel stays numerically correct with OUR stalls, the stall model is
/// validated on-device — the first half of a working control-code assigner. Returns the patched cubin bytes.
pub fn assign_stalls_by_latency(cubin_path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cubin_path).ok()?;
    let insns = instructions(&bytes);
    if insns.is_empty() {
        return None;
    }
    let dis = nvdisasm(cubin_path)?;
    let mnems: Vec<String> = dis
        .lines()
        .filter_map(|l| {
            let s = l.trim();
            if s.starts_with("/*") {
                s.split("*/")
                    .nth(1)
                    .and_then(|r| r.trim().split_whitespace().next())
                    .map(|m| m.to_string())
            } else {
                None
            }
        })
        .collect();
    let mut out = bytes.clone();
    for (i, ins) in insns.iter().enumerate() {
        let class = classify_opcode(mnems.get(i).map(|s| s.as_str()).unwrap_or("IMAD"));
        let mut ctl = Control::decode(ins.hi); // preserve barriers/waits/reuse/yield
                                               // our own stall from the latency model, but never LOWER than what ptxas chose (ptxas may know a
                                               // throughput/dual-issue constraint we don't model — staying >= its stall keeps us sound).
        let ours = class.fixed_latency().min(15);
        ctl.stall = ctl.stall.max(ours);
        patch_hi(&mut out, i, ctl.apply(ins.hi));
    }
    Some(out)
}

/// CONTROL-CODE ASSIGNER (perf): compute the MINIMAL stall per instruction from the fixed-latency dependency
/// timing, keeping ptxas's memory barriers. We walk the instruction order tracking the cycle each register's
/// fixed-latency result becomes ready; each instruction issues at the max of (prev issue + 1) and its
/// producers' ready cycles; the stall between two instructions is exactly that gap (clamped to `[1,15]`). This is
/// the tightest schedule that still satisfies every fixed-latency hazard — if ptxas over-stalled anywhere, ours
/// is faster; if it was already minimal, ours matches. Correctness is guarded on-device (a too-tight stall that
/// violates a hazard we don't model shows up as a wrong result). Returns the patched cubin bytes.
pub fn assign_minimal_stalls(cubin_path: &Path) -> Option<Vec<u8>> {
    assign_minimal_stalls_opt(cubin_path, true)
}

/// As above, but `floor_ptxas` selects whether to floor our computed stall at ptxas's (sound) or trust our
/// CALIBRATED model outright (`false` — the true test of whether the measured latencies are complete).
pub fn assign_minimal_stalls_opt(cubin_path: &Path, floor_ptxas: bool) -> Option<Vec<u8>> {
    let (insns, classes, _g) = precise_dependency_graph(cubin_path)?;
    // precise reads/writes again (the graph consumed them); recompute from the disassembly text.
    let dis = nvdisasm(cubin_path)?;
    let mut ops: Vec<String> = Vec::new();
    for l in dis.lines() {
        let s = l.trim();
        if s.starts_with("/*") {
            if let Some(rest) = s.split("*/").nth(1) {
                let body = rest.split(';').next().unwrap_or("").trim();
                let body = body
                    .strip_prefix('@')
                    .map(|b| b.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
                    .unwrap_or_else(|| body.to_string());
                let operands = body
                    .splitn(2, char::is_whitespace)
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                ops.push(operands);
            }
        }
    }
    let n = insns.len().min(ops.len());
    let lat: Vec<u32> = classes.iter().map(|c| c.fixed_latency() as u32).collect();

    let mut ready = [0u32; 256]; // cycle each reg's FIXED-latency result is available
    let mut issue = vec![0u32; n];
    // functional-unit throughput: consecutive ops on the same unit cannot issue faster than the unit's rate,
    // EVEN with no register dependency (e.g. IMMAs writing different accumulators still serialize on the tensor
    // pipe). This is the constraint the register-only model missed (it caused a tensor-pipe overrun -> hang).
    let mut last_unit_issue: std::collections::HashMap<SassClass, u32> =
        std::collections::HashMap::new();
    // multi-warp effective tensor issue rate (8) — a well-occupied kernel interleaves >=2 warps over the
    // 16-cycle tensor pipe. Other units from the calibrated throughput() table.
    let throughput = |c: SassClass| -> u32 {
        match c {
            SassClass::Tensor => 8,
            other => other.throughput() as u32,
        }
    };
    for p in 0..n {
        let (w, r) = precise_operands(&ops[p]);
        let mut earliest = if p > 0 { issue[p - 1] + 1 } else { 0 };
        for &reg in &r {
            earliest = earliest.max(ready[reg as usize]);
        }
        // throughput on this instruction's functional unit
        if let Some(&last) = last_unit_issue.get(&classes[p]) {
            earliest = earliest.max(last + throughput(classes[p]));
        }
        issue[p] = earliest;
        last_unit_issue.insert(classes[p], issue[p]);
        if !classes[p].is_variable_latency() {
            for &reg in &w {
                ready[reg as usize] = issue[p] + lat[p];
            }
        }
        // variable-latency writes use ptxas's write barrier (kept) — consumers wait on it, not on `ready`.
    }
    // stall[p] = gap to the next instruction's issue (>=1, <=15). Keep barriers/waits/reuse from ptxas.
    let bytes = std::fs::read(cubin_path).ok()?;
    let mut out = bytes.clone();
    for p in 0..n {
        let mut ctl = Control::decode(insns[p].hi);
        let gap = if p + 1 < n {
            issue[p + 1].saturating_sub(issue[p])
        } else {
            lat[p]
        };
        // SOUND floor: never go below ptxas's stall. Our latency+throughput model captures the hazards we know;
        // ptxas encodes additional ones we haven't fully reverse-engineered (barrier-wait timing, dual-issue,
        // register-bank conflicts). Flooring at its value keeps the kernel correct while applying our (higher)
        // stalls where our model demands them. A truly-minimal below-ptxas schedule needs the COMPLETE hazard
        // model — that is the remaining reverse-engineering to actually beat ptxas.
        ctl.stall = if floor_ptxas {
            (gap.clamp(1, 15) as u8).max(ctl.stall)
        } else {
            gap.clamp(1, 15) as u8
        };
        patch_hi(&mut out, p, ctl.apply(insns[p].hi));
    }
    Some(out)
}

/// LIST SCHEDULER — the scheduler engine. Given the sound dependency DAG and per-instruction latencies, produce
/// a new instruction order that respects every hazard while placing latency-critical work early so its result
/// is ready by the time a consumer needs it (classic critical-path list scheduling). Returns the new order as
/// original-instruction indices. This is what a re-emitter writes back to close the gap ptxas's schedule leaves.
pub fn list_schedule(g: &DepGraph, latencies: &[u8]) -> Vec<usize> {
    let n = g.deps.len();
    // successors + in-degree
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (i, ds) in g.deps.iter().enumerate() {
        indeg[i] = ds.len();
        for &j in ds {
            succ[j].push(i);
        }
    }
    // critical-path length (longest latency-weighted path to a sink), computed in reverse topo order.
    let mut cpl = vec![0u32; n];
    // process nodes in reverse index order — since edges point backward (j<i), successors have larger indices;
    // a reverse pass gives each node its successors' cpl already computed.
    for i in (0..n).rev() {
        let mut best = 0u32;
        for &s in &succ[i] {
            best = best.max(cpl[s]);
        }
        cpl[i] = best + latencies.get(i).copied().unwrap_or(1) as u32;
    }
    // Kahn with a max-cpl tie-break (and stable original order as the final tie-break for determinism).
    let mut ready: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while !ready.is_empty() {
        // pick the ready node with the highest critical-path length; tie-break to the earliest original index
        // (keeps the schedule close to ptxas's where the DAG is indifferent, which is where it is already good).
        let (mut bi, mut bpos) = (0usize, 0usize);
        for (pos, &node) in ready.iter().enumerate() {
            let better = cpl[node] > cpl[bi] || (cpl[node] == cpl[bi] && node < bi);
            if pos == 0 || better {
                bi = node;
                bpos = pos;
            }
        }
        ready.swap_remove(bpos);
        order.push(bi);
        for &s in &succ[bi] {
            indeg[s] -= 1;
            if indeg[s] == 0 {
                ready.push(s);
            }
        }
    }
    order
}

/// The reuse-cache optimizer pass — our first SASS optimization ON TOP of ptxas. The operand collector can
/// REUSE a source register read by the previous instruction instead of re-reading the register file, saving
/// RF bandwidth (the bottleneck under our 222-register, 16.7%-occupancy kernel). ptxas already sets many reuse
/// flags; this finds the ones it MISSED: consecutive instructions that read the same register into the same
/// operand slot (Ra→reuse bit 0 = `hi[58]`, Rb→reuse bit 1 = `hi[59]`) but where the flag is unset. RZ is skipped
/// (reusing the zero register is pointless). Returns the patched cubin bytes and the number of flags added.
/// The result MUST be validated on-device (varied-fill oracle) — the operand→slot mapping is verified there,
/// never trusted blind.
pub fn optimize_reuse(cubin: &[u8]) -> (Vec<u8>, usize) {
    let insns = instructions(cubin);
    let mut out = cubin.to_vec();
    let mut added = 0usize;
    for i in 1..insns.len() {
        let (prev, cur) = (&insns[i - 1], &insns[i]);
        let mut ctl = Control::decode(cur.hi);
        let mut changed = false;
        // Ra reuse (slot 0): current Ra read the same non-RZ register the previous instruction read as Ra.
        if cur.ra() != 255 && cur.ra() == prev.ra() && (ctl.reuse & 0b0001) == 0 {
            ctl.reuse |= 0b0001;
            changed = true;
        }
        // Rb reuse (slot 1): same for the Rb operand slot.
        if cur.rb() != 255 && cur.rb() == prev.rb() && (ctl.reuse & 0b0010) == 0 {
            ctl.reuse |= 0b0010;
            changed = true;
        }
        if changed {
            patch_hi(&mut out, i, ctl.apply(cur.hi));
            added += 1;
        }
    }
    (out, added)
}

/// Disassemble a cubin file to SASS text via nvdisasm (for inspection / labelling).
pub fn nvdisasm(cubin_path: &Path) -> Option<String> {
    let o = Command::new("nvdisasm")
        .args(["-c", "-hex"])
        .arg(cubin_path)
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// What the COMPILER learns about the machine code `ptxas` produced for one of its kernels — the SASS-awareness
/// the backend needs to reason about the ptxas ceiling (register pressure/occupancy, tensor-core utilisation,
/// reuse-cache usage). Every field is read from the real cubin, not guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SassReport {
    pub kernel: String,
    pub instructions: usize,
    pub imma: usize,         // tensor-core MACs
    pub ldsm: usize,         // ldmatrix cooperative loads
    pub reuse_insns: usize,  // instructions that use the operand reuse cache
    pub registers: usize,    // registers/thread (SHI_REGISTERS) — the occupancy driver
    pub avg_stall_x100: u64, // mean fixed stall ×100 (integer to stay Eq/Hash-friendly)
}

impl SassReport {
    /// Estimate resident blocks/SM from the register count (Ampere: 65,536 regs/SM) at `threads` per block —
    /// this is where a register-bound kernel loses to cuBLAS, and the compiler can now SEE it.
    pub fn blocks_per_sm(&self, threads: usize) -> usize {
        if self.registers == 0 || threads == 0 {
            return 0;
        }
        (65536 / (self.registers * threads)).min(16)
    }

    /// Analyse the cubin a compiler backend just produced: parse its machine code + control codes and correlate
    /// with the SASS mnemonics. The backend calls this to know how well ptxas scheduled its kernel.
    pub fn analyze(cubin_path: &Path, kernel: &str) -> Option<SassReport> {
        let bytes = std::fs::read(cubin_path).ok()?;
        let insns = instructions(&bytes);
        if insns.is_empty() {
            return None;
        }
        let dis = nvdisasm(cubin_path)?;
        // mnemonic per instruction (the token after the `/*addr*/` marker).
        let mnems: Vec<String> = dis
            .lines()
            .filter_map(|l| {
                let s = l.trim();
                if s.starts_with("/*") {
                    s.split("*/")
                        .nth(1)
                        .and_then(|r| r.trim().split_whitespace().next())
                        .map(|m| m.to_string())
                } else {
                    None
                }
            })
            .collect();
        let count = |needle: &str| mnems.iter().filter(|m| m.contains(needle)).count();
        let reuse_insns = insns
            .iter()
            .filter(|i| Control::decode(i.hi).reuse != 0)
            .count();
        let stall_sum: u64 = insns
            .iter()
            .map(|i| Control::decode(i.hi).stall as u64)
            .sum();
        // registers/thread from nvdisasm's `.sectioninfo @"SHI_REGISTERS=N"`.
        let registers = dis
            .lines()
            .find_map(|l| l.split("SHI_REGISTERS=").nth(1))
            .and_then(|r| {
                r.trim_end_matches(|c: char| !c.is_ascii_digit())
                    .parse::<usize>()
                    .ok()
            })
            .unwrap_or(0);
        Some(SassReport {
            kernel: kernel.to_string(),
            instructions: insns.len(),
            imma: count("IMMA"),
            ldsm: count("LDSM"),
            reuse_insns,
            registers,
            avg_stall_x100: stall_sum * 100 / insns.len() as u64,
        })
    }
}

/// The compiler emits `cu_src`, compiles it with nvcc to a cubin, and returns the SASS report — the backend
/// "knowing" the machine code its codegen ultimately becomes. Requires nvcc + nvdisasm; returns None otherwise.
pub fn analyze_source(cu_src: &str, kernel: &str, arch: &str) -> Option<SassReport> {
    let dir = std::env::temp_dir().join(format!("logos_sassrep_{}_{}", std::process::id(), kernel));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, cu_src).ok()?;
    let o = Command::new("nvcc")
        .args(["-O3", &format!("-arch={arch}"), "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let rep = SassReport::analyze(&cubin, kernel);
    let _ = std::fs::remove_dir_all(&dir);
    rep
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sass/tests.rs"]
mod tests;
