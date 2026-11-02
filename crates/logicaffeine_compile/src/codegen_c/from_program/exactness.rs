//! Which integers of a region the walker holds exact: the walker's typing, asked of the decision every backend that
//! lowers the bytecode shares ([`crate::vm::exactness`]).

use crate::vm::exactness::{ExactnessHost, IntForm, Region};
use crate::vm::CompiledProgram;

pub(super) use crate::vm::exactness::Exactness;

use super::{Emitter, Kind};

/// The walker's kinds on machine words for one region, one entry per pc plus the state after the last op: only
/// what the exact tower computes is exact in them.
struct WordKinds<'e, 'a> {
    emitter: &'e Emitter<'a>,
    start: usize,
    end: usize,
    holes: &'e [(usize, usize)],
    kinds: &'e [Vec<Kind>],
}

/// The integer form of a walker kind; `None` when it holds no integer.
fn int_form(k: Kind) -> Option<IntForm> {
    match k {
        Kind::Int => Some(IntForm::Word),
        Kind::BigInt => Some(IntForm::Exact),
        _ => None,
    }
}

impl ExactnessHost for WordKinds<'_, '_> {
    fn program(&self) -> &CompiledProgram {
        self.emitter.prog
    }

    fn int_after(&self, pc: usize, reg: u16) -> Option<IntForm> {
        int_form(
            self.kinds[pc - self.start + 1]
                .get(reg as usize)
                .copied()
                .unwrap_or(Kind::Unknown),
        )
    }

    fn arrives_exact(&self, pc: usize) -> bool {
        self.emitter
            .arrives_exact(self.op(pc), &self.kinds[pc - self.start])
    }

    fn successors(&self, pc: usize) -> Vec<usize> {
        self.emitter
            .successors(pc, self.start, self.end, self.holes)
    }
}

impl<'a> Emitter<'a> {
    /// Decide which integer definitions of region `[start, end)` are exact, given the region's kinds on machine
    /// words (`kinds`), its entry kinds (`seed`: a parameter an exact value arrives in is exact) and the function it
    /// is the body of.
    pub(super) fn exactness(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
        kinds: &[Vec<Kind>],
        seed: &[Kind],
        func: Option<usize>,
    ) -> Exactness {
        let seed: Vec<Option<IntForm>> = seed.iter().map(|k| int_form(*k)).collect();
        let host = WordKinds {
            emitter: self,
            start,
            end,
            holes,
            kinds,
        };
        crate::vm::exactness::exactness(
            &host,
            &Region {
                start,
                end,
                holes,
                func,
                seed: &seed,
            },
        )
    }
}
