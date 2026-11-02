//! Primitive validation: the literal extension is only as sound as the definitions it replaces.
//!
//! Lean's kernel evaluates `Nat.add 2 3` on machine numbers instead of unfolding `Nat.add`. That
//! is sound only if the environment's `Nat.add` really is addition. Before a primitive name is
//! admitted, its definition is checked against its defining equations, stated over a free
//! variable so they hold for every argument (the scheme of `lean4lean`'s `Primitive.lean`). A
//! primitive whose equations are proven is accelerated; one whose form is wrong is rejected.

use super::check::DefnDecl;
use super::env::Safety;
use super::error::{KResult, KernelFailure};
use super::expr::ExprId;
use super::level::LevelsId;
use super::name::NameId;
use super::tc::Kernel;

impl Kernel {
    fn prim_fail(&self, n: NameId) -> KernelFailure {
        KernelFailure::reject(format!(
            "invalid form for primitive definition '{}'",
            self.s.names.display(n)
        ))
    }

    fn c0(&mut self, n: NameId) -> ExprId {
        self.s.cnst(n, LevelsId::EMPTY)
    }

    /// `Nat → a ≡ Nat → b`: equality under one bound natural number (`BVar(0)`).
    fn defeq1(&mut self, a: ExprId, b: ExprId) -> KResult<bool> {
        let nat = self.c0(self.wk.nat);
        let pa = self.s.pi(nat, a);
        let pb = self.s.pi(nat, b);
        self.is_def_eq(pa, pb)
    }

    /// Equality under two bound natural numbers (`BVar(1)`, `BVar(0)`).
    fn defeq2(&mut self, a: ExprId, b: ExprId) -> KResult<bool> {
        let nat = self.c0(self.wk.nat);
        let pa = self.s.pi(nat, a);
        let pb = self.s.pi(nat, b);
        self.defeq1(pa, pb)
    }

    fn arrow(&mut self, a: ExprId, b: ExprId) -> ExprId {
        let b = self.s.lift_loose(b, 0, 1);
        self.s.pi(a, b)
    }

    fn app2(&mut self, f: ExprId, a: ExprId, b: ExprId) -> ExprId {
        self.s.mk_app_n(f, &[a, b])
    }

    /// Returns whether `d` may take a primitive name (Lean's `checkPrimitiveDef`), and records a
    /// primitive whose equations check as computed natively (`validated_primitives`).
    ///
    /// In LOGOS's theory a primitive's name refuses nothing: a definition that does not check as
    /// the primitive is an ordinary one, computed by unfolding, and its literal rules read `Zero`
    /// and `Succ` only while they are `Nat`'s constructors (`Kernel::reads_nat_literal`). So a
    /// definition named `Succ` is admitted, and an impostor `Nat.div` is not computed natively.
    pub fn check_primitive_def(&mut self, d: &DefnDecl) -> KResult<bool> {
        let checked = self.check_primitive_form(d);
        if self.theory == super::tc::Theory::Logos {
            return Ok(matches!(checked, Ok(true)));
        }
        checked
    }

    fn check_primitive_form(&mut self, d: &DefnDecl) -> KResult<bool> {
        let n = d.header.name;
        if d.safety != Safety::Safe || !self.wk.primitives().contains(&n) {
            return Ok(false);
        }
        if !d.header.lparams.is_empty() {
            return Err(self.prim_fail(n));
        }
        let w = WellKnownIds::of(self);
        let nat = self.c0(w.nat);
        let bool_ = self.c0(w.bool_);
        let tru = self.c0(w.tru);
        let fal = self.c0(w.fal);
        let zero = self.c0(w.zero);
        let succ_c = self.c0(w.succ);
        let x = self.s.bvar(0);
        let y = self.s.bvar(1);
        let succ = |k: &mut Kernel, e: ExprId| k.s.app(succ_c, e);
        let nn = self.arrow(nat, nat);
        let nnn = self.arrow(nat, nn);
        let nb = self.arrow(nat, bool_);
        let nnb = self.arrow(nat, nb);
        let v = d.value;
        let ty = d.header.ty;
        let ok = if n == w.add {
            self.env.contains(w.nat)
                && self.is_def_eq(ty, nnn)?
                && {
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, x)?
                }
                && {
                    let sx = succ(self, x);
                    let lhs = self.app2(v, y, sx);
                    let r = self.app2(v, y, x);
                    let rhs = succ(self, r);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.pred {
            self.env.contains(w.nat)
                && self.is_def_eq(ty, nn)?
                && {
                    let lhs = self.s.app(v, zero);
                    self.is_def_eq(lhs, zero)?
                }
                && {
                    let sx = succ(self, x);
                    let lhs = self.s.app(v, sx);
                    self.defeq1(lhs, x)?
                }
        } else if n == w.sub {
            self.env.contains(w.pred)
                && self.is_def_eq(ty, nnn)?
                && {
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, x)?
                }
                && {
                    let pred = self.c0(w.pred);
                    let sx = succ(self, x);
                    let lhs = self.app2(v, y, sx);
                    let r = self.app2(v, y, x);
                    let rhs = self.s.app(pred, r);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.mul {
            self.env.contains(w.add)
                && self.is_def_eq(ty, nnn)?
                && {
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, zero)?
                }
                && {
                    let add = self.c0(w.add);
                    let sx = succ(self, x);
                    let lhs = self.app2(v, y, sx);
                    let r = self.app2(v, y, x);
                    let rhs = self.app2(add, r, y);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.pow {
            self.env.contains(w.mul)
                && self.is_def_eq(ty, nnn)?
                && {
                    let one = succ(self, zero);
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, one)?
                }
                && {
                    let mul = self.c0(w.mul);
                    let sx = succ(self, x);
                    let lhs = self.app2(v, y, sx);
                    let r = self.app2(v, y, x);
                    let rhs = self.app2(mul, r, y);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.beq || n == w.ble {
            let is_beq = n == w.beq;
            self.env.contains(w.nat)
                && self.env.contains(w.bool_)
                && self.is_def_eq(ty, nnb)?
                && {
                    let lhs = self.app2(v, zero, zero);
                    self.is_def_eq(lhs, tru)?
                }
                && {
                    let sx = succ(self, x);
                    let lhs = self.app2(v, zero, sx);
                    let expect = if is_beq { fal } else { tru };
                    self.defeq1(lhs, expect)?
                }
                && {
                    let sx = succ(self, x);
                    let lhs = self.app2(v, sx, zero);
                    self.defeq1(lhs, fal)?
                }
                && {
                    let sx = succ(self, x);
                    let sy = succ(self, y);
                    let lhs = self.app2(v, sy, sx);
                    let rhs = self.app2(v, y, x);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.shl {
            self.env.contains(w.mul)
                && self.is_def_eq(ty, nnn)?
                && {
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, x)?
                }
                && {
                    let mul = self.c0(w.mul);
                    let one = succ(self, zero);
                    let two = succ(self, one);
                    let sy = succ(self, y);
                    let lhs = self.app2(v, x, sy);
                    let tx = self.app2(mul, two, x);
                    let rhs = self.app2(v, tx, y);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.shr {
            self.env.contains(w.div)
                && self.is_def_eq(ty, nnn)?
                && {
                    let lhs = self.app2(v, x, zero);
                    self.defeq1(lhs, x)?
                }
                && {
                    let div = self.c0(w.div);
                    let one = succ(self, zero);
                    let two = succ(self, one);
                    let sy = succ(self, y);
                    let lhs = self.app2(v, x, sy);
                    let r = self.app2(v, x, y);
                    let rhs = self.app2(div, r, two);
                    self.defeq2(lhs, rhs)?
                }
        } else if n == w.land || n == w.lor || n == w.xor {
            self.env.contains(w.bitwise)
                && self.is_def_eq(ty, nnn)?
                && self.check_bitwise_op(n, v, &w)?
        } else if n == w.char_of_nat {
            let ch = self.c0(w.char_);
            self.env.contains(w.nat) && self.env.contains(w.char_) && {
                let t = self.arrow(nat, ch);
                self.is_def_eq(ty, t)?
            }
        } else if n == w.string_of_list {
            self.check_string_of_list(ty)?
        } else if n == w.mod_ || n == w.div || n == w.gcd || n == w.bitwise {
            let admitted = self.env.contains(w.nat) && {
                let expect = if n == w.bitwise {
                    let bb = self.arrow(bool_, bool_);
                    let bbb = self.arrow(bool_, bb);
                    self.arrow(bbb, nnn)
                } else {
                    nnn
                };
                self.is_def_eq(ty, expect)?
            };
            if admitted && self.check_wf_primitive(d)? && self.accelerable_after_validation(n) {
                self.validated_primitives.insert(n);
            }
            if !admitted {
                return Err(self.prim_fail(n));
            }
            return Ok(true);
        } else {
            return Ok(false);
        };
        if !ok {
            return Err(self.prim_fail(n));
        }
        if self.accelerable_after_validation(n) {
            self.validated_primitives.insert(n);
        }
        Ok(true)
    }

    /// Whether a primitive whose equations just checked may be computed natively. Its equations
    /// are stated in terms of other primitives (`Nat.mul` in terms of `Nat.add`, `Nat.land` as
    /// `Nat.bitwise and`), so they pin its meaning only once those are pinned too. A
    /// well-founded primitive whose unfolding could not be validated is still admitted by its
    /// type, and is then computed by unfolding rather than natively.
    fn accelerable_after_validation(&self, n: NameId) -> bool {
        let w = &self.wk;
        let deps: &[NameId] = if n == w.nat_mod || n == w.nat_div {
            &[w.nat_sub]
        } else if n == w.nat_gcd {
            &[w.nat_mod]
        } else if n == w.nat_bitwise {
            &[w.nat_div, w.nat_mod, w.nat_add]
        } else if n == w.nat_sub {
            &[w.nat_pred]
        } else if n == w.nat_mul || n == w.nat_shift_left {
            &[w.nat_add, w.nat_mul]
        } else if n == w.nat_pow {
            &[w.nat_add, w.nat_mul]
        } else if n == w.nat_shift_right {
            &[w.nat_div]
        } else if [w.nat_land, w.nat_lor, w.nat_xor].contains(&n) {
            &[w.nat_bitwise]
        } else {
            &[]
        };
        deps.iter()
            .all(|d| *d == n || self.validated_primitives.contains(d))
    }

    fn check_bitwise_op(&mut self, n: NameId, v: ExprId, w: &WellKnownIds) -> KResult<bool> {
        let super::expr::Node::App(f, op) = self.s.node(v) else {
            return Ok(false);
        };
        if f != self.c0(w.bitwise) {
            return Ok(false);
        }
        let tru = self.c0(w.tru);
        let fal = self.c0(w.fal);
        let x = self.s.bvar(0);
        let bool_ = self.c0(w.bool_);
        let under_bool = |k: &mut Kernel, a: ExprId, b: ExprId| -> KResult<bool> {
            let pa = k.s.pi(bool_, a);
            let pb = k.s.pi(bool_, b);
            k.is_def_eq(pa, pb)
        };
        let app = |k: &mut Kernel, a: ExprId, b: ExprId| k.s.mk_app_n(op, &[a, b]);
        Ok(if n == w.land {
            let a = app(self, fal, x);
            let b = app(self, tru, x);
            under_bool(self, a, fal)? && under_bool(self, b, x)?
        } else if n == w.lor {
            let a = app(self, fal, x);
            let b = app(self, tru, x);
            under_bool(self, a, x)? && under_bool(self, b, tru)?
        } else {
            let cases = [
                (fal, fal, fal),
                (tru, fal, tru),
                (fal, tru, tru),
                (tru, tru, fal),
            ];
            let mut all = true;
            for (a, b, r) in cases {
                let e = app(self, a, b);
                if !self.is_def_eq(e, r)? {
                    all = false;
                    break;
                }
            }
            all
        })
    }

    fn check_string_of_list(&mut self, ty: ExprId) -> KResult<bool> {
        let w = WellKnownIds::of(self);
        for needed in [w.char_, w.list, w.list_nil, w.list_cons, w.string] {
            if !self.env.contains(needed) {
                return Ok(false);
            }
        }
        let zero = self.s.levels.list(&[super::level::LevelId::ZERO]);
        let ch = self.c0(w.char_);
        let list_c = self.s.cnst(w.list, zero);
        let list_char = self.s.app(list_c, ch);
        let string = self.c0(w.string);
        let expect = self.arrow(list_char, string);
        let nil_c = self.s.cnst(w.list_nil, zero);
        let nil = self.s.app(nil_c, ch);
        let nil_t = self.check(nil)?;
        let cons_c = self.s.cnst(w.list_cons, zero);
        let cons = self.s.app(cons_c, ch);
        let cons_t = self.check(cons)?;
        let lc_lc = self.arrow(list_char, list_char);
        let cons_expect = self.arrow(ch, lc_lc);
        Ok(self.is_def_eq(nil_t, list_char)?
            && self.is_def_eq(cons_t, cons_expect)?
            && self.is_def_eq(ty, expect)?)
    }
}

/// The primitive names, copied out so validation can borrow the kernel mutably.
pub(crate) struct WellKnownIds {
    nat: NameId,
    zero: NameId,
    succ: NameId,
    add: NameId,
    pred: NameId,
    sub: NameId,
    mul: NameId,
    pow: NameId,
    div: NameId,
    mod_: NameId,
    gcd: NameId,
    beq: NameId,
    ble: NameId,
    shl: NameId,
    shr: NameId,
    land: NameId,
    lor: NameId,
    xor: NameId,
    bitwise: NameId,
    bool_: NameId,
    tru: NameId,
    fal: NameId,
    char_: NameId,
    char_of_nat: NameId,
    string: NameId,
    string_of_list: NameId,
    list: NameId,
    list_nil: NameId,
    list_cons: NameId,
}

impl WellKnownIds {
    fn of(k: &Kernel) -> WellKnownIds {
        let w = &k.wk;
        WellKnownIds {
            nat: w.nat,
            zero: w.nat_zero,
            succ: w.nat_succ,
            add: w.nat_add,
            pred: w.nat_pred,
            sub: w.nat_sub,
            mul: w.nat_mul,
            pow: w.nat_pow,
            div: w.nat_div,
            mod_: w.nat_mod,
            gcd: w.nat_gcd,
            beq: w.nat_beq,
            ble: w.nat_ble,
            shl: w.nat_shift_left,
            shr: w.nat_shift_right,
            land: w.nat_land,
            lor: w.nat_lor,
            xor: w.nat_xor,
            bitwise: w.nat_bitwise,
            bool_: w.bool_,
            tru: w.bool_true,
            fal: w.bool_false,
            char_: w.char_,
            char_of_nat: w.char_of_nat,
            string: w.string,
            string_of_list: w.string_of_list,
            list: w.list,
            list_nil: w.list_nil,
            list_cons: w.list_cons,
        }
    }
}
