//! Each region cell as a lane routine: the fabric's checked 64-bit semantics
//! (`AluOp`/`FuncUnit`/`CmpKind::eval_checked`) realized at a device's lane shape by the multi-word core and
//! the softfloat.
//!
//! Every binary cell routine takes its two operands as 64-bit values (the lane words of `a`, then of `b`) and
//! returns the 64-bit result followed by the fault word (0, or a `Divergence` code). A unary unit ignores
//! `b`. A select takes the two compared operands and the two arms, and returns the chosen arm.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::codegen_sva::fabric_rtl::{AluOp, CmpKind, FuncUnit};

use super::int64::{self, n64, nfault, no_fault};
use super::lane::{LaneShape, Routine};
use super::multiword::{Builder, Mw};
use super::softfloat;

/// What a cell computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellOp {
    Alu(AluOp),
    Unit(FuncUnit),
    /// An integer select: `cmp(ca, cb) ? t : e`.
    Select(CmpKind),
}

impl CellOp {
    pub fn name(self) -> String {
        match self {
            CellOp::Alu(op) => format!("alu_{op:?}").to_lowercase(),
            CellOp::Unit(u) => format!("unit_{u:?}").to_lowercase(),
            CellOp::Select(c) => format!("select_{c:?}").to_lowercase(),
        }
    }

    /// The 64-bit operands the routine takes.
    pub fn operands(self) -> usize {
        match self {
            CellOp::Select(_) => 4,
            _ => 2,
        }
    }

    /// Whether the routine returns a fault word after its result.
    pub fn faults(self) -> bool {
        !matches!(self, CellOp::Select(_))
    }
}

fn binary(
    shape: LaneShape,
    name: String,
    f: impl FnOnce(&mut Builder, &Mw, &Mw) -> (Mw, Mw),
) -> Routine {
    let mut b = Builder::new(shape);
    let n = n64(&b);
    let x = b.mw_input(n);
    let y = b.mw_input(n);
    let (v, fault) = f(&mut b, &x, &y);
    assert_eq!(v.len(), n);
    assert_eq!(fault.len(), nfault(&b));
    let inputs = x.0.iter().chain(&y.0).copied().collect();
    let outputs = v.0.iter().chain(&fault.0).copied().collect();
    b.finish(name, inputs, outputs)
}

fn exact(
    f: impl FnOnce(&mut Builder, &Mw, &Mw) -> Mw,
) -> impl FnOnce(&mut Builder, &Mw, &Mw) -> (Mw, Mw) {
    move |b, x, y| {
        let v = f(b, x, y);
        let ok = no_fault(b);
        (v, ok)
    }
}

fn predicate(
    f: impl FnOnce(&mut Builder, &Mw, &Mw) -> u32,
) -> impl FnOnce(&mut Builder, &Mw, &Mw) -> (Mw, Mw) {
    move |b, x, y| {
        let n = x.len();
        let p = f(b, x, y);
        let v = int64::bool64(b, p, n);
        let ok = no_fault(b);
        (v, ok)
    }
}

/// An integer comparison as 0/1.
fn int_compare(b: &mut Builder, c: CmpKind, x: &Mw, y: &Mw) -> u32 {
    match c {
        CmpKind::Lt => b.mw_lts(x, y),
        CmpKind::Gt => b.mw_lts(y, x),
        CmpKind::Le => {
            let gt = b.mw_lts(y, x);
            b.bit_not(gt)
        }
        CmpKind::Ge => {
            let lt = b.mw_lts(x, y);
            b.bit_not(lt)
        }
        CmpKind::Eq => b.mw_eq(x, y),
        CmpKind::Ne => {
            let e = b.mw_eq(x, y);
            b.bit_not(e)
        }
    }
}

/// Build `op`'s routine at `shape`.
pub fn build(shape: LaneShape, op: CellOp) -> Routine {
    let name = op.name();
    match op {
        CellOp::Alu(alu) => match alu {
            AluOp::Add => binary(shape, name, int64::add),
            AluOp::Sub => binary(shape, name, int64::sub),
            AluOp::Mul => binary(shape, name, int64::mul),
            AluOp::And => binary(shape, name, exact(|b, x, y| b.mw_and(x, y))),
            AluOp::Or => binary(shape, name, exact(|b, x, y| b.mw_or(x, y))),
            AluOp::Xor => binary(shape, name, exact(|b, x, y| b.mw_xor(x, y))),
            AluOp::Shl => binary(shape, name, exact(int64::shl)),
            AluOp::Shr => binary(shape, name, exact(int64::shr)),
            AluOp::UShr => binary(shape, name, exact(int64::ushr)),
        },
        CellOp::Unit(unit) => match unit {
            FuncUnit::DivS => binary(shape, name, int64::div),
            FuncUnit::ModS => binary(shape, name, int64::rem),
            FuncUnit::FloorDivS => binary(shape, name, int64::floor_div),
            FuncUnit::Pow => binary(shape, name, int64::pow),
            FuncUnit::MulHiU => binary(shape, name, exact(int64::mul_hi_u)),
            FuncUnit::MulOvfS => binary(shape, name, exact(int64::mul_ovf_s)),
            FuncUnit::PopCount => binary(shape, name, exact(|b, x, _| int64::popcount(b, x))),
            FuncUnit::ApproxEqF => binary(shape, name, predicate(softfloat::approx_eq_ints)),
            FuncUnit::FAddF => binary(shape, name, exact(|b, x, y| softfloat::add(b, x, y, false))),
            FuncUnit::FSubF => binary(shape, name, exact(|b, x, y| softfloat::add(b, x, y, true))),
            FuncUnit::FMulF => binary(shape, name, exact(softfloat::mul)),
            FuncUnit::FDivF => binary(shape, name, |b, x, y| {
                let v = softfloat::div(b, x, y);
                let uy = softfloat::unpack(b, y);
                let ok = no_fault(b);
                let f = int64::fault_when(
                    b,
                    uy.is_zero,
                    crate::codegen_sva::fabric_rtl::Divergence::FloatDivideByZero,
                    &ok,
                );
                (v, f)
            }),
            FuncUnit::FSqrtF => binary(shape, name, exact(|b, x, _| softfloat::sqrt(b, x))),
            FuncUnit::FFloorF => {
                binary(shape, name, exact(|b, x, _| softfloat::floor_to_int(b, x)))
            }
            FuncUnit::IntToF => binary(shape, name, exact(|b, x, _| softfloat::from_int(b, x))),
            FuncUnit::FLtF => binary(
                shape,
                name,
                predicate(|b, x, y| softfloat::compare(b, x, y).0),
            ),
            FuncUnit::FGtF => binary(
                shape,
                name,
                predicate(|b, x, y| softfloat::compare(b, y, x).0),
            ),
            FuncUnit::FLeF => binary(
                shape,
                name,
                predicate(|b, x, y| softfloat::compare(b, x, y).1),
            ),
            FuncUnit::FGeF => binary(
                shape,
                name,
                predicate(|b, x, y| softfloat::compare(b, y, x).1),
            ),
            FuncUnit::FEqF => binary(
                shape,
                name,
                predicate(|b, x, y| softfloat::compare(b, x, y).2),
            ),
            FuncUnit::FNeF => binary(
                shape,
                name,
                predicate(|b, x, y| {
                    let e = softfloat::compare(b, x, y).2;
                    b.bit_not(e)
                }),
            ),
        },
        CellOp::Select(c) => {
            let mut b = Builder::new(shape);
            let n = n64(&b);
            let ca = b.mw_input(n);
            let cb = b.mw_input(n);
            let t = b.mw_input(n);
            let e = b.mw_input(n);
            let p = int_compare(&mut b, c, &ca, &cb);
            let v = b.mw_sel(p, &t, &e);
            let inputs = [&ca, &cb, &t, &e]
                .iter()
                .flat_map(|m| m.0.iter().copied())
                .collect();
            b.finish(name, inputs, v.0)
        }
    }
}

/// `op`'s routine at `shape`, built once per process.
pub fn routine(shape: LaneShape, op: CellOp) -> Arc<Routine> {
    static CACHE: OnceLock<Mutex<HashMap<(LaneShape, CellOp), Arc<Routine>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(r) = cache.lock().expect("the routine cache").get(&(shape, op)) {
        return r.clone();
    }
    let built = Arc::new(build(shape, op));
    cache
        .lock()
        .expect("the routine cache")
        .entry((shape, op))
        .or_insert(built)
        .clone()
}

/// Every cell op, in a fixed order.
pub fn all_ops() -> Vec<CellOp> {
    let alu = [
        AluOp::Add,
        AluOp::Sub,
        AluOp::Mul,
        AluOp::And,
        AluOp::Or,
        AluOp::Xor,
        AluOp::Shl,
        AluOp::Shr,
        AluOp::UShr,
    ];
    let units = [
        FuncUnit::DivS,
        FuncUnit::ModS,
        FuncUnit::FloorDivS,
        FuncUnit::Pow,
        FuncUnit::MulHiU,
        FuncUnit::ApproxEqF,
        FuncUnit::FAddF,
        FuncUnit::FSubF,
        FuncUnit::FMulF,
        FuncUnit::FDivF,
        FuncUnit::FSqrtF,
        FuncUnit::FFloorF,
        FuncUnit::MulOvfS,
        FuncUnit::IntToF,
        FuncUnit::FLtF,
        FuncUnit::FGtF,
        FuncUnit::FLeF,
        FuncUnit::FGeF,
        FuncUnit::FEqF,
        FuncUnit::FNeF,
        FuncUnit::PopCount,
    ];
    let cmps = [
        CmpKind::Lt,
        CmpKind::Gt,
        CmpKind::Le,
        CmpKind::Ge,
        CmpKind::Eq,
        CmpKind::Ne,
    ];
    alu.into_iter()
        .map(CellOp::Alu)
        .chain(units.into_iter().map(CellOp::Unit))
        .chain(cmps.into_iter().map(CellOp::Select))
        .collect()
}
