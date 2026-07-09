//! The stencil data model — shared between build.rs (which CONSTRUCTS these
//! from parsed object files) and the runtime (which copies and patches them).
//! Included via `include!` from both sides, so it must stay dependency-free.

/// One extracted stencil: machine code plus the holes to patch.
#[derive(Debug)]
pub struct Stencil {
    /// The stencil's symbol name in the object file.
    pub name: &'static str,
    /// The extracted machine code.
    pub code: &'static [u8],
    /// Every patchable site within `code`.
    pub relocs: &'static [Reloc],
}

/// One patchable site inside a stencil's code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reloc {
    /// Byte offset of the relocation site within `code`.
    pub offset: u32,
    /// The normalized relocation kind (decides the patcher).
    pub kind: RelocKind,
    /// Which hole this site resolves.
    pub target: HoleId,
    /// Constant folded into the patched value (`S + A - P` convention).
    pub addend: i64,
}

/// Normalized relocation kinds across Mach-O / ELF / COFF, for the two
/// architectures the JIT targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocKind {
    /// aarch64 `b`/`bl` imm26 (±128 MiB, 4-byte aligned).
    Branch26,
    /// aarch64 `adrp` hi21 page, direct.
    Page21,
    /// aarch64 `add`/`ldr` lo12, direct. `scale` is the access-size shift the
    /// patcher must apply (0 for `add`, 3 for 8-byte `ldr`).
    PageOff12 {
        /// Access-size shift the patcher applies (0 for `add`, 3 for 8-byte `ldr`).
        scale: u8,
    },
    /// aarch64 `adrp` hi21 page → GOT slot.
    GotPage21,
    /// aarch64 `ldr` lo12 → GOT slot (8-byte scaled).
    GotPageOff12,
    /// x86-64 rip-relative disp32 (`jmp`/`call`/`lea`/`mov`).
    Rel32,
    /// x86-64 rip-relative disp32 → GOT/IAT slot.
    GotRel32,
    /// 8-byte absolute address, little-endian.
    Abs64,
}

/// Which hole a relocation targets, decoded from the hole symbol's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoleId {
    /// `logos_hole_cont_N` — a continuation (branch target).
    Cont(u8),
    /// `LOGOS_HOLE_I64_N` — a 64-bit constant.
    ConstI64(u8),
}
