//! The copy-and-patch assembler: glue stencils into executable code.
//!
//! [`JitBuffer`] collects stencil instances with their hole values, then
//! `finish()` lays everything out — stencil code, an 8-byte-aligned literal
//! pool for constant holes, and pointer slots for GOT-indirect sites — maps a
//! page (learning the final base address), patches every relocation against
//! that base, writes ONCE, and seals. No mutable window ever escapes; W^X
//! toggling stays confined to the constructing thread.
//!
//! Layout:
//! ```text
//! [piece 0][piece 1]…[piece N]  [pointer slots…]  [value slots…]
//!  ^code, 16-byte aligned        ^8-byte aligned GOT-style slots
//! ```

use crate::patch;
use crate::stencil_model::{HoleId, RelocKind, Stencil};
use crate::{JitError, JitPage};

/// A placed stencil instance, identifying a continuation target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(usize);

/// What to plug into a stencil's holes, by hole id.
#[derive(Clone, Copy, Debug)]
pub enum HoleValue {
    /// Continuation hole N jumps to this piece.
    Cont(u8, Label),
    /// Constant hole N reads this value.
    Const(u8, i64),
}

/// Assembly errors.
#[derive(Debug)]
pub enum BufferError {
    /// A hole had no value supplied.
    MissingHoleValue {
        /// The stencil whose hole went unfilled.
        stencil: &'static str,
        /// The unfilled hole.
        hole: HoleId,
    },
    /// A label referenced a piece that does not exist.
    UnresolvedLabel(Label),
    /// A relocation could not be patched.
    Patch(patch::PatchError),
    /// The page could not be created.
    Jit(JitError),
    /// The buffer has no pieces.
    Empty,
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BufferError::MissingHoleValue { stencil, hole } => {
                write!(f, "stencil '{stencil}': no value for hole {hole:?}")
            }
            BufferError::UnresolvedLabel(l) => write!(f, "unresolved label {l:?}"),
            BufferError::Patch(e) => write!(f, "patch failed: {e}"),
            BufferError::Jit(e) => write!(f, "page failed: {e}"),
            BufferError::Empty => write!(f, "empty JitBuffer"),
        }
    }
}

impl std::error::Error for BufferError {}

struct Piece {
    stencil: &'static Stencil,
    holes: Vec<HoleValue>,
}

/// The staged assembly.
#[derive(Default)]
pub struct JitBuffer {
    pieces: Vec<Piece>,
}

/// A finished, executable chain.
#[derive(Debug)]
pub struct JitChain {
    page: JitPage,
}

impl JitChain {
    /// The CPS entry point: `fn(base, sp) -> i64` — `base` is the frame (the
    /// compiled function's register slots), `sp` the operand-stack top.
    ///
    /// # Safety
    /// `base` must point at a frame at least as large as every patched slot
    /// index; `sp` must point into an operand stack with capacity for the
    /// chain's pushes; the page must outlive every call.
    pub unsafe fn entry(&self) -> unsafe extern "C" fn(*mut i64, *mut i64) -> i64 {
        std::mem::transmute::<*const u8, unsafe extern "C" fn(*mut i64, *mut i64) -> i64>(
            self.page.as_ptr(),
        )
    }

    /// Run the chain with an empty frame and a fresh operand stack.
    pub fn run(&self) -> i64 {
        let mut frame = vec![0i64; 64];
        self.run_with_frame(&mut frame)
    }

    /// Run the chain over the given frame (slot 0 = VM register 0, …), with a
    /// fresh operand stack. The frame is read AND written by slot stencils.
    pub fn run_with_frame(&self, frame: &mut [i64]) -> i64 {
        let mut stack = vec![0i64; 256];
        unsafe { (self.entry())(frame.as_mut_ptr(), stack.as_mut_ptr()) }
    }

    /// The mapped code+pool bytes (diagnostics/tests).
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.page.as_ptr(), self.page.len()) }
    }

    /// The runtime base address (diagnostics/tests).
    pub fn base(&self) -> u64 {
        self.page.as_ptr() as u64
    }
}

const PIECE_ALIGN: usize = 16;
const SLOT_SIZE: usize = 8;

fn align_up(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

impl JitBuffer {
    /// An empty buffer.
    pub fn new() -> Self {
        JitBuffer::default()
    }

    /// Append a stencil instance; the returned [`Label`] is its address for
    /// continuation holes (including back-edges to earlier pieces).
    pub fn push_stencil(&mut self, stencil: &'static Stencil, holes: &[HoleValue]) -> Label {
        self.pieces.push(Piece { stencil, holes: holes.to_vec() });
        Label(self.pieces.len() - 1)
    }

    /// The label of the piece at `index` — usable for FORWARD references
    /// (validated when `finish` runs).
    pub fn label(&self, index: usize) -> Label {
        Label(index)
    }

    /// Lay out, map, patch against the final base, write once, seal.
    pub fn finish(self) -> Result<JitChain, BufferError> {
        if self.pieces.is_empty() {
            return Err(BufferError::Empty);
        }

        // Piece offsets.
        let mut piece_off: Vec<usize> = Vec::with_capacity(self.pieces.len());
        let mut cursor = 0usize;
        for piece in &self.pieces {
            cursor = align_up(cursor, PIECE_ALIGN);
            piece_off.push(cursor);
            cursor += piece.stencil.code.len();
        }

        // Slot assignment: one VALUE slot per Const hole instance; one POINTER
        // slot per (piece, hole) among indirect relocs — SHARED by every
        // indirect reloc of that hole. An arm64 GOT load is an ADRP+LDR PAIR
        // (two relocs, one hole): the ADRP contributes the 4 KiB page and the
        // LDR the low 12 bits, so both MUST resolve to the same slot or the
        // composed address breaks as soon as the pool straddles a page
        // boundary.
        let mut value_slot: Vec<Vec<Option<usize>>> = Vec::new(); // [piece][hole-n] -> slot off
        let mut pointer_slot: std::collections::HashMap<(usize, HoleId), usize> = std::collections::HashMap::new();
        let mut slot_cursor = align_up(cursor, SLOT_SIZE);

        for (pi, piece) in self.pieces.iter().enumerate() {
            let mut per_hole: Vec<Option<usize>> = vec![None; 8];
            for hv in &piece.holes {
                if let HoleValue::Const(n, _) = hv {
                    if per_hole[*n as usize].is_none() {
                        per_hole[*n as usize] = Some(slot_cursor);
                        slot_cursor += SLOT_SIZE;
                    }
                }
            }
            value_slot.push(per_hole);
            for reloc in piece.stencil.relocs.iter() {
                if patch::is_indirect(reloc.kind) {
                    pointer_slot.entry((pi, reloc.target)).or_insert_with(|| {
                        let s = slot_cursor;
                        slot_cursor += SLOT_SIZE;
                        s
                    });
                }
            }
        }
        let total_len = slot_cursor;

        // Resolve a hole on a piece to (target address fn of base, is_const_value).
        let find_hole = |piece: &Piece, hole: HoleId| -> Result<HoleValue, BufferError> {
            for hv in &piece.holes {
                match (hv, hole) {
                    (HoleValue::Cont(n, _), HoleId::Cont(m)) if *n == m => return Ok(*hv),
                    (HoleValue::Const(n, _), HoleId::ConstI64(m)) if *n == m => return Ok(*hv),
                    _ => {}
                }
            }
            Err(BufferError::MissingHoleValue { stencil: piece.stencil.name, hole })
        };

        // Validate labels up front.
        for piece in &self.pieces {
            for hv in &piece.holes {
                if let HoleValue::Cont(_, Label(t)) = hv {
                    if *t >= self.pieces.len() {
                        return Err(BufferError::UnresolvedLabel(Label(*t)));
                    }
                }
            }
        }

        let pieces = self.pieces;
        let mut patch_err: Option<BufferError> = None;
        let page = JitPage::with_layout(total_len, |base| {
            let mut buf = vec![0u8; total_len];

            // Copy stencil code.
            for (pi, piece) in pieces.iter().enumerate() {
                let off = piece_off[pi];
                buf[off..off + piece.stencil.code.len()].copy_from_slice(piece.stencil.code);
            }

            let mut do_patch = || -> Result<(), BufferError> {
                // Fill value slots.
                for (pi, piece) in pieces.iter().enumerate() {
                    for hv in &piece.holes {
                        if let HoleValue::Const(n, v) = hv {
                            if let Some(slot) = value_slot[pi][*n as usize] {
                                buf[slot..slot + 8].copy_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                // Fill pointer slots (one per (piece, hole)).
                for (&(pi, hole), &slot) in &pointer_slot {
                    let piece = &pieces[pi];
                    let content: u64 = match find_hole(piece, hole)? {
                        HoleValue::Cont(_, Label(t)) => base + piece_off[t] as u64,
                        HoleValue::Const(n, _) => {
                            let vslot = value_slot[pi][n as usize]
                                .expect("const hole has a value slot");
                            base + vslot as u64
                        }
                    };
                    buf[slot..slot + 8].copy_from_slice(&content.to_le_bytes());
                }

                // Patch every relocation site.
                for (pi, piece) in pieces.iter().enumerate() {
                    for reloc in piece.stencil.relocs.iter() {
                        let site_off = piece_off[pi] + reloc.offset as usize;
                        let site_addr = base + site_off as u64;
                        // The address the SITE refers to: a pointer slot for
                        // indirect kinds, else the direct target.
                        let target_addr: u64 = if patch::is_indirect(reloc.kind) {
                            base + pointer_slot[&(pi, reloc.target)] as u64
                        } else {
                            match find_hole(piece, reloc.target)? {
                                HoleValue::Cont(_, Label(t)) => base + piece_off[t] as u64,
                                HoleValue::Const(n, _) => {
                                    let vslot = value_slot[pi][n as usize]
                                        .expect("const hole has a value slot");
                                    base + vslot as u64
                                }
                            }
                        };
                        let r = match reloc.kind {
                            RelocKind::Branch26 => {
                                patch::patch_aarch64_branch26(&mut buf, site_off, site_addr, target_addr)
                            }
                            RelocKind::Page21 | RelocKind::GotPage21 => {
                                patch::patch_aarch64_page21(&mut buf, site_off, site_addr, target_addr)
                            }
                            RelocKind::PageOff12 { scale } => {
                                patch::patch_aarch64_pageoff12(&mut buf, site_off, target_addr, scale)
                            }
                            RelocKind::GotPageOff12 => {
                                patch::patch_aarch64_pageoff12(&mut buf, site_off, target_addr, 3)
                            }
                            RelocKind::Rel32 | RelocKind::GotRel32 => {
                                patch::patch_x64_rel32(&mut buf, site_off, site_addr, target_addr, reloc.addend)
                            }
                            RelocKind::Abs64 => {
                                patch::patch_abs64(&mut buf, site_off, target_addr, reloc.addend);
                                Ok(())
                            }
                        };
                        r.map_err(BufferError::Patch)?;
                    }
                }
                Ok(())
            };
            if let Err(e) = do_patch() {
                patch_err = Some(e);
            }
            buf
        })
        .map_err(BufferError::Jit)?;

        if let Some(e) = patch_err {
            return Err(e);
        }
        Ok(JitChain { page })
    }
}
