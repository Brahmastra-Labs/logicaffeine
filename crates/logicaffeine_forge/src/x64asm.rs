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
    /// UNSIGNED below: `cc=0x2` (`jb`). The strict ordered-less primitive used
    /// by the epsilon equality (`|a-b| < EPSILON` via `ucomisd EPS, |a-b|` then
    /// `setb`/`jb`); NaN folds to FALSE.
    BU,
    /// PARITY EVEN (`jp`/`jpe`): `cc=0xA`. After `ucomisd`, PF=1 ⟺ the operands
    /// were UNORDERED (a NaN). The `DivF` zero-divisor guard uses this to skip
    /// the side-exit on a NaN divisor (NaN is not `0.0`).
    ParityEven,
}

impl Cond {
    /// The 4-bit x86 condition-code tetrad (`jcc` low nibble = `0x80 | cc`,
    /// `setcc` = `0x90 | cc`).
    fn cc(self) -> u8 {
        match self {
            Cond::Eq => 0x4,
            Cond::Ne => 0x5,
            Cond::Ge => 0xD, // GE (signed): NL
            Cond::Lt => 0xC, // L  (signed)
            Cond::Le => 0xE, // LE (signed)
            Cond::Gt => 0xF, // G  (signed): NLE
            Cond::AeU => 0x3, // AE (unsigned): NB/NC
            Cond::AU => 0x7,  // A  (unsigned): NBE
            Cond::BeU => 0x6, // BE (unsigned): NA
            Cond::BU => 0x2,  // B  (unsigned): C
            Cond::ParityEven => 0xA, // P/PE (parity even): unordered after ucomisd
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
        Asm { buf: Vec::with_capacity(256), label_pos: Vec::new(), fixups: Vec::new() }
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
mod tests {
    use super::*;
    use crate::JitPage;

    /// Build a tiny function with the test ABI `fn(i64, i64) -> i64` and run it.
    fn run2(code: &[u8], a: i64, b: i64) -> i64 {
        let page = JitPage::new(code).expect("map");
        let f = unsafe { page.as_fn_i64_i64() };
        f(a, b)
    }

    #[test]
    fn mov_imm_and_ret_returns_constant() {
        let mut a = Asm::new();
        a.mov_ri(Reg::Rax, 0x1234_5678_9ABC_DEF0u64 as i64);
        a.ret();
        assert_eq!(run2(&a.resolve(), 0, 0), 0x1234_5678_9ABC_DEF0u64 as i64);
    }

    #[test]
    fn add_two_args_via_sysv() {
        // SysV: arg0 = rdi, arg1 = rsi, return rax.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.add_rr(Reg::Rax, Reg::Rsi);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 40, 2), 42);
        assert_eq!(run2(&code, -5, 5), 0);
    }

    #[test]
    fn imul_and_extended_registers() {
        let mut a = Asm::new();
        a.mov_rr(Reg::R10, Reg::Rdi);
        a.imul_rr(Reg::R10, Reg::Rsi);
        a.mov_rr(Reg::Rax, Reg::R10);
        a.ret();
        assert_eq!(run2(&a.resolve(), 6, 7), 42);
    }

    #[test]
    fn setcc_materializes_zero_one() {
        // return (rdi < rsi) as i64
        let mut a = Asm::new();
        a.cmp_rr(Reg::Rdi, Reg::Rsi);
        a.setcc_movzx(Cond::Lt, Reg::Rax);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 1, 2), 1);
        assert_eq!(run2(&code, 2, 1), 0);
        assert_eq!(run2(&code, 2, 2), 0);
    }

    #[test]
    fn conditional_jump_branches() {
        // if rdi < rsi { return 100 } else { return 200 }
        let mut a = Asm::new();
        let els = a.new_label();
        a.cmp_rr(Reg::Rdi, Reg::Rsi);
        a.jcc(Cond::Ge, els); // not (rdi<rsi) -> else
        a.mov_ri(Reg::Rax, 100);
        a.ret();
        a.bind(els);
        a.mov_ri(Reg::Rax, 200);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 1, 2), 100);
        assert_eq!(run2(&code, 2, 1), 200);
    }

    #[test]
    fn frame_load_store_roundtrip() {
        // treat rdi as a base pointer to an i64 array; store rsi at [rdi+8],
        // load it back into rax.
        let mut a = Asm::new();
        a.mov_mr(Reg::Rdi, 8, Reg::Rsi);
        a.mov_rm(Reg::Rax, Reg::Rdi, 8);
        a.ret();
        let mut frame = [0i64; 4];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut i64, i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        let r = f(frame.as_mut_ptr(), 777);
        assert_eq!(r, 777);
        assert_eq!(frame[1], 777);
    }

    #[test]
    fn idiv_signed_division() {
        // return rdi / rsi
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.cqo();
        a.idiv_r(Reg::Rsi);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 100, 7), 14);
        assert_eq!(run2(&code, -100, 7), -14);
    }

    #[test]
    fn mul_r_high_product_unsigned() {
        // return the HIGH 64 bits of the unsigned product rdi * rsi (rdx after
        // `mul`). The classic case that distinguishes unsigned `mul` from signed
        // `imul`: with rdi = u64::MAX (== -1 as i64), the unsigned high half is
        // rsi - 1, NOT the signed -1.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.mul_r(Reg::Rsi); // rdx:rax = rax * rsi (unsigned)
        a.mov_rr(Reg::Rax, Reg::Rdx); // return the high half
        a.ret();
        let code = a.resolve();
        // (2^64 - 1) * 3 = 3*2^64 - 3 → high half = 2 (since low half borrows).
        assert_eq!(run2(&code, -1i64, 3) as u64, 2);
        // 2^32 * 2^32 = 2^64 → high half = 1.
        assert_eq!(run2(&code, 1i64 << 32, 1i64 << 32) as u64, 1);
        // Small product: high half is 0.
        assert_eq!(run2(&code, 123, 456) as u64, 0);
    }

    #[test]
    fn shr_ri_is_logical_not_arithmetic() {
        // `shr` zero-fills (logical); `sar` sign-fills. With rdi = -1 (all bits
        // set) shifted right by 1, logical gives i64::MAX, arithmetic gives -1.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.shr_ri(Reg::Rax, 1);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, -1i64, 0), i64::MAX);
        assert_eq!(run2(&code, 1024, 0), 512);
        // Shift by a larger amount.
        let mut b = Asm::new();
        b.mov_rr(Reg::Rax, Reg::Rdi);
        b.shr_ri(Reg::Rax, 60);
        b.ret();
        assert_eq!(run2(&b.resolve(), -1i64, 0) as u64, 0xF);
    }

    #[test]
    fn rsp_base_needs_sib() {
        // Exercise the SIB path: use rsp-relative store would clobber the
        // stack, so instead verify r12 (also lo3==100) routes through SIB.
        let mut a = Asm::new();
        a.mov_rr(Reg::R12, Reg::Rdi);
        a.mov_mr(Reg::R12, 16, Reg::Rsi);
        a.mov_rm(Reg::Rax, Reg::R12, 16);
        a.ret();
        let mut frame = [0i64; 8];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut i64, i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        let r = f(frame.as_mut_ptr(), 555);
        assert_eq!(r, 555);
        assert_eq!(frame[2], 555);
    }

    #[test]
    fn shift_and_bitwise() {
        // return (rdi << rcx_amount) where amount is in rsi; use cl.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.mov_rr(Reg::Rcx, Reg::Rsi);
        a.shl_cl(Reg::Rax);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 1, 4), 16);
        assert_eq!(run2(&code, 3, 2), 12);
    }

    #[test]
    fn sub_immediate_and_shl_immediate() {
        // return ((rdi - 1) << 3)  — the array byte-offset computation.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.sub_ri(Reg::Rax, 1);
        a.shl_ri(Reg::Rax, 3);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 1, 0), 0); // (1-1)*8
        assert_eq!(run2(&code, 5, 0), 32); // (5-1)*8
        assert_eq!(run2(&code, 0, 0), -8); // (0-1)*8 wraps
    }

    #[test]
    fn unsigned_above_equal_branch() {
        // if (rdi as u64) >= (rsi as u64) { 1 } else { 0 } — the bounds guard.
        let mut a = Asm::new();
        let oob = a.new_label();
        a.cmp_rr(Reg::Rdi, Reg::Rsi);
        a.jcc(Cond::AeU, oob);
        a.mov_ri(Reg::Rax, 0);
        a.ret();
        a.bind(oob);
        a.mov_ri(Reg::Rax, 1);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 3, 4), 0); // 3 < 4
        assert_eq!(run2(&code, 4, 4), 1); // 4 >= 4
        assert_eq!(run2(&code, -1, 4), 1); // (-1) as u64 huge >= 4 (the i=0 case)
    }

    /// Run a frame-ABI function `fn(*mut i64) -> i64` over a mutable frame.
    fn run_frame(code: &[u8], frame: &mut [i64]) -> i64 {
        let page = JitPage::new(code).unwrap();
        let f: extern "C" fn(*mut i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        f(frame.as_mut_ptr())
    }

    #[test]
    fn sse_arithmetic_matches_ieee() {
        // frame: 0=a 1=b ; compute ((a+b)*a - b) / a and return its bits.
        for (a, b) in [(3.0f64, 4.0f64), (1.5, -2.25), (0.1, 0.2), (-7.0, 11.0)] {
            let mut a_asm = Asm::new();
            a_asm.movsd_rm(Xmm::Xmm0, Reg::Rdi, 0); // x0 = a
            a_asm.movsd_rm(Xmm::Xmm1, Reg::Rdi, 8); // x1 = b
            a_asm.movsd_rr(Xmm::Xmm2, Xmm::Xmm0); // x2 = a
            a_asm.addsd_rr(Xmm::Xmm2, Xmm::Xmm1); // x2 = a+b
            a_asm.mulsd_rr(Xmm::Xmm2, Xmm::Xmm0); // x2 = (a+b)*a
            a_asm.subsd_rr(Xmm::Xmm2, Xmm::Xmm1); // x2 = (a+b)*a - b
            a_asm.divsd_rr(Xmm::Xmm2, Xmm::Xmm0); // x2 = .. / a
            a_asm.movq_rx(Reg::Rax, Xmm::Xmm2);
            a_asm.ret();
            let mut frame = [a.to_bits() as i64, b.to_bits() as i64];
            let got = f64::from_bits(run_frame(&a_asm.resolve(), &mut frame) as u64);
            let want = ((a + b) * a - b) / a;
            assert_eq!(got.to_bits(), want.to_bits(), "a={a} b={b}");
        }
    }

    #[test]
    fn sse_sqrt_and_cvtsi2sd() {
        // frame: 0=n(int) ; return bits of sqrt((f64)n).
        for n in [0i64, 1, 2, 4, 9, 1_000_000, 123_456_789] {
            let mut a = Asm::new();
            a.mov_rm(Reg::Rax, Reg::Rdi, 0);
            a.cvtsi2sd(Xmm::Xmm0, Reg::Rax);
            a.sqrtsd_rr(Xmm::Xmm1, Xmm::Xmm0);
            a.movq_rx(Reg::Rax, Xmm::Xmm1);
            a.ret();
            let mut frame = [n];
            let got = f64::from_bits(run_frame(&a.resolve(), &mut frame) as u64);
            assert_eq!(got.to_bits(), (n as f64).sqrt().to_bits(), "n={n}");
        }
    }

    #[test]
    fn sse_high_registers_route_rex() {
        // Exercise xmm8..xmm15 (the high-register REX path): x8 = a; x9 = b;
        // x8 += x9; return bits.
        for (a, b) in [(2.5f64, 0.5f64), (-1.0, 3.0)] {
            let mut asm = Asm::new();
            asm.movsd_rm(Xmm::Xmm8, Reg::Rdi, 0);
            asm.movsd_rm(Xmm::Xmm9, Reg::Rdi, 8);
            asm.addsd_rr(Xmm::Xmm8, Xmm::Xmm9);
            asm.movsd_mr(Reg::Rdi, 16, Xmm::Xmm8);
            asm.mov_rm(Reg::Rax, Reg::Rdi, 16);
            asm.ret();
            let mut frame = [a.to_bits() as i64, b.to_bits() as i64, 0];
            let got = f64::from_bits(run_frame(&asm.resolve(), &mut frame) as u64);
            assert_eq!(got.to_bits(), (a + b).to_bits(), "a={a} b={b}");
        }
    }

    #[test]
    fn sse_ucomisd_ordering_and_nan() {
        // return (a > b) as i64 via ucomisd + seta (Cond::AU). NaN -> 0.
        let nan = f64::NAN;
        for (a, b, want) in [
            (2.0f64, 1.0, 1i64),
            (1.0, 2.0, 0),
            (3.0, 3.0, 0),
            (nan, 1.0, 0),
            (1.0, nan, 0),
            (nan, nan, 0),
        ] {
            let mut asm = Asm::new();
            asm.movsd_rm(Xmm::Xmm0, Reg::Rdi, 0);
            asm.movsd_rm(Xmm::Xmm1, Reg::Rdi, 8);
            asm.ucomisd_rr(Xmm::Xmm0, Xmm::Xmm1); // compare a, b
            asm.setcc_movzx(Cond::AU, Reg::Rax); // a > b (ordered)
            asm.ret();
            let mut frame = [a.to_bits() as i64, b.to_bits() as i64];
            assert_eq!(run_frame(&asm.resolve(), &mut frame), want, "a={a} b={b}");
        }
    }

    #[test]
    fn movq_bridges_gp_and_xmm_bit_exact() {
        // GP -> XMM -> GP round-trips the exact bit pattern (incl. NaN/-0.0).
        for v in [0.0f64, -0.0, f64::NAN, f64::INFINITY, 1.5, -1e300] {
            let mut asm = Asm::new();
            asm.mov_rm(Reg::Rax, Reg::Rdi, 0);
            asm.movq_xr(Xmm::Xmm3, Reg::Rax);
            asm.movq_rx(Reg::Rax, Xmm::Xmm3);
            asm.ret();
            let mut frame = [v.to_bits() as i64];
            assert_eq!(run_frame(&asm.resolve(), &mut frame) as u64, v.to_bits(), "v={v:?}");
        }
    }

    #[test]
    fn add_cmp_immediate() {
        // return (rdi + 7) then verify cmp_ri sets flags: if (rax cmp 10) >= use jge.
        let mut a = Asm::new();
        a.mov_rr(Reg::Rax, Reg::Rdi);
        a.add_ri(Reg::Rax, 7);
        a.ret();
        let code = a.resolve();
        assert_eq!(run2(&code, 35, 0), 42);
        assert_eq!(run2(&code, -7, 0), 0);

        // cmp_ri: return (rdi >= 100) as i64.
        let mut b = Asm::new();
        b.cmp_ri(Reg::Rdi, 100);
        b.setcc_movzx(Cond::Ge, Reg::Rax);
        b.ret();
        let codeb = b.resolve();
        assert_eq!(run2(&codeb, 100, 0), 1);
        assert_eq!(run2(&codeb, 99, 0), 0);
        assert_eq!(run2(&codeb, 200, 0), 1);
    }

    #[test]
    fn indirect_call_through_register() {
        // Build a callee that returns rdi*3, then a caller that loads the callee
        // address into r11 and `call`s it, returning the result. Verifies the
        // FF /2 (call r64) encoding including the REX.B path (r11).
        let mut callee = Asm::new();
        callee.mov_rr(Reg::Rax, Reg::Rdi);
        callee.add_rr(Reg::Rax, Reg::Rdi);
        callee.add_rr(Reg::Rax, Reg::Rdi); // rax = 3*rdi
        callee.ret();
        let callee_page = JitPage::new(&callee.resolve()).expect("map callee");
        let callee_addr = callee_page.as_ptr() as i64;

        let mut caller = Asm::new();
        // Keep the stack 16-aligned at the call: entry rsp ≡ 8 (mod 16); one
        // `sub rsp, 8` makes it 16-aligned so the callee's `ret` lands clean.
        caller.sub_ri(Reg::Rsp, 8);
        caller.mov_ri(Reg::R11, callee_addr);
        caller.call_r(Reg::R11); // rax = 3 * rdi (rdi survives: caller-passed arg)
        caller.add_ri(Reg::Rsp, 8);
        caller.ret();
        let code = caller.resolve();
        assert_eq!(run2(&code, 14, 0), 42);
        assert_eq!(run2(&code, -5, 0), -15);
    }

    #[test]
    fn byte_load_zero_extends() {
        // base = rdi (u8*); load byte [rdi + (rsi-1)] zero-extended into rax.
        // A SIGN-extending load of 0xFF would give -1; movzx gives 255.
        let mut a = Asm::new();
        a.mov_rr(Reg::R10, Reg::Rsi);
        a.sub_ri(Reg::R10, 1); // im1
        a.add_rr(Reg::R10, Reg::Rdi); // addr = base + im1
        a.movzx_rm8(Reg::Rax, Reg::R10, 0);
        a.ret();
        let buf = [0u8, 1, 0xFF, 0x80, 7];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*const u8, i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        assert_eq!(f(buf.as_ptr(), 1), 0);
        assert_eq!(f(buf.as_ptr(), 2), 1);
        assert_eq!(f(buf.as_ptr(), 3), 255); // zero-extended, not -1
        assert_eq!(f(buf.as_ptr(), 4), 128);
        assert_eq!(f(buf.as_ptr(), 5), 7);
    }

    #[test]
    fn byte_store_low_byte_only() {
        // base = rdi (u8*); store the low byte of a normalized value (rsi != 0)
        // into [rdi]. Exercises setcc8 + mov_mr8. The value 0x1234_5678_FF00_0000
        // is nonzero, so setne → 1 → stored byte is 1, NOT 0 (its low byte).
        let mut a = Asm::new();
        a.test_rr(Reg::Rsi, Reg::Rsi);
        a.setcc8(Cond::Ne, Reg::Rdx);
        a.mov_mr8(Reg::Rdi, 0, Reg::Rdx);
        a.mov_ri(Reg::Rax, 0);
        a.ret();
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut u8, i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        for (val, want) in [(0i64, 0u8), (1, 1), (256, 1), (-1, 1), (0xFF00_0000i64, 1)] {
            let mut cell = [0xAAu8];
            f(cell.as_mut_ptr(), val);
            assert_eq!(cell[0], want, "val={val}");
        }
    }

    #[test]
    fn byte_store_through_rsp_class_base() {
        // Exercise mov_mr8's SIB path (base r12, lo3 == 100) and a src whose
        // low byte needs the REX prefix only via the high bit (r8b).
        let mut a = Asm::new();
        a.mov_rr(Reg::R12, Reg::Rdi);
        a.mov_ri(Reg::R8, 1);
        a.mov_mr8(Reg::R12, 0, Reg::R8);
        a.mov_ri(Reg::Rax, 0);
        a.ret();
        let mut cell = [0u8, 9];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut u8) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        f(cell.as_mut_ptr());
        assert_eq!(cell[0], 1);
        assert_eq!(cell[1], 9, "neighbor byte must be untouched (1-byte store)");
    }

    #[test]
    fn byte_store_spl_class_register() {
        // mov_mr8 with src = rsi (encoding 6, low byte `sil`): without a REX
        // prefix this would encode `dh`, so the need_rex path is mandatory.
        let mut a = Asm::new();
        a.mov_ri(Reg::Rsi, 0x77);
        a.mov_mr8(Reg::Rdi, 0, Reg::Rsi);
        a.mov_ri(Reg::Rax, 0);
        a.ret();
        let mut cell = [0u8];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut u8) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        f(cell.as_mut_ptr());
        assert_eq!(cell[0], 0x77);
    }

    #[test]
    fn computed_address_load_store() {
        // base = rdi (i64*); compute addr = base + (rsi-1)*8 in a scratch reg,
        // store 0xABCD there, then load it back — mirrors the array path.
        let mut a = Asm::new();
        a.mov_rr(Reg::R10, Reg::Rsi); // im = idx
        a.sub_ri(Reg::R10, 1); // im1 = idx - 1
        a.shl_ri(Reg::R10, 3); // byte offset
        a.add_rr(Reg::R10, Reg::Rdi); // addr = base + offset
        a.mov_ri(Reg::R11, 0xABCD);
        a.mov_mr(Reg::R10, 0, Reg::R11); // [addr] = 0xABCD
        a.mov_rm(Reg::Rax, Reg::R10, 0); // rax = [addr]
        a.ret();
        let mut frame = [0i64; 8];
        let page = JitPage::new(&a.resolve()).unwrap();
        let f: extern "C" fn(*mut i64, i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        let r = f(frame.as_mut_ptr(), 3); // index 3 (1-based) → frame[2]
        assert_eq!(r, 0xABCD);
        assert_eq!(frame[2], 0xABCD);
    }
}
