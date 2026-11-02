//! Number variables on the bytecode walker whose definitions make numbers of different forms
//! ([`crate::vm::numbers`]). Each definition whose own kind differs from its variable's computes its own kind and lands
//! converted into the variable's, so every read finds the variable's form; a parameter is converted as its region
//! begins; a variable no machine form holds refuses the program by name. Integer width is not a form here: the
//! walker's exactness analysis holds an integer that can outgrow a machine word ([`super::exactness`]).

use std::collections::BTreeMap;

use super::{kind_at, Emitter, Kind, WalkerRegion};
use crate::vm::numbers::{joined, Form, Number};
use crate::vm::variables::{defined, Site};

/// How a region's number variables are held: the kind each definition lands as (by pc), the kind each parameter is
/// converted to, and one definition of each variable no machine form holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Landings {
    pub(super) defs: BTreeMap<usize, Kind>,
    pub(super) entries: BTreeMap<u16, Kind>,
    pub(super) refused: Vec<Site>,
}

impl<'a> Emitter<'a> {
    /// The number form of a walker kind: an integer of any width, a rational, a real, or a list of one of them.
    fn number_form(&self, k: Kind) -> Option<Form> {
        let number = |k: Kind| match k {
            Kind::Int | Kind::BigInt => Some(Number::Int),
            Kind::Rational => Some(Number::Rational),
            Kind::Float => Some(Number::Real),
            _ => None,
        };
        match k {
            Kind::SeqInt => Some(Form::List(Number::Int)),
            Kind::SeqFloat => Some(Form::List(Number::Real)),
            Kind::SeqOf(_) => number(self.elem_kind(k)).map(Form::List),
            other => number(other).map(Form::Number),
        }
    }

    /// The walker kind that holds number form `f`.
    fn number_kind(&self, f: Form) -> Kind {
        match f {
            Form::Number(Number::Int | Number::BigInt) => Kind::Int,
            Form::Number(Number::Rational) => Kind::Rational,
            Form::Number(Number::Real) => Kind::Float,
            Form::List(Number::Int | Number::BigInt) => Kind::SeqInt,
            Form::List(Number::Real) => Kind::SeqFloat,
            Form::List(Number::Rational) => self.seq_kind_for(Kind::Rational),
        }
    }

    /// The landings of region `[start, end)` under its kinds `per_pc` and entry kinds `seed`. A definition's own kind
    /// is what it computes ([`Emitter::number_natural`]), not what it lands as.
    pub(super) fn number_landings(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
        per_pc: &[Vec<Kind>],
        seed: &[Kind],
    ) -> Landings {
        let region = WalkerRegion {
            emitter: self,
            start,
            end,
            holes,
        };
        let code = &self.prog.code;
        let j = joined(
            code,
            &self.prog.constants,
            &region,
            seed.len() as u16,
            |site| match site {
                Site::Entry(reg) => self.number_form(kind_at(seed, reg)),
                Site::Op(pc) => {
                    let w = defined(&code[pc])?;
                    let k = self
                        .number_natural(pc)
                        .unwrap_or_else(|| kind_at(&per_pc[pc - start + 1], w));
                    self.number_form(k)
                }
            },
            |pc, reg| kind_at(&per_pc[pc - start], reg) == Kind::Float,
        );
        Landings {
            defs: j.lands.into_iter().map(|(pc, f)| (pc, self.number_kind(f))).collect(),
            entries: j.entries.into_iter().map(|(reg, f)| (reg, self.number_kind(f))).collect(),
            refused: j.refused,
        }
    }

    /// The kind the op at `pc` computes where a number join lands it as another, in the version being inferred.
    pub(super) fn number_natural(&self, pc: usize) -> Option<Kind> {
        self.number_from.borrow()[self.on_words.get() as usize].get(&pc).map(|(natural, _)| *natural)
    }

    /// A register value `v` of kind `from`, converted to number kind `to`.
    pub(super) fn convert_number(&mut self, v: &str, from: Kind, to: Kind) -> Result<String, String> {
        Ok(match (from, to) {
            (Kind::Int | Kind::Bool, Kind::Rational) => {
                let n = self.fresh();
                format!("({{ LDec *_nl{n} = (LDec*)lc_gc_alloc(sizeof(LDec)); *_nl{n} = ldec_from_i64({v}); (int64_t)(intptr_t)_nl{n}; }})")
            }
            (Kind::BigInt, Kind::Rational) => {
                let n = self.fresh();
                format!("({{ LDec *_nl{n} = (LDec*)lc_gc_alloc(sizeof(LDec)); *_nl{n} = ldec_from_lint(lc_xget({v})); (int64_t)(intptr_t)_nl{n}; }})")
            }
            (Kind::Int | Kind::Bool, Kind::Float) => format!("as_bits((double)({v}))"),
            (Kind::BigInt, Kind::Float) => format!("as_bits(lint_to_f64(lc_xget({v})))"),
            (Kind::SeqInt, Kind::SeqOf(_)) if self.elem_kind(to) == Kind::Rational => {
                format!("(int64_t)(intptr_t)lc_seq_i64_as_ldec((Seq_i64*)(intptr_t)({v}))")
            }
            _ => return Err(format!("a {from:?} landing as {to:?} is not yet lowered")),
        })
    }
}
