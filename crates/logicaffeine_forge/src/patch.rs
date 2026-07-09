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
                write!(f, "{kind}: target {target:#x} out of range of site {site:#x}")
            }
            PatchError::Misaligned { kind, target } => {
                write!(f, "{kind}: target {target:#x} is not 4-byte aligned")
            }
            PatchError::BadScale { kind, target, scale } => {
                write!(f, "{kind}: target {target:#x} not aligned for scale {scale}")
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
        return Err(PatchError::Misaligned { kind: "branch26", target });
    }
    let delta = (target as i64).wrapping_sub(site_addr as i64);
    if !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(PatchError::OutOfRange { kind: "branch26", site: site_addr, target });
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
        return Err(PatchError::OutOfRange { kind: "page21", site: site_addr, target });
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
        return Err(PatchError::BadScale { kind: "pageoff12", target, scale });
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
    let delta = (target as i64).wrapping_add(addend).wrapping_sub(site_addr as i64);
    let disp = i32::try_from(delta)
        .map_err(|_| PatchError::OutOfRange { kind: "rel32", site: site_addr, target })?;
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
    matches!(kind, RelocKind::GotPage21 | RelocKind::GotPageOff12 | RelocKind::GotRel32)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- aarch64 branch26: hand-assembled expectations from the ARM ARM ----

    #[test]
    fn b26_forward_8_bytes_encodes_0x14000002() {
        // `b +8` at address 0: imm26 = 8/4 = 2 → 0x14000002.
        let mut buf = vec![0x00, 0x00, 0x00, 0x14];
        patch_aarch64_branch26(&mut buf, 0, 0, 8).unwrap();
        assert_eq!(buf, vec![0x02, 0x00, 0x00, 0x14]);
    }

    #[test]
    fn b26_backward_4_bytes_encodes_0x17ffffff() {
        // `b -4`: imm26 = -1 → 0x17FFFFFF.
        let mut buf = vec![0x00, 0x00, 0x00, 0x14];
        patch_aarch64_branch26(&mut buf, 0, 4, 0).unwrap();
        assert_eq!(buf, vec![0xFF, 0xFF, 0xFF, 0x17]);
    }

    #[test]
    fn b26_preserves_opcode_bits_including_bl() {
        // A BL site keeps its BL opcode (0x94000000 family).
        let mut buf = vec![0x00, 0x00, 0x00, 0x94];
        patch_aarch64_branch26(&mut buf, 0, 0x1000, 0x1010).unwrap();
        assert_eq!(buf, vec![0x04, 0x00, 0x00, 0x94]);
    }

    #[test]
    fn b26_range_and_alignment_errors() {
        let mut buf = vec![0u8; 4];
        // Beyond ±128 MiB.
        assert!(matches!(
            patch_aarch64_branch26(&mut buf, 0, 0, 1 << 28),
            Err(PatchError::OutOfRange { .. })
        ));
        // At the exact positive edge: 2^27 - 4 is in range.
        assert!(patch_aarch64_branch26(&mut buf, 0, 0, (1 << 27) - 4).is_ok());
        // Misaligned target.
        assert!(matches!(
            patch_aarch64_branch26(&mut buf, 0, 0, 6),
            Err(PatchError::Misaligned { .. })
        ));
    }

    // ---- aarch64 adrp ----

    #[test]
    fn adrp_same_page_encodes_zero_imm() {
        // adrp x0, . → immlo=0 immhi=0; base opcode 0x90000000.
        let mut buf = vec![0x00, 0x00, 0x00, 0x90];
        patch_aarch64_page21(&mut buf, 0, 0x4000, 0x4FFF).unwrap();
        assert_eq!(read_u32(&buf, 0), 0x9000_0000);
    }

    #[test]
    fn adrp_immhi_immlo_split_matches_hand_encoding() {
        // +5 pages: imm=5 → immlo=0b01, immhi=1.
        // encoding: 0x90000000 | (1 << 29) | (1 << 5).
        let mut buf = vec![0x00, 0x00, 0x00, 0x90];
        patch_aarch64_page21(&mut buf, 0, 0, 5 << 12).unwrap();
        assert_eq!(read_u32(&buf, 0), 0x9000_0000 | (1 << 29) | (1 << 5));
    }

    #[test]
    fn adrp_beyond_4gb_is_err() {
        let mut buf = vec![0x00, 0x00, 0x00, 0x90];
        assert!(matches!(
            patch_aarch64_page21(&mut buf, 0, 0, 1 << 33),
            Err(PatchError::OutOfRange { .. })
        ));
    }

    // ---- aarch64 lo12 ----

    #[test]
    fn pageoff12_on_add_is_unscaled() {
        // add x0, x0, #imm12: base 0x91000000; target lo12 = 0x123.
        let mut buf = vec![0x00, 0x00, 0x00, 0x91];
        patch_aarch64_pageoff12(&mut buf, 0, 0x123, 0).unwrap();
        assert_eq!(read_u32(&buf, 0), 0x9100_0000 | (0x123 << 10));
    }

    #[test]
    fn pageoff12_on_ldr64_scales_offset_by_8() {
        // ldr x0, [x0, #imm12*8]: base 0xF9400000; lo12 = 0x18 → imm12 = 3.
        let mut buf = vec![0x00, 0x00, 0x40, 0xF9];
        patch_aarch64_pageoff12(&mut buf, 0, 0x18, 3).unwrap();
        assert_eq!(read_u32(&buf, 0), 0xF940_0000 | (3 << 10));
    }

    #[test]
    fn pageoff12_value_unaligned_for_scale_is_err() {
        let mut buf = vec![0x00, 0x00, 0x40, 0xF9];
        assert!(matches!(
            patch_aarch64_pageoff12(&mut buf, 0, 0x1C, 3),
            Err(PatchError::BadScale { .. })
        ));
    }

    // ---- x86-64 rel32 ----

    #[test]
    fn rel32_jmp_to_next_instruction_encodes_zero() {
        // jmp rel32: opcode at 0x1000, disp field at 0x1001, next insn at
        // 0x1005. With the normalized addend (-4): disp = S - 4 - P = 0.
        let mut buf = vec![0xE9, 0xAA, 0xAA, 0xAA, 0xAA];
        patch_x64_rel32(&mut buf, 1, 0x1001, 0x1005, -4).unwrap();
        assert_eq!(&buf[1..], &[0, 0, 0, 0]);
    }

    #[test]
    fn rel32_backward_with_normalized_addend() {
        // Field at 0x2000 (rip 0x2004), target 0x1000, normalized addend -4:
        // disp = 0x1000 - 4 - 0x2000 = -0x1004.
        let mut buf = vec![0u8; 4];
        patch_x64_rel32(&mut buf, 0, 0x2000, 0x1000, -4).unwrap();
        assert_eq!(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), -0x1004);
        // COFF REL32_1 (one trailing byte): one more byte of distance.
        patch_x64_rel32(&mut buf, 0, 0x2000, 0x1000, -5).unwrap();
        assert_eq!(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), -0x1005);
    }

    #[test]
    fn rel32_beyond_2gb_is_err() {
        let mut buf = vec![0u8; 4];
        assert!(matches!(
            patch_x64_rel32(&mut buf, 0, 0, 1 << 32, 0),
            Err(PatchError::OutOfRange { .. })
        ));
    }

    // ---- abs64 ----

    #[test]
    fn abs64_writes_little_endian_with_addend() {
        let mut buf = vec![0u8; 8];
        patch_abs64(&mut buf, 0, 0x1122_3344_5566_7788, 0x10);
        assert_eq!(buf, 0x1122_3344_5566_7798u64.to_le_bytes());
    }

    // ---- two-implementation differential over a displacement grid ----------

    /// An independent from-the-spec encoder for B/BL imm26 (ARM ARM C6.2.26):
    /// imm26 = (target - pc) / 4, masked into bits [25:0].
    fn reference_branch26(insn: u32, pc: u64, target: u64) -> u32 {
        let delta = target.wrapping_sub(pc) as i64;
        (insn & 0xFC00_0000) | (((delta as u64 >> 2) as u32) & 0x03FF_FFFF)
    }

    #[test]
    fn branch26_matches_reference_encoder_across_grid() {
        let base: u64 = 0x10_0000;
        for k in 0..10_000u64 {
            // Mixed forward/backward displacements across the range.
            let delta: i64 = ((k as i64) - 5_000) * 25_036;
            let target = (base as i64 + delta) as u64 & !3;
            let mut buf = vec![0x00, 0x00, 0x00, 0x14];
            patch_aarch64_branch26(&mut buf, 0, base, target).unwrap();
            assert_eq!(
                read_u32(&buf, 0),
                reference_branch26(0x1400_0000, base, target),
                "delta {delta}"
            );
        }
    }
}
