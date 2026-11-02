//! unified_placement — Phase 5: ONE LOGOS program placed across THREE execution planes that all run
//! LOGOS (host-CPU tiers, the reconfigurable fabric-CPU, and the DRAM-compute plane), under a single
//! shared primitive descriptor, certified ≡ the VM.
//!
//! The bet of the whole campaign, made mechanical: a bulk-bitwise/copy LOGOS op can run *inside the
//! memory* (ComputeDRAM), a spatial op on the fabric, and everything else on the host CPU — and the
//! compiler places each op where it wins, with the shared DRAM heap as the common data plane and no
//! marshalling between planes. Because every plane's per-op semantics are individually ≡VM (host is
//! the reference; the DRAM path is `super::dram::word_majority` — AND=MAJ(a,b,0), OR=MAJ(a,b,1),
//! proven in Phase 0; the fabric computes the same bitwise result), the placed whole program is ≡VM.
//!
//! `PRIMS` is the SINGLE SOURCE OF TRUTH: both the placement pass and the per-plane executors read it,
//! and a drift lock proves the DRAM executor supports exactly the ops the descriptor advertises.

use super::dram::word_majority;

/// The execution planes, all of which run LOGOS.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Plane {
    /// The i9 running the interpreter / VM / JIT / AOT / WASM tiers — universal (runs every op).
    HostCpu,
    /// The reconfigurable spatial fabric.
    Fabric,
    /// Compute-in-memory: bulk bitwise / copy inside the DDR2 (ComputeDRAM).
    Dram,
}

/// The op kinds in the shared primitive vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimKind {
    Copy,
    And,
    Or,
    Not,
    Xor,
    Popcount,
}

/// A bulk primitive over whole bit-vector rows in the shared data plane (row indices into the heap).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prim {
    Copy {
        dst: usize,
        src: usize,
    },
    And {
        dst: usize,
        a: usize,
        b: usize,
    },
    Or {
        dst: usize,
        a: usize,
        b: usize,
    },
    Not {
        dst: usize,
        src: usize,
    },
    Xor {
        dst: usize,
        a: usize,
        b: usize,
    },
    /// Count the set bits of `src` into word 0 of `dst` (a reduction — host-CPU only).
    Popcount {
        dst: usize,
        src: usize,
    },
}

impl Prim {
    pub fn kind(&self) -> PrimKind {
        match self {
            Prim::Copy { .. } => PrimKind::Copy,
            Prim::And { .. } => PrimKind::And,
            Prim::Or { .. } => PrimKind::Or,
            Prim::Not { .. } => PrimKind::Not,
            Prim::Xor { .. } => PrimKind::Xor,
            Prim::Popcount { .. } => PrimKind::Popcount,
        }
    }
}

/// One entry of the shared primitive descriptor: which planes can realize this op. The DRAM plane has
/// TWO tiers: `dram` = unmodified ComputeDRAM (Gao et al., MICRO 2019 — RowClone + MAJ-based AND/OR);
/// `dram_ambit` = the Ambit tier (Seshadri et al., MICRO 2017), which adds bulk NOT (dual-contact
/// cells) and thereby XOR, at the cost of MINOR DRAM modifications. `dram_ambit` is a superset of
/// `dram`.
#[derive(Clone, Copy, Debug)]
pub struct PrimDesc {
    pub kind: PrimKind,
    pub host_cpu: bool,
    pub fabric: bool,
    pub dram: bool,
    pub dram_ambit: bool,
}

/// THE SINGLE SOURCE OF TRUTH. host_cpu is always true (the host runs everything — the universal
/// fallback). `dram` is true only for the ops UNMODIFIED ComputeDRAM realizes: RowClone (copy) + bulk
/// bitwise AND/OR (the sense-amp majority) — the conservative tier the placement pass routes to.
/// `dram_ambit` additionally covers NOT + XOR (the Ambit boolean basis) on minor-modification DRAM;
/// the codegen_dram target uses this tier. Reductions (Popcount) are control-shaped → host only, on
/// neither tier.
pub const PRIMS: &[PrimDesc] = &[
    PrimDesc {
        kind: PrimKind::Copy,
        host_cpu: true,
        fabric: true,
        dram: true,
        dram_ambit: true,
    },
    PrimDesc {
        kind: PrimKind::And,
        host_cpu: true,
        fabric: true,
        dram: true,
        dram_ambit: true,
    },
    PrimDesc {
        kind: PrimKind::Or,
        host_cpu: true,
        fabric: true,
        dram: true,
        dram_ambit: true,
    },
    PrimDesc {
        kind: PrimKind::Not,
        host_cpu: true,
        fabric: true,
        dram: false,
        dram_ambit: true,
    },
    PrimDesc {
        kind: PrimKind::Xor,
        host_cpu: true,
        fabric: true,
        dram: false,
        dram_ambit: true,
    },
    PrimDesc {
        kind: PrimKind::Popcount,
        host_cpu: true,
        fabric: false,
        dram: false,
        dram_ambit: false,
    },
];

pub fn desc(kind: PrimKind) -> &'static PrimDesc {
    PRIMS
        .iter()
        .find(|d| d.kind == kind)
        .expect("every PrimKind has a descriptor")
}

/// Operands at least this many u64 words wide favor in-DRAM execution (the bytes-not-moved win).
pub const WIDE_WORDS: usize = 8;

/// The placement pass: choose the plane for `prim` given its operand width, from the shared descriptor.
/// Bulk/wide bitwise + copy → DRAM (compute-in-memory); otherwise the fabric; else the host CPU (which
/// runs everything). Never fails — the host is the universal fallback.
pub fn place(prim: &Prim, operand_words: usize) -> Plane {
    let d = desc(prim.kind());
    if d.dram && operand_words >= WIDE_WORDS {
        Plane::Dram // bulk/wide bitwise or copy: compute in place, move no bytes across the bus
    } else if d.fabric {
        Plane::Fabric // spatial/parallel bitwise the DRAM can't do (or too narrow to win in-memory)
    } else {
        debug_assert!(d.host_cpu, "every op is at least host-CPU realizable");
        Plane::HostCpu // reductions / control — the universal fallback
    }
}

/// A row of the shared data plane: a bit vector packed into u64 words.
pub type Row = Vec<u64>;

/// The reference (VM) semantics of a primitive over the heap — the oracle every plane is proven against.
pub fn exec_reference(heap: &mut [Row], prim: &Prim) {
    match *prim {
        Prim::Copy { dst, src } => heap[dst] = heap[src].clone(),
        Prim::And { dst, a, b } => {
            heap[dst] = heap[a].iter().zip(&heap[b]).map(|(x, y)| x & y).collect()
        }
        Prim::Or { dst, a, b } => {
            heap[dst] = heap[a].iter().zip(&heap[b]).map(|(x, y)| x | y).collect()
        }
        Prim::Not { dst, src } => heap[dst] = heap[src].iter().map(|x| !x).collect(),
        Prim::Xor { dst, a, b } => {
            heap[dst] = heap[a].iter().zip(&heap[b]).map(|(x, y)| x ^ y).collect()
        }
        Prim::Popcount { dst, src } => {
            let n: u64 = heap[src].iter().map(|w| w.count_ones() as u64).sum();
            let width = heap[dst].len();
            heap[dst] = std::iter::once(n)
                .chain(std::iter::repeat(0))
                .take(width)
                .collect();
        }
    }
}

/// Execute `prim` on the DRAM-compute plane (ComputeDRAM semantics): RowClone for copy, the sense-amp
/// majority reduction for AND (MAJ(a,b,0)) and OR (MAJ(a,b,1)). Returns `None` for ops with no plain-
/// ComputeDRAM realization (the descriptor's `dram: false` ops). MUST agree op-for-op with [`exec_reference`].
pub fn exec_dram(heap: &mut [Row], prim: &Prim) -> Option<()> {
    match *prim {
        Prim::Copy { dst, src } => heap[dst] = heap[src].clone(), // RowClone
        Prim::And { dst, a, b } => {
            heap[dst] = heap[a]
                .iter()
                .zip(&heap[b])
                .map(|(x, y)| word_majority(*x, *y, 0))
                .collect()
        }
        Prim::Or { dst, a, b } => {
            heap[dst] = heap[a]
                .iter()
                .zip(&heap[b])
                .map(|(x, y)| word_majority(*x, *y, u64::MAX))
                .collect()
        }
        // No plain-ComputeDRAM realization (matches the descriptor's dram: false): NOT needs
        // Ambit-style dual-contact cells; XOR is multi-step; Popcount is a reduction → fabric/host.
        Prim::Not { .. } | Prim::Xor { .. } | Prim::Popcount { .. } => return None,
    }
    Some(())
}

/// Execute `prim` on the DRAM plane at the AMBIT tier (Seshadri et al., MICRO 2017): the unmodified
/// ComputeDRAM ops ([`exec_dram`]) PLUS bulk NOT (dual-contact cells) and XOR (NOT + triple-row MAJ,
/// a multi-row in-DRAM sequence — the RESULT is exactly `a ^ b`, the ≡VM contract; the sequencing is
/// the in-DRAM implementation detail). Ambit needs MINOR DRAM modifications, distinct from the
/// unmodified [`exec_dram`]. Returns `None` only for the reduction (Popcount is control-shaped →
/// host). Realizes EXACTLY the descriptor's `dram_ambit` ops, and agrees op-for-op with
/// [`exec_reference`].
pub fn exec_dram_ambit(heap: &mut [Row], prim: &Prim) -> Option<()> {
    match *prim {
        Prim::Copy { .. } | Prim::And { .. } | Prim::Or { .. } => return exec_dram(heap, prim),
        Prim::Not { dst, src } => heap[dst] = heap[src].iter().map(|x| !x).collect(),
        Prim::Xor { dst, a, b } => {
            heap[dst] = heap[a].iter().zip(&heap[b]).map(|(x, y)| x ^ y).collect()
        }
        Prim::Popcount { .. } => return None,
    }
    Some(())
}

/// Execute `prim` on the fabric plane. The fabric computes the same bitwise result as the VM (its
/// tile ALUs are proven ≡VM elsewhere); returns `false` for ops with no fabric realization.
pub fn exec_fabric(heap: &mut [Row], prim: &Prim) -> bool {
    if !desc(prim.kind()).fabric {
        return false;
    }
    exec_reference(heap, prim);
    true
}

/// Dispatch `prim` to `plane`, asserting the plane can realize it (placement must never assign an op to
/// a plane that cannot run it). Each plane's result equals the VM reference (proven by the locks).
pub fn exec_on(plane: Plane, heap: &mut [Row], prim: &Prim) {
    match plane {
        Plane::HostCpu => exec_reference(heap, prim),
        Plane::Fabric => assert!(
            exec_fabric(heap, prim),
            "fabric cannot run {:?}",
            prim.kind()
        ),
        Plane::Dram => {
            exec_dram(heap, prim).unwrap_or_else(|| panic!("DRAM cannot run {:?}", prim.kind()))
        }
    }
}

/// Run a program with placement: each op executes on the plane the cost model assigns it. `operand_words`
/// is the row width (the placement's size input). Returns the final heap.
pub fn run_placed(heap0: &[Row], program: &[Prim], operand_words: usize) -> Vec<Row> {
    let mut heap = heap0.to_vec();
    for prim in program {
        exec_on(place(prim, operand_words), &mut heap, prim);
    }
    heap
}

/// Run a program entirely on the host CPU — the VM reference.
pub fn run_reference(heap0: &[Row], program: &[Prim]) -> Vec<Row> {
    let mut heap = heap0.to_vec();
    for prim in program {
        exec_reference(&mut heap, prim);
    }
    heap
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/unified_placement/tests.rs"]
mod tests;
