//! Validating the well-founded `Nat` primitives: `Nat.div`, `Nat.mod`, `Nat.gcd`, `Nat.bitwise`.
//!
//! These are defined by fuel or by `WellFounded.fix`, so their defining equations are not
//! definitional for a symbolic argument the way `Nat.add`'s are. Following `lean4lean`'s
//! `Primitive.lean`, the kernel instead checks, over free variables:
//!
//! * the decision procedures they branch on (`Nat.decLe`, `Nat.decEq`, `Bool.decEq`) reflect their
//!   Boolean counterparts (`Nat.ble`, `Nat.beq`), so `if`/`dite` on them compute like the
//!   corresponding literal comparison;
//! * `Nat.div`/`Nat.mod` unfold, one fuel step at a time, to exactly the recursion of division;
//! * `Nat.gcd`/`Nat.bitwise` unfold through `WellFounded.Nat.fix` to the right-hand sides of their
//!   `eq_def` equations, whose fuel is always sufficient.
//!
//! The reference terms these checks are stated with were elaborated by the pinned Lean toolchain
//! (`primitive_refs/PrimitiveRefs.lean`), so no elaborated shape is reconstructed by hand.

use super::check::DefnDecl;
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Node};
use super::level::LevelsId;
use super::tc::Kernel;
use rustc_hash::FxHashMap as FxMap;

const REFS: &[u8] = include_bytes!("primitive_refs/lean-4.29.1.ndjson");

struct Refs(FxMap<String, ExprId>);

impl Refs {
    fn get(&self, k: &str) -> KResult<ExprId> {
        self.0.get(k).copied().ok_or_else(|| {
            KernelFailure::resource(format!("missing primitive reference term '{k}'"))
        })
    }
}

#[derive(Clone, Copy)]
struct Reflection {
    ty: ExprId,
    of_true: ExprId,
    of_false: ExprId,
    to_dec: ExprId,
}

#[derive(Clone, Copy)]
enum Impl {
    Bool,
    ReflectNatNat {
        as_bool: ExprId,
        reflect: Reflection,
        proof: ExprId,
    },
}

#[derive(Clone, Copy)]
struct Condition {
    prop: ExprId,
    dec: ExprId,
    imp: Impl,
}

/// The constants the checks are written with, interned once.
struct C {
    prop: ExprId,
    ty1: ExprId,
    nat: ExprId,
    bool_: ExprId,
    tru: ExprId,
    fal: ExprId,
    zero: ExprId,
    succ: ExprId,
    not: ExprId,
    ite1: ExprId,
    dite_nat: ExprId,
    ite_nat: ExprId,
}

struct Fail;

type V<T> = Result<T, Fail>;

impl Kernel {
    /// `Ok(true)` when the definition passes, `Ok(false)` when it does not have the canonical
    /// form (the primitive is then admitted by its type but never accelerated).
    pub(crate) fn check_wf_primitive(&mut self, d: &DefnDecl) -> KResult<bool> {
        let refs = Refs(super::export::load_refs(self, REFS)?);
        match self.check_wf_primitive_inner(d, &refs) {
            Ok(r) => Ok(r),
            Err(e) if e.is_reject() => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn check_wf_primitive_inner(&mut self, d: &DefnDecl, refs: &Refs) -> KResult<bool> {
        let c = self.prim_consts();
        let n = d.header.name;
        let w = (
            self.wk.nat_mod,
            self.wk.nat_div,
            self.wk.nat_gcd,
            self.wk.nat_bitwise,
        );
        let r = if n == w.0 {
            self.check_mod(d, refs, &c)?
        } else if n == w.1 {
            self.check_div(d, refs, &c)?
        } else if n == w.2 {
            self.check_gcd(d, refs, &c)?
        } else if n == w.3 {
            self.check_bitwise(d, refs, &c)?
        } else {
            return Ok(false);
        };
        Ok(r.is_ok())
    }

    fn prim_consts(&mut self) -> C {
        let one = self.s.levels.of_nat(1);
        let l1 = self.s.levels.list(&[one]);
        let e = LevelsId::EMPTY;
        let nat = self.s.cnst(self.wk.nat, e);
        let dite = self.s.names.dotted("dite");
        let ite = self.s.names.dotted("ite");
        let not = self.s.names.dotted("Not");
        let dite1 = self.s.cnst(dite, l1);
        let ite1 = self.s.cnst(ite, l1);
        C {
            prop: self.s.prop(),
            ty1: self.s.sort(one),
            nat,
            bool_: self.s.cnst(self.wk.bool_, e),
            tru: self.s.cnst(self.wk.bool_true, e),
            fal: self.s.cnst(self.wk.bool_false, e),
            zero: self.s.cnst(self.wk.nat_zero, e),
            succ: self.s.cnst(self.wk.nat_succ, e),
            not: self.s.cnst(not, e),
            ite1,
            dite_nat: self.s.app(dite1, nat),
            ite_nat: self.s.app(ite1, nat),
        }
    }

    fn c0n(&mut self, dotted: &str) -> ExprId {
        let n = self.s.names.dotted(dotted);
        self.s.cnst(n, LevelsId::EMPTY)
    }

    fn apps(&mut self, f: ExprId, args: &[ExprId]) -> ExprId {
        self.s.mk_app_n(f, args)
    }

    fn lam0(&mut self, t: ExprId, b: ExprId) -> ExprId {
        self.s.lam(t, b)
    }

    fn raw_arrow(&mut self, d: ExprId, b: ExprId) -> ExprId {
        self.s.pi(d, b)
    }

    fn reflection(&self, refs: &Refs, which: &str) -> KResult<Reflection> {
        Ok(Reflection {
            ty: refs.get(&format!("{which}.type"))?,
            of_true: refs.get(&format!("{which}.ofTrue"))?,
            of_false: refs.get(&format!("{which}.ofFalse"))?,
            to_dec: refs.get(&format!("{which}.toDec"))?,
        })
    }

    fn cond_nat_le(&mut self, refs: &Refs) -> KResult<Condition> {
        let as_bool = self.s.cnst(self.wk.nat_ble, LevelsId::EMPTY);
        Ok(Condition {
            prop: refs.get("natLE.prop")?,
            dec: refs.get("natLE.dec")?,
            imp: Impl::ReflectNatNat {
                as_bool,
                reflect: self.reflection(refs, "refl1")?,
                proof: refs.get("natLE.proof")?,
            },
        })
    }

    fn cond_nat_eq(&mut self, refs: &Refs) -> KResult<Condition> {
        let as_bool = self.s.cnst(self.wk.nat_beq, LevelsId::EMPTY);
        Ok(Condition {
            prop: refs.get("natEq.prop")?,
            dec: refs.get("natEq.dec")?,
            imp: Impl::ReflectNatNat {
                as_bool,
                reflect: self.reflection(refs, "refl2")?,
                proof: refs.get("natEq.proof")?,
            },
        })
    }

    fn cond_bool(&self, refs: &Refs) -> KResult<Condition> {
        Ok(Condition {
            prop: refs.get("bool.prop")?,
            dec: refs.get("bool.dec")?,
            imp: Impl::Bool,
        })
    }

    fn defeq(&mut self, a: ExprId, b: ExprId) -> KResult<V<()>> {
        Ok(if self.is_def_eq(a, b)? {
            Ok(())
        } else {
            Err(Fail)
        })
    }

    fn nat_nat_prop(&mut self, c: &C) -> ExprId {
        let np = self.raw_arrow(c.nat, c.prop);
        self.raw_arrow(c.nat, np)
    }

    fn reflection_check(&mut self, r: &Reflection, c: &C) -> KResult<V<()>> {
        let t = self.check(r.ty)?;
        let bp = self.raw_arrow(c.bool_, c.prop);
        let pbp = self.raw_arrow(c.prop, bp);
        self.defeq(t, pbp)
    }

    fn reflection_ite(&mut self, r: &Reflection, c: &C) -> ExprId {
        let (b0, b1, b2, b3) = (
            self.s.bvar(0),
            self.s.bvar(1),
            self.s.bvar(2),
            self.s.bvar(3),
        );
        let dec = self.apps(r.to_dec, &[b3, b2, b1]);
        let body = self.apps(c.ite1, &[b0, b3, dec]);
        let l4 = self.lam0(c.ty1, body);
        let h_ty = self.apps(r.ty, &[b1, b0]);
        let l3 = self.lam0(h_ty, l4);
        let l2 = self.lam0(c.bool_, l3);
        self.lam0(c.prop, l2)
    }

    fn reflection_nat_dite(&mut self, r: &Reflection, c: &C) -> ExprId {
        let (b0, b1, b2) = (self.s.bvar(0), self.s.bvar(1), self.s.bvar(2));
        let dec = self.apps(r.to_dec, &[b2, b1, b0]);
        let body = self.apps(c.dite_nat, &[b2, dec]);
        let h_ty = self.apps(r.ty, &[b1, b0]);
        let l3 = self.lam0(h_ty, body);
        let l2 = self.lam0(c.bool_, l3);
        self.lam0(c.prop, l2)
    }

    /// The body of `fun α a _ => a` (`first`) or `fun α _ a => a` under the binder `α`.
    fn select(&mut self, first: bool) -> ExprId {
        let (b0, b1) = (self.s.bvar(0), self.s.bvar(1));
        let pick = if first { b1 } else { b0 };
        let inner = self.lam0(b1, pick);
        self.lam0(b0, inner)
    }

    fn check_ite(&mut self, r: &Reflection, c: &C) -> KResult<V<()>> {
        let ite = self.reflection_ite(r, c);
        let t = self.check(ite)?;
        let (b0, b1, b2) = (self.s.bvar(0), self.s.bvar(1), self.s.bvar(2));
        let aaa = {
            let a_a = self.raw_arrow(b1, b2);
            let a_aa = self.raw_arrow(b0, a_a);
            self.s.pi(c.ty1, a_aa)
        };
        let h_ty = self.apps(r.ty, &[b1, b0]);
        let x3 = self.raw_arrow(h_ty, aaa);
        let x2 = self.raw_arrow(c.bool_, x3);
        let expect = self.raw_arrow(c.prop, x2);
        if self.defeq(t, expect)?.is_err() {
            return Ok(Err(Fail));
        }
        let p = self.push_local(c.prop, None);
        for (b, first) in [(c.tru, true), (c.fal, false)] {
            let hty = self.apps(r.ty, &[p, b]);
            let h = self.push_local(hty, None);
            let lhs = self.apps(ite, &[p, b, h]);
            let sel = self.select(first);
            let rhs = self.lam0(c.ty1, sel);
            if self.defeq(lhs, rhs)?.is_err() {
                return Ok(Err(Fail));
            }
        }
        Ok(Ok(()))
    }

    fn check_nat_dite(&mut self, r: &Reflection, c: &C) -> KResult<V<()>> {
        let not_t = self.check(c.not)?;
        let pp = self.raw_arrow(c.prop, c.prop);
        if self.defeq(not_t, pp)?.is_err() {
            return Ok(Err(Fail));
        }
        let nd = self.reflection_nat_dite(r, c);
        let t = self.check(nd)?;
        let (b0, b1, b2, b3) = (
            self.s.bvar(0),
            self.s.bvar(1),
            self.s.bvar(2),
            self.s.bvar(3),
        );
        let not_b3 = self.s.app(c.not, b3);
        let a5 = self.raw_arrow(not_b3, c.nat);
        let x5 = self.raw_arrow(a5, c.nat);
        let a4 = self.raw_arrow(b2, c.nat);
        let x4 = self.raw_arrow(a4, x5);
        let h_ty = self.apps(r.ty, &[b1, b0]);
        let x3 = self.raw_arrow(h_ty, x4);
        let x2 = self.raw_arrow(c.bool_, x3);
        let expect = self.raw_arrow(c.prop, x2);
        if self.defeq(t, expect)?.is_err() {
            return Ok(Err(Fail));
        }
        for (term, b, negate) in [(r.of_true, c.tru, false), (r.of_false, c.fal, true)] {
            let t = self.check(term)?;
            let rt = self.apps(r.ty, &[b0, b]);
            let concl = if negate { self.s.app(c.not, b1) } else { b1 };
            let x = self.raw_arrow(rt, concl);
            let expect = self.raw_arrow(c.prop, x);
            if self.defeq(t, expect)?.is_err() {
                return Ok(Err(Fail));
            }
        }
        let p = self.push_local(c.prop, None);
        let pn = self.raw_arrow(p, c.nat);
        let a = self.push_local(pn, None);
        let not_p = self.s.app(c.not, p);
        let npn = self.raw_arrow(not_p, c.nat);
        let bb = self.push_local(npn, None);
        for (bv, of, branch) in [(c.tru, r.of_true, a), (c.fal, r.of_false, bb)] {
            let hty = self.apps(r.ty, &[p, bv]);
            let h = self.push_local(hty, None);
            let lhs = self.apps(nd, &[p, bv, h, a, bb]);
            let proof = self.apps(of, &[p, h]);
            let rhs = self.s.app(branch, proof);
            if self.defeq(lhs, rhs)?.is_err() {
                return Ok(Err(Fail));
            }
        }
        Ok(Ok(()))
    }

    fn condition_check(
        &mut self,
        cond: &Condition,
        c: &C,
        ite: bool,
        dite: bool,
    ) -> KResult<V<()>> {
        self.check(cond.dec)?;
        match cond.imp {
            Impl::ReflectNatNat {
                as_bool,
                reflect,
                proof,
            } => {
                let pt = self.infer(cond.prop, true)?;
                let nnp = self.nat_nat_prop(c);
                if self.defeq(pt, nnp)?.is_err() || self.reflection_check(&reflect, c)?.is_err() {
                    return Ok(Err(Fail));
                }
                if ite && self.check_ite(&reflect, c)?.is_err() {
                    return Ok(Err(Fail));
                }
                if dite && self.check_nat_dite(&reflect, c)?.is_err() {
                    return Ok(Err(Fail));
                }
                let (y, x) = (self.s.bvar(0), self.s.bvar(1));
                let p = self.apps(cond.prop, &[x, y]);
                let b = self.apps(as_bool, &[x, y]);
                let pr = self.apps(proof, &[x, y]);
                let body = self.apps(reflect.to_dec, &[p, b, pr]);
                let l = self.lam0(c.nat, body);
                let e = self.lam0(c.nat, l);
                self.check(e)?;
                let bt = self.infer(as_bool, true)?;
                let nb = self.raw_arrow(c.nat, c.bool_);
                let nnb = self.raw_arrow(c.nat, nb);
                if self.defeq(bt, nnb)?.is_err() {
                    return Ok(Err(Fail));
                }
                let prt = self.infer(proof, true)?;
                if !self.is_prop(prt)? {
                    return Ok(Err(Fail));
                }
                self.defeq(e, cond.dec)
            }
            Impl::Bool => {
                let pt = self.infer(cond.prop, true)?;
                let bp = self.raw_arrow(c.bool_, c.prop);
                if self.defeq(pt, bp)?.is_err() {
                    return Ok(Err(Fail));
                }
                if dite {
                    return Ok(Err(Fail));
                }
                if ite {
                    let b = self.s.bvar(0);
                    let pb = self.s.app(cond.prop, b);
                    let db = self.s.app(cond.dec, b);
                    let body = self.apps(c.ite_nat, &[pb, db]);
                    let nat_ite = self.lam0(c.bool_, body);
                    let t = self.check(nat_ite)?;
                    let nn = self.raw_arrow(c.nat, c.nat);
                    let nnn = self.raw_arrow(c.nat, nn);
                    let expect = self.raw_arrow(c.bool_, nnn);
                    if self.defeq(t, expect)?.is_err() {
                        return Ok(Err(Fail));
                    }
                    for (bv, first) in [(c.tru, true), (c.fal, false)] {
                        let lhs = self.s.app(nat_ite, bv);
                        let (b0, b1) = (self.s.bvar(0), self.s.bvar(1));
                        let pick = if first { b1 } else { b0 };
                        let inner = self.lam0(c.nat, pick);
                        let rhs = self.lam0(c.nat, inner);
                        if self.defeq(lhs, rhs)?.is_err() {
                            return Ok(Err(Fail));
                        }
                    }
                }
                Ok(Ok(()))
            }
        }
    }

    fn cond_ite(
        &mut self,
        cond: &Condition,
        c: &C,
        alpha: ExprId,
        args: &[ExprId],
        t: ExprId,
        e: ExprId,
    ) -> ExprId {
        let p = self.apps(cond.prop, args);
        let d = self.apps(cond.dec, args);
        self.apps(c.ite1, &[alpha, p, d, t, e])
    }

    fn cond_dite(
        &mut self,
        cond: &Condition,
        c: &C,
        args: &[ExprId],
        t: ExprId,
        e: ExprId,
    ) -> ExprId {
        let p = self.apps(cond.prop, args);
        let d = self.apps(cond.dec, args);
        let tl = self.lam0(p, t);
        let np = self.s.app(c.not, p);
        let el = self.lam0(np, e);
        self.apps(c.dite_nat, &[p, d, tl, el])
    }

    fn cond_decide(&mut self, cond: &Condition, c: &C, args: &[ExprId]) -> ExprId {
        self.cond_ite(cond, c, c.bool_, args, c.tru, c.fal)
    }

    fn nat3(&mut self, c: &C) -> ExprId {
        let nn = self.raw_arrow(c.nat, c.nat);
        self.raw_arrow(c.nat, nn)
    }

    fn defeq1n(&mut self, c: &C, a: ExprId, b: ExprId) -> KResult<bool> {
        let pa = self.raw_arrow(c.nat, a);
        let pb = self.raw_arrow(c.nat, b);
        self.is_def_eq(pa, pb)
    }

    fn check_div_mod_common(
        &mut self,
        d: &DefnDecl,
        refs: &Refs,
        c: &C,
        go_name: &str,
    ) -> KResult<V<(ExprId, Condition)>> {
        let sub = self.wk.nat_sub;
        if !self.env.contains(sub) || !self.env.contains(self.wk.bool_) {
            return Ok(Err(Fail));
        }
        let n3 = self.nat3(c);
        if self.defeq(d.header.ty, n3)?.is_err() {
            return Ok(Err(Fail));
        }
        let le = refs.get("natLE.prop")?;
        let le_t = self.check(le)?;
        let nnp = refs.get("natLE.type")?;
        if self.defeq(le_t, nnp)?.is_err() {
            return Ok(Err(Fail));
        }
        let go = self.c0n(go_name);
        if self.env.get(self.s.const_head(go).unwrap().0).is_none() {
            return Ok(Err(Fail));
        }
        let go_t = self.check(go)?;
        let go_ty = refs.get("goType")?;
        if self.defeq(go_t, go_ty)?.is_err() {
            return Ok(Err(Fail));
        }
        let cond = self.cond_nat_le(refs)?;
        Ok(Ok((go, cond)))
    }

    fn check_mod(&mut self, d: &DefnDecl, refs: &Refs, c: &C) -> KResult<V<()>> {
        let (go, cond) = match self.check_div_mod_common(d, refs, c, "Nat.modCore.go")? {
            Ok(x) => x,
            Err(f) => return Ok(Err(f)),
        };
        let v = d.value;
        let x = self.s.bvar(0);
        let m0 = self.apps(v, &[c.zero, x]);
        if !self.defeq1n(c, m0, c.zero)? {
            return Ok(Err(Fail));
        }
        if self.condition_check(&cond, c, true, true)?.is_err() {
            return Ok(Err(Fail));
        }
        let lt_succ_self = self.c0n("Nat.lt_succ_self");
        let fuel_lemma = self.c0n("Nat.div_rec_fuel_lemma");
        let sub = self.s.cnst(self.wk.nat_sub, LevelsId::EMPTY);
        let le = refs.get("natLE.prop")?;
        let one = self.s.app(c.succ, c.zero);
        let x = self.push_local(c.nat, None);
        let y = self.push_local(c.nat, None);
        let sx = self.s.app(c.succ, x);
        let ssx = self.s.app(c.succ, sx);
        let lt = self.s.app(lt_succ_self, sx);
        let h0 = self.s.bvar(0);
        let go_app = self.apps(go, &[y, h0, ssx, sx, lt]);
        let inner = self.cond_dite(&cond, c, &[one, y], go_app, sx);
        let e = self.cond_ite(&cond, c, c.nat, &[y, sx], inner, sx);
        self.check(e)?;
        let lhs = self.apps(v, &[sx, y]);
        if self.defeq(lhs, e)?.is_err() {
            return Ok(Err(Fail));
        }
        let hy_ty = self.apps(le, &[one, y]);
        let hy = self.push_local(hy_ty, None);
        let fuel = self.push_local(c.nat, None);
        let sfuel = self.s.app(c.succ, fuel);
        let h_ty = self.apps(le, &[sx, sfuel]);
        let h = self.push_local(h_ty, None);
        let xy = self.apps(sub, &[x, y]);
        let lem = self.apps(fuel_lemma, &[x, y, fuel, hy, h0, h]);
        let rec = self.apps(go, &[y, hy, fuel, xy, lem]);
        let e = self.cond_dite(&cond, c, &[y, x], rec, x);
        self.check(e)?;
        let lhs = self.apps(go, &[y, hy, sfuel, x, h]);
        self.defeq(lhs, e)
    }

    fn check_div(&mut self, d: &DefnDecl, refs: &Refs, c: &C) -> KResult<V<()>> {
        let sub_ok = self.env.contains(self.wk.nat_sub) && self.env.contains(self.wk.bool_);
        if !sub_ok {
            return Ok(Err(Fail));
        }
        let n3 = self.nat3(c);
        if self.defeq(d.header.ty, n3)?.is_err() {
            return Ok(Err(Fail));
        }
        let cond = self.cond_nat_le(refs)?;
        if self.condition_check(&cond, c, false, true)?.is_err() {
            return Ok(Err(Fail));
        }
        let (go, _) = match self.check_div_mod_common(d, refs, c, "Nat.div.go")? {
            Ok(x) => x,
            Err(f) => return Ok(Err(f)),
        };
        let v = d.value;
        let lt_succ_self = self.c0n("Nat.lt_succ_self");
        let fuel_lemma = self.c0n("Nat.div_rec_fuel_lemma");
        let sub = self.s.cnst(self.wk.nat_sub, LevelsId::EMPTY);
        let le = refs.get("natLE.prop")?;
        let one = self.s.app(c.succ, c.zero);
        let h0 = self.s.bvar(0);
        let x = self.push_local(c.nat, None);
        let y = self.push_local(c.nat, None);
        let sx = self.s.app(c.succ, x);
        let lt = self.s.app(lt_succ_self, x);
        let go_app = self.apps(go, &[y, h0, sx, x, lt]);
        let e = self.cond_dite(&cond, c, &[one, y], go_app, c.zero);
        self.check(e)?;
        let lhs = self.apps(v, &[x, y]);
        if self.defeq(lhs, e)?.is_err() {
            return Ok(Err(Fail));
        }
        let hy_ty = self.apps(le, &[one, y]);
        let hy = self.push_local(hy_ty, None);
        let fuel = self.push_local(c.nat, None);
        let sfuel = self.s.app(c.succ, fuel);
        let h_ty = self.apps(le, &[sx, sfuel]);
        let h = self.push_local(h_ty, None);
        let xy = self.apps(sub, &[x, y]);
        let lem = self.apps(fuel_lemma, &[x, y, fuel, hy, h0, h]);
        let rec = self.apps(go, &[y, hy, fuel, xy, lem]);
        let srec = self.s.app(c.succ, rec);
        let e = self.cond_dite(&cond, c, &[y, x], srec, c.zero);
        self.check(e)?;
        let lhs = self.apps(go, &[y, hy, sfuel, x, h]);
        self.defeq(lhs, e)
    }

    fn check_gcd(&mut self, d: &DefnDecl, refs: &Refs, c: &C) -> KResult<V<()>> {
        if !self.env.contains(self.wk.nat_mod) {
            return Ok(Err(Fail));
        }
        let n3 = self.nat3(c);
        if self.defeq(d.header.ty, n3)?.is_err() {
            return Ok(Err(Fail));
        }
        let m = self.push_local(c.nat, None);
        let n = self.push_local(c.nat, None);
        let eq_def = refs.get("gcd.eq_def")?;
        let gcd2 = match self.unfold_nat_well_founded(d.value, &[m, n], eq_def, refs, c)? {
            Ok(g) => g,
            Err(f) => return Ok(Err(f)),
        };
        let lhs = self.apps(gcd2, &[c.zero, m]);
        if self.defeq(lhs, m)?.is_err() {
            return Ok(Err(Fail));
        }
        let modc = self.s.cnst(self.wk.nat_mod, LevelsId::EMPTY);
        let sn = self.s.app(c.succ, n);
        let lhs = self.apps(gcd2, &[sn, m]);
        let mm = self.apps(modc, &[m, sn]);
        let rhs = self.apps(d.value, &[mm, sn]);
        self.defeq(lhs, rhs)
    }

    fn check_bitwise(&mut self, d: &DefnDecl, refs: &Refs, c: &C) -> KResult<V<()>> {
        if !self.env.contains(self.wk.nat) || !self.env.contains(self.wk.bool_) {
            return Ok(Err(Fail));
        }
        let bb = self.raw_arrow(c.bool_, c.bool_);
        let bbb = self.raw_arrow(c.bool_, bb);
        let n3 = self.nat3(c);
        let expect = self.raw_arrow(bbb, n3);
        if self.defeq(d.header.ty, expect)?.is_err() {
            return Ok(Err(Fail));
        }
        let f = self.push_local(bbb, None);
        let n = self.push_local(c.nat, None);
        let m = self.push_local(c.nat, None);
        let eq_def = refs.get("bitwise.eq_def")?;
        let bw2 = match self.unfold_nat_well_founded(d.value, &[f, n, m], eq_def, refs, c)? {
            Ok(g) => g,
            Err(fl) => return Ok(Err(fl)),
        };
        let ce = self.cond_nat_eq(refs)?;
        if self.condition_check(&ce, c, true, false)?.is_err() {
            return Ok(Err(Fail));
        }
        let bc = self.cond_bool(refs)?;
        if self.condition_check(&bc, c, true, false)?.is_err() {
            return Ok(Err(Fail));
        }
        let div = self.s.cnst(self.wk.nat_div, LevelsId::EMPTY);
        let modc = self.s.cnst(self.wk.nat_mod, LevelsId::EMPTY);
        let add = self.s.cnst(self.wk.nat_add, LevelsId::EMPTY);
        let one = self.s.app(c.succ, c.zero);
        let two = self.s.app(c.succ, one);
        let f_ft = self.apps(f, &[c.fal, c.tru]);
        let t1 = self.cond_ite(&bc, c, c.nat, &[f_ft], m, c.zero);
        let f_tf = self.apps(f, &[c.tru, c.fal]);
        let t2 = self.cond_ite(&bc, c, c.nat, &[f_tf], n, c.zero);
        let n2 = self.apps(div, &[n, two]);
        let m2 = self.apps(div, &[m, two]);
        let nmod = self.apps(modc, &[n, two]);
        let mmod = self.apps(modc, &[m, two]);
        let b1 = self.cond_decide(&ce, c, &[nmod, one]);
        let b2 = self.cond_decide(&ce, c, &[mmod, one]);
        let r = self.apps(d.value, &[f, n2, m2]);
        let rr = self.apps(add, &[r, r]);
        let rr1 = self.apps(add, &[rr, one]);
        let fb = self.apps(f, &[b1, b2]);
        let last = self.cond_ite(&bc, c, c.nat, &[fb], rr1, rr);
        let inner = self.cond_ite(&ce, c, c.nat, &[m, c.zero], t2, last);
        let e = self.cond_ite(&ce, c, c.nat, &[n, c.zero], t1, inner);
        self.check(e)?;
        let lhs = self.apps(bw2, &[f, n, m]);
        self.defeq(lhs, e)
    }

    fn lambda_telescope(&mut self, mut e: ExprId) -> (Vec<ExprId>, ExprId) {
        let mut fvs = Vec::new();
        while let Node::Lam(d, b) = self.s.node(e) {
            let dt = self.s.instantiate_rev(d, &fvs);
            fvs.push(self.push_local(dt, None));
            e = b;
        }
        let body = self.s.instantiate_rev(e, &fvs);
        (fvs, body)
    }

    fn forall_telescope(&mut self, mut e: ExprId) -> (Vec<ExprId>, ExprId) {
        let mut fvs = Vec::new();
        while let Node::Pi(d, b) = self.s.node(e) {
            let dt = self.s.instantiate_rev(d, &fvs);
            fvs.push(self.push_local(dt, None));
            e = b;
        }
        let body = self.s.instantiate_rev(e, &fvs);
        (fvs, body)
    }

    fn replace_expr(
        &mut self,
        e: ExprId,
        from: ExprId,
        to: ExprId,
        memo: &mut FxMap<ExprId, ExprId>,
    ) -> ExprId {
        if e == from {
            return to;
        }
        if let Some(r) = memo.get(&e) {
            return *r;
        }
        let r = match self.s.node(e) {
            Node::App(f, a) => {
                let (f2, a2) = (
                    self.replace_expr(f, from, to, memo),
                    self.replace_expr(a, from, to, memo),
                );
                self.s.app(f2, a2)
            }
            Node::Lam(t, b) => {
                let (t2, b2) = (
                    self.replace_expr(t, from, to, memo),
                    self.replace_expr(b, from, to, memo),
                );
                self.s.lam(t2, b2)
            }
            Node::Pi(t, b) => {
                let (t2, b2) = (
                    self.replace_expr(t, from, to, memo),
                    self.replace_expr(b, from, to, memo),
                );
                self.s.pi(t2, b2)
            }
            Node::Let(t, v, b) => {
                let t2 = self.replace_expr(t, from, to, memo);
                let v2 = self.replace_expr(v, from, to, memo);
                let b2 = self.replace_expr(b, from, to, memo);
                self.s.let_(t2, v2, b2)
            }
            Node::Proj(n, i, x) => {
                let x2 = self.replace_expr(x, from, to, memo);
                self.s.proj(n, i, x2)
            }
            _ => e,
        };
        memo.insert(e, r);
        r
    }

    /// lean4lean's `unfoldNatWellFounded`: shows the definition `e`, applied to `fvs`, unfolds
    /// through `WellFounded.Nat.fix` to the right-hand side of its `eq_def`, and returns that
    /// right-hand side abstracted over `fvs` (with the recursive constant replaced by `e`).
    fn unfold_nat_well_founded(
        &mut self,
        e: ExprId,
        fvs: &[ExprId],
        eq_def: ExprId,
        _refs: &Refs,
        c: &C,
    ) -> KResult<V<ExprId>> {
        let mut body = eq_def;
        while let Node::Pi(_, b) = self.s.node(body) {
            body = b;
        }
        let body = self.s.instantiate_rev(body, fvs);
        let Node::App(eq_l, rhs) = self.s.node(body) else {
            return Ok(Err(Fail));
        };
        let Node::App(_, lhs) = self.s.node(eq_l) else {
            return Ok(Err(Fail));
        };
        let orig = self.s.app_fn(lhs);
        let rhs = self.replace_expr(rhs, orig, e, &mut FxMap::default());
        let applied = self.apps(e, fvs);
        let e1 = self.whnf_core(applied, false)?;
        let Some(e1) = self.unfold_definition(e1) else {
            return Ok(Err(Fail));
        };
        let e1w = self.whnf_core(e1, false)?;
        let (fix, args) = self.s.app_args(e1w);
        let fix_name = self.s.names.dotted("WellFounded.Nat.fix");
        let Node::Const(fixn, fix_ls) = self.s.node(fix) else {
            return Ok(Err(Fail));
        };
        if fixn != fix_name || self.s.levels.get_list(fix_ls).len() != 2 || args.len() != 5 {
            return Ok(Err(Fail));
        }
        let (alpha, motive, f, big_f, a0) = (args[0], args[1], args[2], args[3], args[4]);
        let fix_fn = self.apps(fix, &[alpha, motive, f, big_f]);
        {
            let a0t = self.infer(a0, true)?;
            let a = self.push_local(a0t, None);
            let fa = self.s.app(fix_fn, a);
            let Some(u) = self.unfold_definition(fa) else {
                return Ok(Err(Fail));
            };
            let e1 = self.whnf_core(u, false)?;
            let (fix_go, gargs) = self.s.app_args(e1);
            if gargs.len() != 7
                || (gargs[0], gargs[1], gargs[2], gargs[3], gargs[5])
                    != (alpha, motive, f, big_f, a)
            {
                return Ok(Err(Fail));
            }
            let fuel = gargs[4];
            let Node::App(eager, nn) = self.s.node(fuel) else {
                return Ok(Err(Fail));
            };
            let fa2 = self.s.app(f, a);
            let sfa = self.s.app(c.succ, fa2);
            if !self.is_def_eq(nn, sfa)? {
                return Ok(Err(Fail));
            }
            if !self.env.contains(self.wk.nat_beq) {
                return Ok(Err(Fail));
            }
            let bc = self.cond_bool(_refs)?;
            if self.condition_check(&bc, c, true, false)?.is_err() {
                return Ok(Err(Fail));
            }
            let x = self.s.bvar(0);
            let beq = self.s.cnst(self.wk.nat_beq, LevelsId::EMPTY);
            let lhs = self.s.app(eager, x);
            let bxx = self.apps(beq, &[x, x]);
            let rhs_e = self.cond_ite(&bc, c, c.nat, &[bxx], x, x);
            if !self.defeq1n(c, lhs, rhs_e)? {
                return Ok(Err(Fail));
            }
            let go2 = self.unfold_definition(fix_go).unwrap_or(fix_go);
            let (gfvs, go_body) = self.lambda_telescope(go2);
            if gfvs.len() != 5 {
                return Ok(Err(Fail));
            }
            let (gf, t) = (gfvs[3], gfvs[4]);
            let Node::App(nat_rec, t2) = self.s.node(go_body) else {
                return Ok(Err(Fail));
            };
            let t_idx = self.fvar_index(t);
            let mentions_t = self.s.any_node(
                nat_rec,
                &mut |s, n| matches!(s.node(n), Node::FVar(i) if i == t_idx),
            );
            if mentions_t || t != t2 {
                return Ok(Err(Fail));
            }
            let st = self.s.app(c.succ, t);
            self.check(st)?;
            let nr_st = self.s.app(nat_rec, st);
            let gor = self.whnf_core(nr_st, false)?;
            let (xfvs, gor_body) = self.lambda_telescope(gor);
            if xfvs.len() != 2 {
                return Ok(Err(Fail));
            }
            let Node::App(fx, ih) = self.s.node(gor_body) else {
                return Ok(Err(Fail));
            };
            let expect_fx = self.s.app(gf, xfvs[0]);
            if fx != expect_fx {
                return Ok(Err(Fail));
            }
            let (yfvs, ih_body) = self.lambda_telescope(ih);
            if yfvs.len() != 2 {
                return Ok(Err(Fail));
            }
            let Node::App(ih_head, _) = self.s.node(ih_body) else {
                return Ok(Err(Fail));
            };
            let nrt = self.s.app(nat_rec, t);
            let expect = self.s.app(nrt, yfvs[0]);
            if ih_head != expect {
                return Ok(Err(Fail));
            }
        }
        let fa0 = self.s.app(big_f, a0);
        let ft = self.infer(fa0, true)?;
        let Node::Pi(dom, _) = self.s.node(ft) else {
            return Ok(Err(Fail));
        };
        let (dfvs, _) = self.forall_telescope(dom);
        if dfvs.len() != 2 {
            return Ok(Err(Fail));
        }
        let fy = self.s.app(fix_fn, dfvs[0]);
        let ih2 = self.mk_lambda(&dfvs, fy);
        let rhs2 = self.apps(big_f, &[a0, ih2]);
        self.check(rhs2)?;
        if !self.is_def_eq(rhs, rhs2)? {
            return Ok(Err(Fail));
        }
        Ok(Ok(self.mk_lambda(fvs, rhs)))
    }
}
