//! A minimal x86-64 instruction encoder for the contiguous register-allocated
//! region backend (`compile_region_regalloc`).
//!
//! This is deliberately tiny: it covers exactly the instruction shapes the
//! linear-scan int backend emits — register/immediate moves, the three-operand
//! arithmetic the allocator lowers (via a two-address `dst = lhs; dst OP rhs`
//! discipline), compares + `setcc`, conditional and unconditional jumps with
//! late-bound labels, frame loads/stores (`base + disp`), prologue/epilogue
//! register save/restore, and `ret`. It is NOT a general assembler.
//!
//! Register numbering follows the hardware encoding (`rax=0 … r15=15`); the REX
//! prefix and ModRM/SIB bytes are emitted directly. All emitted code is
//! position-independent except the `jcc`/`jmp` displacements, which are resolved
//! against final offsets by [`Asm::resolve`] before the bytes leave this module.

#![cfg(target_arch = "x86_64")]

/// A hardware general-purpose register, by its 4-bit encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Reg {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

impl Reg {
    /// The low 3 bits (the ModRM/opcode field).
    #[inline]
    fn lo3(self) -> u8 {
        (self as u8) & 0b111
    }
    /// The high bit (the REX extension bit).
    #[inline]
    fn hi(self) -> u8 {
        ((self as u8) >> 3) & 1
    }
}

/// A hardware XMM register (SSE2), by its 4-bit encoding. f64 slots that win a
/// physical register live here for the whole function (a SECOND register class
/// alongside the GP [`Reg`] one); the rest spill to the frame. All XMM registers
/// are caller-saved under SysV, so none need save/restore in the prologue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Xmm {
    Xmm0 = 0,
    Xmm1 = 1,
    Xmm2 = 2,
    Xmm3 = 3,
    Xmm4 = 4,
    Xmm5 = 5,
    Xmm6 = 6,
    Xmm7 = 7,
    Xmm8 = 8,
    Xmm9 = 9,
    Xmm10 = 10,
    Xmm11 = 11,
    Xmm12 = 12,
    Xmm13 = 13,
    Xmm14 = 14,
    Xmm15 = 15,
}

impl Xmm {
    /// The low 3 bits (the ModRM/opcode field).
    #[inline]
    fn lo3(self) -> u8 {
        (self as u8) & 0b111
    }
    /// The high bit (the REX extension bit).
    #[inline]
    fn hi(self) -> u8 {
        ((self as u8) >> 3) & 1
    }
}

/// An integer condition, mapping to an x86 condition code. The first six are
/// SIGNED; [`Cond::AeU`] is the one UNSIGNED code the array bounds check needs
/// (`jae` — "above or equal", unsigned `>=`), used so a 1-based index whose
/// `idx - 1` wraps negative (e.g. index 0) trips the OOB exit, matching the
/// stencil's `(im1 as u64) >= (len as u64)` guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    /// `<` (signed less): `cc=0xC`.
    Lt,
    /// `>` (signed greater): `cc=0xF`.
    Gt,
    /// `<=` (signed less-or-equal): `cc=0xE`.
    Le,
    /// `>=` (signed greater-or-equal): `cc=0xD`.
    Ge,
    /// `==`: `cc=0x4`.
    Eq,
    /// `!=`: `cc=0x5`.
    Ne,
    /// `>=` UNSIGNED (above or equal): `cc=0x3` (`jae`/`jnc`). The array
    /// bounds-check uses this to fold the lower (`im1 < 0`) and upper
    /// (`im1 >= len`) out-of-bounds cases into ONE unsigned comparison.
    AeU,
    /// `>` UNSIGNED (above): `cc=0x7` (`ja`). After `ucomisd`, `seta` is the
    /// strict ordered-greater test (CF=0 && ZF=0) — NaN (ZF=1) folds to FALSE,
    /// the float `>`/`<` (with swapped operands) primitive.
    AU,
    /// UNSIGNED below-or-equal: `cc=0x6` (`jbe`). The NEGATION of `AU` (CF=1 ||
    /// ZF=1) — the "comparison FALSE" branch for a float `>`/`<` BranchF, where
    /// NaN (ZF=1) must TAKE the false branch.
    BeU,
    /// UNSIGNED below: `cc=0x2` (`jb`). After `ucomisd`, the strict
    /// ordered-less primitive (CF=1); NaN folds to FALSE.
    BU,
    /// PARITY EVEN (`jp`/`jpe`): `cc=0xA`. After `ucomisd`, PF=1 ⟺ the operands
    /// were UNORDERED (a NaN). The `DivF` zero-divisor guard uses this to skip
    /// the side-exit on a NaN divisor (NaN is not `0.0`).
    ParityEven,
    /// PARITY ODD (`jnp`/`jpo`): `cc=0xB`. After `ucomisd`, PF=0 ⟺ ORDERED —
    /// the second half of the IEEE `==` test (`sete && setnp`), so a NaN
    /// compare yields false exactly like the reference `a == b`.
    ParityOdd,
    /// OVERFLOW (`jo`): `cc=0x0`. OF=1 after a signed `add`/`sub`/`imul` — the
    /// integer-overflow side-exit (`jo deopt`) for exact arithmetic: on overflow
    /// the native tier deopts so the exact VM recomputes (and promotes to BigInt).
    Overflow,
}

impl Cond {
    /// The 4-bit x86 condition-code tetrad (`jcc` low nibble = `0x80 | cc`,
    /// `setcc` = `0x90 | cc`).
    fn cc(self) -> u8 {
        match self {
            Cond::Eq => 0x4,
            Cond::Ne => 0x5,
            Cond::Ge => 0xD,         // GE (signed): NL
            Cond::Lt => 0xC,         // L  (signed)
            Cond::Le => 0xE,         // LE (signed)
            Cond::Gt => 0xF,         // G  (signed): NLE
            Cond::AeU => 0x3,        // AE (unsigned): NB/NC
            Cond::AU => 0x7,         // A  (unsigned): NBE
            Cond::BeU => 0x6,        // BE (unsigned): NA
            Cond::BU => 0x2,         // B  (unsigned): C
            Cond::ParityEven => 0xA, // P/PE (parity even): unordered after ucomisd
            Cond::ParityOdd => 0xB,  // NP/PO (parity odd): ordered after ucomisd
            Cond::Overflow => 0x0,   // O (overflow): OF=1 after signed add/sub/imul
        }
    }
}

/// A label in the instruction stream. Created with [`Asm::new_label`], its
/// position fixed with [`Asm::bind`], and referenced by jumps; all references
/// are patched in [`Asm::resolve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelId(pub usize);

enum Fixup {
    /// A rel32 jump site: 4 LE bytes at `site` are filled with
    /// `target_off - (site + 4)`.
    Rel32 { site: usize, label: LabelId },
}

/// The x86-64 byte emitter with late-bound labels.
pub struct Asm {
    buf: Vec<u8>,
    /// `label_pos[i]` = byte offset of label `i`, or `usize::MAX` until bound.
    label_pos: Vec<usize>,
    fixups: Vec<Fixup>,
}

impl Default for Asm {
    fn default() -> Self {
        Asm::new()
    }
}

impl Asm {
    /// An empty assembler.
    pub fn new() -> Self {
        Asm {
            buf: Vec::with_capacity(256),
            label_pos: Vec::new(),
            fixups: Vec::new(),
        }
    }

    /// The current byte length (a position label).
    pub fn pos(&self) -> usize {
        self.buf.len()
    }

    /// Reserve a new (unbound) label.
    pub fn new_label(&mut self) -> LabelId {
        let id = LabelId(self.label_pos.len());
        self.label_pos.push(usize::MAX);
        id
    }

    /// Fix label `l` at the current position.
    pub fn bind(&mut self, l: LabelId) {
        self.label_pos[l.0] = self.buf.len();
    }

    /// REX prefix. `w` = 64-bit operand; `r` = ModRM.reg high bit; `x` = SIB
    /// index high bit; `b` = ModRM.rm / opcode-reg high bit.
    fn rex(&mut self, w: bool, r: u8, x: u8, b: u8) {
        let byte = 0x40 | ((w as u8) << 3) | (r << 2) | (x << 1) | b;
        self.buf.push(byte);
    }

    /// `mov dst, imm64` (REX.W + B8+rd io). Always 10 bytes; simple and exact.
    pub fn mov_ri(&mut self, dst: Reg, imm: i64) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xB8 + dst.lo3());
        self.buf.extend_from_slice(&imm.to_le_bytes());
    }

    /// `mov dst, src` (REX.W 89 /r, src in reg field).
    pub fn mov_rr(&mut self, dst: Reg, src: Reg) {
        if dst == src {
            return;
        }
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x89);
        self.modrm_reg(src, dst);
    }

    /// ModRM for register-direct (`mod=11`): reg field = `r`, rm field = `m`.
    fn modrm_reg(&mut self, r: Reg, m: Reg) {
        self.buf.push(0b1100_0000 | (r.lo3() << 3) | m.lo3());
    }

    /// ModRM + (optional) SIB + disp for a `[base + disp32]` memory operand,
    /// with `r` as the reg field. Always emits a disp32 for simplicity.
    fn modrm_mem(&mut self, r: Reg, base: Reg, disp: i32) {
        // mod=10 (disp32). rm=base.lo3(); rm==100 (rsp/r12) needs a SIB byte.
        let rm = base.lo3();
        self.buf.push(0b1000_0000 | (r.lo3() << 3) | rm);
        if rm == 0b100 {
            // SIB: scale=0, index=100 (none), base=rm.
            self.buf.push(0b0000_0000 | (0b100 << 3) | rm);
        }
        self.buf.extend_from_slice(&disp.to_le_bytes());
    }

    /// `mov dst, [base + disp]` (REX.W 8B /r).
    pub fn mov_rm(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.rex(true, dst.hi(), 0, base.hi());
        self.buf.push(0x8B);
        self.modrm_mem(dst, base, disp);
    }

    /// `mov [base + disp], src` (REX.W 89 /r).
    pub fn mov_mr(&mut self, base: Reg, disp: i32, src: Reg) {
        self.rex(true, src.hi(), 0, base.hi());
        self.buf.push(0x89);
        self.modrm_mem(src, base, disp);
    }

    /// `add dst, src` (REX.W 01 /r).
    pub fn add_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x01);
        self.modrm_reg(src, dst);
    }

    /// `sub dst, src` (REX.W 29 /r).
    pub fn sub_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x29);
        self.modrm_reg(src, dst);
    }

    /// `imul dst, src` (REX.W 0F AF /r — dst is the reg field).
    pub fn imul_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, dst.hi(), 0, src.hi());
        self.buf.push(0x0F);
        self.buf.push(0xAF);
        self.modrm_reg(dst, src);
    }

    /// `popcnt dst, src` (F3 REX.W 0F B8 /r) — `dst = src.count_ones()`. The F3 prefix precedes REX.
    pub fn popcnt_rr(&mut self, dst: Reg, src: Reg) {
        self.buf.push(0xF3);
        self.rex(true, dst.hi(), 0, src.hi());
        self.buf.push(0x0F);
        self.buf.push(0xB8);
        self.modrm_reg(dst, src);
    }

    /// `lzcnt dst, src` (F3 REX.W 0F BD /r) — `dst = src.leading_zeros()` (64 for 0). The F3 prefix (which
    /// precedes REX) is what distinguishes it from `bsr` (0F BD, undefined for 0); requires the ABM feature.
    pub fn lzcnt_rr(&mut self, dst: Reg, src: Reg) {
        self.buf.push(0xF3);
        self.rex(true, dst.hi(), 0, src.hi());
        self.buf.push(0x0F);
        self.buf.push(0xBD);
        self.modrm_reg(dst, src);
    }

    /// `tzcnt dst, src` (F3 REX.W 0F BC /r) — `dst = src.trailing_zeros()` (64 for 0). The F3 prefix (which
    /// precedes REX) is what distinguishes it from `bsf` (0F BC, undefined for 0); requires the BMI1 feature.
    pub fn tzcnt_rr(&mut self, dst: Reg, src: Reg) {
        self.buf.push(0xF3);
        self.rex(true, dst.hi(), 0, src.hi());
        self.buf.push(0x0F);
        self.buf.push(0xBC);
        self.modrm_reg(dst, src);
    }

    /// `sub dst, imm32` (REX.W 81 /5 id) — sign-extended 32-bit immediate.
    pub fn sub_ri(&mut self, dst: Reg, imm: i32) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0x81);
        self.buf.push(0b1100_0000 | (5 << 3) | dst.lo3());
        self.buf.extend_from_slice(&imm.to_le_bytes());
    }

    /// `add dst, imm32` (REX.W 81 /0 id) — sign-extended 32-bit immediate.
    pub fn add_ri(&mut self, dst: Reg, imm: i32) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0x81);
        self.buf.push(0b1100_0000 | dst.lo3());
        self.buf.extend_from_slice(&imm.to_le_bytes());
    }

    /// `cmp dst, imm32` (REX.W 81 /7 id) — `dst - imm`, sets flags.
    pub fn cmp_ri(&mut self, dst: Reg, imm: i32) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0x81);
        self.buf.push(0b1100_0000 | (7 << 3) | dst.lo3());
        self.buf.extend_from_slice(&imm.to_le_bytes());
    }

    /// `call reg` (FF /2) — an indirect near call through a register.
    pub fn call_r(&mut self, target: Reg) {
        if target.hi() == 1 {
            self.buf.push(0x41);
        }
        self.buf.push(0xFF);
        self.buf.push(0b1100_0000 | (2 << 3) | target.lo3());
    }

    /// `shl dst, imm8` (REX.W C1 /4 ib) — shift left by a constant count.
    pub fn shl_ri(&mut self, dst: Reg, imm: u8) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xC1);
        self.buf.push(0b1100_0000 | (4 << 3) | dst.lo3());
        self.buf.push(imm);
    }

    /// `and dst, src` (REX.W 21 /r).
    pub fn and_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x21);
        self.modrm_reg(src, dst);
    }

    /// `or dst, src` (REX.W 09 /r).
    pub fn or_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x09);
        self.modrm_reg(src, dst);
    }

    /// `xor dst, src` (REX.W 31 /r).
    pub fn xor_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src.hi(), 0, dst.hi());
        self.buf.push(0x31);
        self.modrm_reg(src, dst);
    }

    /// `not dst` (REX.W F7 /2).
    pub fn not_r(&mut self, dst: Reg) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xF7);
        self.buf.push(0b1100_0000 | (2 << 3) | dst.lo3());
    }

    /// `neg dst` (REX.W F7 /3).
    pub fn neg_r(&mut self, dst: Reg) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xF7);
        self.buf.push(0b1100_0000 | (3 << 3) | dst.lo3());
    }

    /// `shl dst, cl` (REX.W D3 /4).
    pub fn shl_cl(&mut self, dst: Reg) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xD3);
        self.buf.push(0b1100_0000 | (4 << 3) | dst.lo3());
    }

    /// `sar dst, cl` (REX.W D3 /7) — arithmetic shift right.
    pub fn sar_cl(&mut self, dst: Reg) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xD3);
        self.buf.push(0b1100_0000 | (7 << 3) | dst.lo3());
    }

    /// `shr dst, cl` (REX.W D3 /5) — LOGICAL (unsigned) shift right (zero-fill).
    pub fn shr_cl(&mut self, dst: Reg) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xD3);
        self.buf.push(0b1100_0000 | (5 << 3) | dst.lo3());
    }

    /// `rol dst, cl` (`D3 /0`). A 32-bit destination write zero-extends.
    pub fn rol_cl(&mut self, dst: Reg, width32: bool) {
        self.rex(!width32, 0, 0, dst.hi());
        self.buf.push(0xD3);
        self.buf.push(0b1100_0000 | dst.lo3());
    }

    /// `ror dst, cl` (`D3 /1`). A 32-bit destination write zero-extends.
    pub fn ror_cl(&mut self, dst: Reg, width32: bool) {
        self.rex(!width32, 0, 0, dst.hi());
        self.buf.push(0xD3);
        self.buf.push(0b1100_0000 | (1 << 3) | dst.lo3());
    }

    /// `cmp a, b` (REX.W 39 /r — `a - b`, sets flags).
    pub fn cmp_rr(&mut self, a: Reg, b: Reg) {
        self.rex(true, b.hi(), 0, a.hi());
        self.buf.push(0x39);
        self.modrm_reg(b, a);
    }

    /// `cqo` (REX.W 99): sign-extend rax into rdx:rax (for idiv).
    pub fn cqo(&mut self) {
        self.buf.push(0x48);
        self.buf.push(0x99);
    }

    /// `idiv src` (REX.W F7 /7): rdx:rax / src → quotient rax, remainder rdx.
    pub fn idiv_r(&mut self, src: Reg) {
        self.rex(true, 0, 0, src.hi());
        self.buf.push(0xF7);
        self.buf.push(0b1100_0000 | (7 << 3) | src.lo3());
    }

    /// `mul src` (REX.W F7 /4): UNSIGNED rdx:rax = rax * src — the high 64 bits
    /// of the 128-bit product land in rdx, the low in rax. The magic-reciprocal
    /// high-multiply primitive.
    pub fn mul_r(&mut self, src: Reg) {
        self.rex(true, 0, 0, src.hi());
        self.buf.push(0xF7);
        self.buf.push(0b1100_0000 | (4 << 3) | src.lo3());
    }

    /// `shr dst, imm` (REX.W C1 /5 ib) — LOGICAL (zero-filling) shift right by a
    /// constant. Distinct from `sar` (`/7`, arithmetic/sign-filling): the
    /// unsigned magic reciprocal shifts unsigned high-product bits, so it must
    /// zero-fill.
    pub fn shr_ri(&mut self, dst: Reg, imm: u8) {
        self.rex(true, 0, 0, dst.hi());
        self.buf.push(0xC1);
        self.buf.push(0b1100_0000 | (5 << 3) | dst.lo3());
        self.buf.push(imm);
    }

    /// `test a, a` (REX.W 85 /r) — sets ZF when `a == 0`.
    pub fn test_rr(&mut self, a: Reg, b: Reg) {
        self.rex(true, b.hi(), 0, a.hi());
        self.buf.push(0x85);
        self.modrm_reg(b, a);
    }

    // ---------------------------------------------------------------
    // SSE2 scalar-double (f64) encodings for the XMM register class.
    //
    // Every scalar-double op is `<prefix> 0F <op> /r` where the prefix is F2
    // for the SD (scalar-double) arithmetic forms, 66 for the abs-mask/move
    // bit ops, F3 for sqrt's reciprocal sibling (we only need F2 0F 51 for
    // sqrtsd). The REX bits route the high registers (xmm8..xmm15). Memory
    // operands reuse the GP `modrm_mem` (base + disp32, SIB for rsp/r12) — the
    // ModRM `reg` field there is taken from a GP `Reg`, so for XMM memory ops
    // we hand-emit the ModRM with the XMM reg in the reg field.
    // ---------------------------------------------------------------

    /// REX for an SSE2 reg-reg op `xmm_r OP xmm_m` (reg field = `r`, rm = `m`).
    /// Emitted only when any high bit is set (the SSE2 forms have no mandatory
    /// REX.W); a spurious zero REX would be harmless but we keep bytes minimal.
    fn sse_rex_rr(&mut self, r: Xmm, m: Xmm) {
        if r.hi() != 0 || m.hi() != 0 {
            self.buf.push(0x40 | (r.hi() << 2) | m.hi());
        }
    }

    /// ModRM for register-direct between two XMM regs.
    fn modrm_xmm_rr(&mut self, r: Xmm, m: Xmm) {
        self.buf.push(0b1100_0000 | (r.lo3() << 3) | m.lo3());
    }

    /// A scalar-double reg-reg op: `<prefix> [REX] 0F <op> /r`, dst is the reg
    /// field (read-modify-write: `dst = dst OP src`).
    fn sse_sd_rr(&mut self, prefix: u8, op: u8, dst: Xmm, src: Xmm) {
        self.buf.push(prefix);
        self.sse_rex_rr(dst, src);
        self.buf.push(0x0F);
        self.buf.push(op);
        self.modrm_xmm_rr(dst, src);
    }

    /// A scalar-double reg-MEM op: `<prefix> [REX] 0F <op> /r` with a
    /// `[base+disp32]` memory operand and the XMM reg in the reg field.
    fn sse_sd_rm(&mut self, prefix: u8, op: u8, dst: Xmm, base: Reg, disp: i32) {
        self.buf.push(prefix);
        // REX: R = dst.hi(), B = base.hi().
        if dst.hi() != 0 || base.hi() != 0 {
            self.buf.push(0x40 | (dst.hi() << 2) | base.hi());
        }
        self.buf.push(0x0F);
        self.buf.push(op);
        let rm = base.lo3();
        self.buf.push(0b1000_0000 | (dst.lo3() << 3) | rm);
        if rm == 0b100 {
            self.buf.push(0b0000_0000 | (0b100 << 3) | rm);
        }
        self.buf.extend_from_slice(&disp.to_le_bytes());
    }

    /// `movsd xmm, [base+disp]` (F2 0F 10 /r) — load an f64 from the frame.
    pub fn movsd_rm(&mut self, dst: Xmm, base: Reg, disp: i32) {
        self.sse_sd_rm(0xF2, 0x10, dst, base, disp);
    }
    /// `movsd [base+disp], xmm` (F2 0F 11 /r) — store an f64 to the frame.
    pub fn movsd_mr(&mut self, base: Reg, disp: i32, src: Xmm) {
        self.sse_sd_rm(0xF2, 0x11, src, base, disp);
    }
    /// `movsd dst, src` (F2 0F 10 /r) — XMM→XMM scalar-double copy.
    pub fn movsd_rr(&mut self, dst: Xmm, src: Xmm) {
        if dst == src {
            return;
        }
        self.sse_sd_rr(0xF2, 0x10, dst, src);
    }
    /// `addsd dst, src` (F2 0F 58 /r) — `dst += src`.
    pub fn addsd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0xF2, 0x58, dst, src);
    }
    /// `subsd dst, src` (F2 0F 5C /r) — `dst -= src`.
    pub fn subsd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0xF2, 0x5C, dst, src);
    }
    /// `mulsd dst, src` (F2 0F 59 /r) — `dst *= src`.
    pub fn mulsd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0xF2, 0x59, dst, src);
    }
    /// `divsd dst, src` (F2 0F 5E /r) — `dst /= src`.
    pub fn divsd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0xF2, 0x5E, dst, src);
    }
    /// `sqrtsd dst, src` (F2 0F 51 /r) — `dst = sqrt(src)`.
    pub fn sqrtsd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0xF2, 0x51, dst, src);
    }

    // ---- Packed double (SIMD, 2-wide) ----------------------------------------
    // Each instruction below shares the scalar-double encoder helpers but swaps
    // the `F2` (scalar) prefix for `66` (packed). A packed op computes BOTH f64
    // lanes with the SAME IEEE-754 rounding as the scalar form, so each lane is
    // bit-identical to the corresponding scalar op — the soundness invariant that
    // makes auto-vectorization a LEGAL bit-exact lever (unlike FMA, which fuses
    // two roundings into one and is therefore NOT bit-exact).

    /// `movupd xmm, [base+disp]` (66 0F 10 /r) — load an UNALIGNED 128-bit pair.
    /// Unaligned (not `movapd`) because frame/array slots carry no 16-byte
    /// alignment guarantee; the unaligned form is penalty-free on modern cores
    /// when the access happens to be aligned.
    pub fn movupd_rm(&mut self, dst: Xmm, base: Reg, disp: i32) {
        self.sse_sd_rm(0x66, 0x10, dst, base, disp);
    }
    /// `movupd [base+disp], xmm` (66 0F 11 /r) — store an unaligned 128-bit pair.
    pub fn movupd_mr(&mut self, base: Reg, disp: i32, src: Xmm) {
        self.sse_sd_rm(0x66, 0x11, src, base, disp);
    }
    /// `movupd dst, src` (66 0F 10 /r) — XMM→XMM 128-bit copy (both lanes).
    pub fn movupd_rr(&mut self, dst: Xmm, src: Xmm) {
        if dst == src {
            return;
        }
        self.sse_sd_rr(0x66, 0x10, dst, src);
    }
    /// `addpd dst, src` (66 0F 58 /r) — `dst[lane] += src[lane]` for both lanes.
    pub fn addpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x58, dst, src);
    }
    /// `subpd dst, src` (66 0F 5C /r) — `dst[lane] -= src[lane]` for both lanes.
    pub fn subpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x5C, dst, src);
    }
    /// `mulpd dst, src` (66 0F 59 /r) — `dst[lane] *= src[lane]` for both lanes.
    pub fn mulpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x59, dst, src);
    }
    /// `cmppd dst, src, pred` (66 0F C2 /r ib) — per-lane compare; each lane is
    /// set to all-ones (true) or all-zero (false). `pred`: 0=EQ 1=LT 2=LE 3=UNORD
    /// 4=NEQ 5=NLT 6=NLE 7=ORD. The mandelbrot escape test uses LE (2).
    pub fn cmppd_rr(&mut self, dst: Xmm, src: Xmm, pred: u8) {
        self.buf.push(0x66);
        self.sse_rex_rr(dst, src);
        self.buf.push(0x0F);
        self.buf.push(0xC2);
        self.modrm_xmm_rr(dst, src);
        self.buf.push(pred);
    }
    /// `movmskpd r32/64, xmm` (66 0F 50 /r) — extract the two lane SIGN bits into
    /// a GP register (bit0=lane0, bit1=lane1). After a cmppd, a true lane's sign
    /// bit is 1, so this yields the active-lane bitmask the loop-exit test reads.
    /// The GP dst is the reg field, the XMM src the rm field.
    pub fn movmskpd(&mut self, dst: Reg, src: Xmm) {
        self.buf.push(0x66);
        let rex = 0x40 | (dst.hi() << 2) | src.hi();
        if rex != 0x40 {
            self.buf.push(rex);
        }
        self.buf.push(0x0F);
        self.buf.push(0x50);
        self.buf.push(0b1100_0000 | (dst.lo3() << 3) | src.lo3());
    }
    /// `andpd dst, src` (66 0F 54 /r) — bitwise AND of both 128-bit lanes. The
    /// branchless counter masks a packed 1.0 by the escape mask.
    pub fn andpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x54, dst, src);
    }
    /// `andnpd dst, src` (66 0F 55 /r) — `dst = (NOT dst) AND src` (128-bit).
    pub fn andnpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x55, dst, src);
    }
    /// `orpd dst, src` (66 0F 56 /r) — bitwise OR of both 128-bit lanes.
    pub fn orpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x56, dst, src);
    }
    /// `xorpd dst, src` (66 0F 57 /r) — bitwise XOR of both 128-bit lanes.
    pub fn xorpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x57, dst, src);
    }
    /// `divpd dst, src` (66 0F 5E /r) — `dst[lane] /= src[lane]` for both lanes.
    pub fn divpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x5E, dst, src);
    }
    /// `sqrtpd dst, src` (66 0F 51 /r) — `dst[lane] = sqrt(src[lane])` both lanes.
    pub fn sqrtpd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.sse_sd_rr(0x66, 0x51, dst, src);
    }
    /// `ucomisd a, b` (66 0F 2E /r) — unordered f64 compare setting CF/ZF/PF.
    /// NaN (unordered) sets ZF=CF=PF=1, so the seta/setae/jbe family used by the
    /// backend folds the unordered case to FALSE, matching the kernel's IEEE
    /// relations (NaN compares false).
    pub fn ucomisd_rr(&mut self, a: Xmm, b: Xmm) {
        self.buf.push(0x66);
        self.sse_rex_rr(a, b);
        self.buf.push(0x0F);
        self.buf.push(0x2E);
        self.modrm_xmm_rr(a, b);
    }
    /// `cvtsi2sd xmm, r64` (F2 REX.W 0F 2A /r) — signed i64 → f64 (the kernel's
    /// IntToFloat). The GP source is the rm field, the XMM dst the reg field.
    pub fn cvtsi2sd(&mut self, dst: Xmm, src: Reg) {
        self.buf.push(0xF2);
        // REX.W=1, R = dst.hi(), B = src.hi().
        self.buf.push(0x48 | (dst.hi() << 2) | src.hi());
        self.buf.push(0x0F);
        self.buf.push(0x2A);
        self.buf.push(0b1100_0000 | (dst.lo3() << 3) | src.lo3());
    }
    /// `roundsd dst, src, imm8` (66 0F 3A 0B /r ib) — round `src` to an integral f64 by `imm8` mode
    /// (bit0-1: 0=nearest-even, 1=floor/−∞, 2=ceil/+∞, 3=trunc/toward-0; bit3 set = suppress inexact).
    /// SSE4.1; both operands XMM (dst = reg field, src = rm field). Guard the lowering on the CPU feature.
    pub fn roundsd_rri(&mut self, dst: Xmm, src: Xmm, imm: u8) {
        self.buf.push(0x66);
        self.sse_rex_rr(dst, src);
        self.buf.push(0x0F);
        self.buf.push(0x3A);
        self.buf.push(0x0B);
        self.modrm_xmm_rr(dst, src);
        self.buf.push(imm);
    }
    /// `cvttsd2si r64, xmm` (F2 REX.W 0F 2C /r) — convert f64 → i64 by TRUNCATION toward zero (the `as
    /// i64` cast; saturating/out-of-range gives i64::MIN). GP dst = reg field, XMM src = rm field.
    pub fn cvttsd2si_rr(&mut self, dst: Reg, src: Xmm) {
        self.buf.push(0xF2);
        // REX.W=1, R = dst.hi() (GP reg field), B = src.hi() (XMM rm field).
        self.buf.push(0x48 | (dst.hi() << 2) | src.hi());
        self.buf.push(0x0F);
        self.buf.push(0x2C);
        self.buf.push(0b1100_0000 | (dst.lo3() << 3) | src.lo3());
    }
    /// `movq xmm, r64` (66 REX.W 0F 6E /r) — bit-copy a GP register into an XMM
    /// (no conversion). The GP src is the rm field, the XMM dst the reg field.
    pub fn movq_xr(&mut self, dst: Xmm, src: Reg) {
        self.buf.push(0x66);
        self.buf.push(0x48 | (dst.hi() << 2) | src.hi());
        self.buf.push(0x0F);
        self.buf.push(0x6E);
        self.buf.push(0b1100_0000 | (dst.lo3() << 3) | src.lo3());
    }
    /// `movq r64, xmm` (66 REX.W 0F 7E /r) — bit-copy an XMM into a GP register.
    /// The XMM src is the reg field, the GP dst the rm field.
    pub fn movq_rx(&mut self, dst: Reg, src: Xmm) {
        self.buf.push(0x66);
        self.buf.push(0x48 | (src.hi() << 2) | dst.hi());
        self.buf.push(0x0F);
        self.buf.push(0x7E);
        self.buf.push(0b1100_0000 | (src.lo3() << 3) | dst.lo3());
    }

    /// `movzx dst, byte [base + disp]` (REX.W 0F B6 /r) — load ONE byte from
    /// memory and zero-extend it into the 64-bit `dst`. The byte-array
    /// (`Seq of Bool`) element load: `frame[D] = buf[i-1] as i64` over 1-byte
    /// elements, where the loaded `u8` widens to a non-negative i64 (0..=255) —
    /// bit-identical to the `logos_stencil_arrldb` `*ptr as i64`.
    pub fn movzx_rm8(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.rex(true, dst.hi(), 0, base.hi());
        self.buf.push(0x0F);
        self.buf.push(0xB6);
        self.modrm_mem(dst, base, disp);
    }

    /// `mov byte [base + disp], src8` (88 /r) — store the LOW byte of `src` to
    /// memory. The byte-array element store; the value is pre-normalized to 0/1
    /// by the caller (matching `logos_stencil_arrstb`'s `(v != 0) as u8`), so
    /// only the low byte is written. A REX prefix is emitted whenever any high
    /// register bit is set OR `src` is one of `spl/bpl/sil/dil` (rsp..rdi,
    /// encodings 4..7) — those low-byte registers are addressable ONLY with a
    /// REX prefix present (without REX the encoding means `ah/ch/dh/bh`).
    pub fn mov_mr8(&mut self, base: Reg, disp: i32, src: Reg) {
        let need_rex = src.hi() != 0 || base.hi() != 0 || ((src as u8) & 0b100) != 0;
        if need_rex {
            self.buf.push(0x40 | (src.hi() << 2) | base.hi());
        }
        self.buf.push(0x88);
        self.modrm_mem(src, base, disp);
    }

    /// `movsxd dst, dword [base + disp]` (REX.W 63 /r) — load FOUR bytes from
    /// memory and SIGN-extend them into the 64-bit `dst`. The half-width
    /// (`IntsI32`) array element load: `frame[D] = buf[i-1] as i64` over 4-byte
    /// `i32` elements — bit-identical to `logos_stencil_arrld_i32`'s
    /// `*(i32*)ptr as i64`.
    pub fn movsxd_rm(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.rex(true, dst.hi(), 0, base.hi());
        self.buf.push(0x63);
        self.modrm_mem(dst, base, disp);
    }

    /// `mov dword [base + disp], src32` (89 /r, NO REX.W) — store the LOW 4 bytes
    /// of `src` to memory. The half-width (`IntsI32`) element store: the value is
    /// truncated to `i32` (lossless under the narrowing proof) — bit-identical to
    /// `logos_stencil_arrst_i32`'s `*(i32*)ptr = v as i32`. A REX prefix (without
    /// the W bit) is emitted only for the high-register extension bits.
    pub fn mov_mr32(&mut self, base: Reg, disp: i32, src: Reg) {
        if src.hi() != 0 || base.hi() != 0 {
            self.buf.push(0x40 | (src.hi() << 2) | base.hi());
        }
        self.buf.push(0x89);
        self.modrm_mem(src, base, disp);
    }

    /// `setcc dst8` — set the LOW byte of `dst` to 0/1 from the flags (no
    /// zero-extension of the upper bits). A REX prefix (even empty) makes
    /// `spl/bpl/sil/dil` and `r8b..r15b` addressable; emit REX with B = dst.hi().
    /// Used to normalize a byte-array store value to 0/1 in the same register
    /// whose low byte is then stored by [`Asm::mov_mr8`].
    pub fn setcc8(&mut self, cond: Cond, dst: Reg) {
        self.buf.push(0x40 | dst.hi());
        self.buf.push(0x0F);
        self.buf.push(0x90 | cond.cc());
        self.buf.push(0b1100_0000 | dst.lo3());
    }

    /// `setcc dst8` then `movzx dst, dst8` — materialize a 0/1 from flags.
    /// `dst` must be a register whose low byte is addressable under REX
    /// (all of rax..r15 are with a REX prefix).
    pub fn setcc_movzx(&mut self, cond: Cond, dst: Reg) {
        // setcc r/m8: 0F (90+cc) /0. A REX prefix (even empty) makes spl/bpl/
        // sil/dil and r8b..r15b addressable; emit REX with B = dst.hi().
        self.buf.push(0x40 | dst.hi());
        self.buf.push(0x0F);
        self.buf.push(0x90 | cond.cc());
        self.buf.push(0b1100_0000 | dst.lo3());
        // movzx dst, dst8 (REX.W 0F B6 /r).
        self.rex(true, dst.hi(), 0, dst.hi());
        self.buf.push(0x0F);
        self.buf.push(0xB6);
        self.modrm_reg(dst, dst);
    }

    /// `jmp label` (E9 cd, rel32 patched in `resolve`).
    pub fn jmp(&mut self, label: LabelId) {
        self.buf.push(0xE9);
        let site = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.fixups.push(Fixup::Rel32 { site, label });
    }

    /// `jcc label` (0F 80+cc cd, rel32 patched in `resolve`).
    pub fn jcc(&mut self, cond: Cond, label: LabelId) {
        self.buf.push(0x0F);
        self.buf.push(0x80 | cond.cc());
        let site = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.fixups.push(Fixup::Rel32 { site, label });
    }

    /// `push reg` (50+rd, with REX.B for r8..r15).
    pub fn push(&mut self, r: Reg) {
        if r.hi() == 1 {
            self.buf.push(0x41);
        }
        self.buf.push(0x50 + r.lo3());
    }

    /// `pop reg` (58+rd, with REX.B for r8..r15).
    pub fn pop(&mut self, r: Reg) {
        if r.hi() == 1 {
            self.buf.push(0x41);
        }
        self.buf.push(0x58 + r.lo3());
    }

    /// `ret` (C3).
    pub fn ret(&mut self) {
        self.buf.push(0xC3);
    }

    /// Resolve all label fixups against bound positions and return the final
    /// machine code. Panics if a referenced label was never bound (a backend
    /// bug — every label this module creates is bound before `resolve`).
    pub fn resolve(mut self) -> Vec<u8> {
        for f in &self.fixups {
            match *f {
                Fixup::Rel32 { site, label } => {
                    let target = self.label_pos[label.0];
                    assert_ne!(target, usize::MAX, "unbound label {label:?}");
                    let rel = (target as i64) - (site as i64 + 4);
                    let rel32 = rel as i32;
                    self.buf[site..site + 4].copy_from_slice(&rel32.to_le_bytes());
                }
            }
        }
        self.buf
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_forge/x64asm/tests.rs"]
mod tests;
