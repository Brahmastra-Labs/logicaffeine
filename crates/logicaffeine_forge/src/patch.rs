//! Relocation patchers — pure byte math, compiled (and unit-tested) on EVERY
//! host so the x86-64 patchers are exercised on an arm64 dev machine and vice
//! versa. Each function rewrites one relocation site in a staging buffer given
//! the site's final runtime address and its target's address.

use crate::stencil_model::RelocKind;

/// Patch errors carry enough context to identify the failing site.
#[derive(Debug, PartialEq, Eq)]
pub enum PatchError {
    /// Branch/load displacement does not fit the encoding.
    OutOfRange {
        /// Relocation kind name.
        kind: &'static str,
        /// Address of the patch site.
        site: u64,
        /// Address the site must reach.
        target: u64,
    },
    /// arm64 targets must be 4-byte aligned for branches.
    Misaligned {
        /// Relocation kind name.
        kind: &'static str,
        /// The misaligned target address.
        target: u64,
    },
    /// The value is not aligned for the access size encoded at the site.
    BadScale {
        /// Relocation kind name.
        kind: &'static str,
        /// The target address.
        target: u64,
        /// Access-size shift encoded at the site.
        scale: u8,
    },
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::OutOfRange { kind, site, target } => {
                write!(
                    f,
                    "{kind}: target {target:#x} out of range of site {site:#x}"
                )
            }
            PatchError::Misaligned { kind, target } => {
                write!(f, "{kind}: target {target:#x} is not 4-byte aligned")
            }
            PatchError::BadScale {
                kind,
                target,
                scale,
            } => {
                write!(
                    f,
                    "{kind}: target {target:#x} not aligned for scale {scale}"
                )
            }
        }
    }
}

impl std::error::Error for PatchError {}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// aarch64 `b`/`bl` imm26: PC-relative, ±128 MiB, 4-byte units. Preserves the
/// opcode bits, rewriting only the immediate.
pub fn patch_aarch64_branch26(
    buf: &mut [u8],
    off: usize,
    site_addr: u64,
    target: u64,
) -> Result<(), PatchError> {
    if target % 4 != 0 {
        return Err(PatchError::Misaligned {
            kind: "branch26",
            target,
        });
    }
    let delta = (target as i64).wrapping_sub(site_addr as i64);
    if !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(PatchError::OutOfRange {
            kind: "branch26",
            site: site_addr,
            target,
        });
    }
    let imm26 = ((delta >> 2) as u32) & 0x03FF_FFFF;
    let insn = read_u32(buf, off);
    write_u32(buf, off, (insn & 0xFC00_0000) | imm26);
    Ok(())
}

/// aarch64 `adrp` hi21: the PAGE delta (4 KiB pages), split immlo\[30:29\] /
/// immhi\[23:5\]. Range ±4 GiB.
pub fn patch_aarch64_page21(
    buf: &mut [u8],
    off: usize,
    site_addr: u64,
    target: u64,
) -> Result<(), PatchError> {
    let page_delta = ((target as i64) >> 12).wrapping_sub((site_addr as i64) >> 12);
    if !(-(1 << 20)..(1 << 20)).contains(&page_delta) {
        return Err(PatchError::OutOfRange {
            kind: "page21",
            site: site_addr,
            target,
        });
    }
    let imm = page_delta as u32;
    let immlo = imm & 0b11;
    let immhi = (imm >> 2) & 0x7FFFF;
    let insn = read_u32(buf, off);
    let cleared = insn & !((0b11 << 29) | (0x7FFFF << 5));
    write_u32(buf, off, cleared | (immlo << 29) | (immhi << 5));
    Ok(())
}

/// aarch64 `add`/`ldr` lo12: the low 12 bits of the target, shifted right by
/// the access-size scale for loads/stores.
pub fn patch_aarch64_pageoff12(
    buf: &mut [u8],
    off: usize,
    target: u64,
    scale: u8,
) -> Result<(), PatchError> {
    let lo12 = (target & 0xFFF) as u32;
    if scale > 0 && lo12 & ((1 << scale) - 1) != 0 {
        return Err(PatchError::BadScale {
            kind: "pageoff12",
            target,
            scale,
        });
    }
    let imm12 = lo12 >> scale;
    let insn = read_u32(buf, off);
    let cleared = insn & !(0xFFF << 10);
    write_u32(buf, off, cleared | (imm12 << 10));
    Ok(())
}

/// x86-64 rip-relative disp32 (`jmp`/`call`/`lea`/`mov`):
/// `disp = target + addend − field_address`, with the rip-after-displacement
/// distance already NORMALIZED into the addend at extraction time (ELF bakes
/// it in natively; build.rs adjusts Mach-O and COFF to match).
pub fn patch_x64_rel32(
    buf: &mut [u8],
    off: usize,
    site_addr: u64,
    target: u64,
    addend: i64,
) -> Result<(), PatchError> {
    let delta = (target as i64)
        .wrapping_add(addend)
        .wrapping_sub(site_addr as i64);
    let disp = i32::try_from(delta).map_err(|_| PatchError::OutOfRange {
        kind: "rel32",
        site: site_addr,
        target,
    })?;
    buf[off..off + 4].copy_from_slice(&disp.to_le_bytes());
    Ok(())
}

/// 8-byte absolute address, little-endian.
pub fn patch_abs64(buf: &mut [u8], off: usize, target: u64, addend: i64) {
    let v = (target as i64).wrapping_add(addend) as u64;
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// Whether this reloc kind resolves through a pointer slot (GOT/IAT/refptr) —
/// the buffer layout must provide one and point the site at it.
pub fn is_indirect(kind: RelocKind) -> bool {
    matches!(
        kind,
        RelocKind::GotPage21 | RelocKind::GotPageOff12 | RelocKind::GotRel32
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_forge/patch/tests.rs"]
mod tests;
