//! Admitting declarations: the kernel's only door into the environment.

use super::env::{Const, Hint, Kind, QuotKind, Safety};
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Node};
use super::level::{LevelId, LevelsId};
use super::name::NameId;
use super::tc::{Engine, Kernel, Theory};
use super::wellknown::QuotNames;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct Header {
    pub name: NameId,
    pub lparams: Rc<[NameId]>,
    pub ty: ExprId,
}

#[derive(Clone, Debug)]
pub struct DefnDecl {
    pub header: Header,
    pub value: ExprId,
    pub hint: Hint,
    pub safety: Safety,
}

#[derive(Clone, Debug)]
pub enum Decl {
    Axiom {
        header: Header,
        is_unsafe: bool,
    },
    Defn(DefnDecl),
    Thm {
        header: Header,
        value: ExprId,
    },
    Opaque {
        header: Header,
        value: ExprId,
        is_unsafe: bool,
    },
    Mutual(Vec<DefnDecl>),
    Quot {
        header: Header,
        kind: QuotKind,
    },
}

impl Decl {
    /// The declared name (a mutual block is named by its first member).
    pub fn name(&self) -> NameId {
        match self {
            Decl::Axiom { header, .. }
            | Decl::Thm { header, .. }
            | Decl::Opaque { header, .. }
            | Decl::Quot { header, .. } => header.name,
            Decl::Defn(d) => d.header.name,
            Decl::Mutual(ds) => ds.first().map_or(NameId::ANON, |d| d.header.name),
        }
    }

    /// The export's record kind for this declaration (`mutual` for a block held until complete).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Decl::Axiom { .. } => "axiom",
            Decl::Defn(_) => "def",
            Decl::Thm { .. } => "thm",
            Decl::Opaque { .. } => "opaque",
            Decl::Mutual(_) => "mutual",
            Decl::Quot { .. } => "quot",
        }
    }
}

impl Kernel {
    pub fn check_name(&self, n: NameId, allow_primitive: bool) -> KResult<()> {
        if self.env.contains(n) {
            return Err(KernelFailure::reject(format!(
                "'{}' has already been declared",
                self.s.names.display(n)
            )));
        }
        if !allow_primitive && self.theory == Theory::Lean && self.wk.primitives().contains(&n) {
            return Err(KernelFailure::reject(format!(
                "unexpected declaration of the primitive '{}' in a non-canonical form",
                self.s.names.display(n)
            )));
        }
        Ok(())
    }

    pub fn check_duplicate_lparams(&self, ps: &[NameId]) -> KResult<()> {
        for (i, p) in ps.iter().enumerate() {
            if ps[i + 1..].contains(p) {
                return Err(KernelFailure::reject(format!(
                    "duplicate universe level parameter '{}'",
                    self.s.names.display(*p)
                )));
            }
        }
        Ok(())
    }

    fn check_closed(&self, e: ExprId) -> KResult<()> {
        if self.s.has_fvar(e) || self.s.has_loose_bvars(e) {
            return Err(KernelFailure::reject(
                "declaration contains free or loose bound variables",
            ));
        }
        Ok(())
    }

    /// Checks a declaration's header; the universe level of its type.
    fn check_constant_val(&mut self, h: &Header, allow_primitive: bool) -> KResult<LevelId> {
        self.check_name(h.name, allow_primitive)?;
        self.check_duplicate_lparams(&h.lparams)?;
        self.check_closed(h.ty)?;
        match self.engine {
            Engine::Sem => self.with_sem(|sem| sem.check_type(h.ty)),
            Engine::Classic | Engine::Dual => {
                let sort = self.check(h.ty)?;
                let sort = self.ensure_sort(sort)?;
                Ok(self.sort_level(sort))
            }
        }
    }

    fn check_value(&mut self, h: &Header, value: ExprId) -> KResult<()> {
        self.check_closed(value)?;
        if self.engine == Engine::Sem {
            return self.with_sem(|sem| sem.check_value(value, h.ty));
        }
        if self.theory == Theory::Logos && self.check_fix_against(value, h.ty)? {
            return Ok(());
        }
        let vt = self.check(value)?;
        if !self.conforms(vt, h.ty)? {
            return Err(
                KernelFailure::reject("declaration type mismatch").with_detail(format!(
                    "the value has type\n    {}\n  but is declared with type\n    {}",
                    self.show(vt),
                    self.show(h.ty)
                )),
            );
        }
        Ok(())
    }

    /// Runs `f` as one declaration's check: fresh caches and local context, scratch memory
    /// released afterwards whatever the outcome.
    pub fn with_decl<T>(
        &mut self,
        lparams: Rc<[NameId]>,
        safety: Safety,
        f: impl FnOnce(&mut Kernel) -> KResult<T>,
    ) -> KResult<T> {
        self.begin_decl(lparams, safety);
        let r = f(self);
        self.end_decl();
        r
    }

    /// Whether `d` may be admitted unchecked in a trusted-prefix run (see
    /// [`super::export::Observe::trust_before`]): anything but a primitive or the quotient.
    pub fn may_trust(&self, d: &Decl) -> bool {
        !matches!(d, Decl::Quot { .. }) && !self.wk.primitives().contains(&d.name())
    }

    /// Whether an inductive block declaring `claimed` may be admitted unchecked: none of its
    /// constants is a primitive.
    pub fn may_trust_block(&self, claimed: &[Const]) -> bool {
        let prims = self.wk.primitives();
        claimed.iter().all(|c| !prims.contains(&c.name))
    }

    /// Admits `d` as the export states it, without checking it (trusted-prefix runs only).
    pub fn admit_trusted(&mut self, d: Decl) {
        self.admit(d);
    }

    /// Admits an inductive block's constants as the export states them, without checking them
    /// (trusted-prefix runs only).
    pub fn admit_trusted_block(&mut self, claimed: Vec<Const>) {
        for c in claimed {
            self.env.add(c);
        }
    }

    /// Everything [`Kernel::add_decl`] checks, admitting nothing.
    #[cfg(feature = "shadow")]
    pub(crate) fn check_without_admitting(&mut self, decl: &Decl) -> KResult<()> {
        let name = decl.name();
        self.check_decl(decl)
            .map_err(|e| e.in_declaration(self.s.names.display(name)))
    }

    pub fn add_decl(&mut self, decl: Decl) -> KResult<()> {
        let name = decl.name();
        self.add_decl_inner(decl)
            .map_err(|e| e.in_declaration(self.s.names.display(name)))
    }

    fn add_decl_inner(&mut self, decl: Decl) -> KResult<()> {
        if let Decl::Quot { header, kind } = decl {
            return self.add_quot(header, kind, self.wk.eq);
        }
        if self.engine == Engine::Dual {
            self.check_decl_by_both(&decl)?;
        } else {
            self.check_decl(&decl)?;
        }
        self.admit(decl);
        Ok(())
    }

    /// Checks a declaration by BOTH engines, which must agree: the value engine first, then the
    /// substitution engine in declaration scopes of its own. A disagreement is never resolved
    /// towards acceptance.
    fn check_decl_by_both(&mut self, decl: &Decl) -> KResult<()> {
        self.engine = Engine::Sem;
        let by_value = self.check_decl(decl);
        self.engine = Engine::Classic;
        let by_substitution = self.check_decl(decl);
        self.engine = Engine::Dual;
        let verdict = |r: &KResult<()>| match r {
            Ok(()) => "accepts",
            Err(e) if e.is_reject() => "rejects",
            Err(_) => "declines",
        };
        match (by_value, by_substitution) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(v), Err(s)) if v.is_reject() && s.is_reject() => Err(s),
            (Err(e), _) | (_, Err(e)) if !e.is_reject() => Err(e),
            (v, s) => Err(KernelFailure::resource(format!(
                "the two engines disagree: evaluation {}, substitution {}",
                verdict(&v),
                verdict(&s)
            ))
            .with_detail(match (v, s) {
                (Err(e), _) | (_, Err(e)) => e.to_string(),
                _ => unreachable!(),
            })),
        }
    }

    /// Everything a declaration must satisfy, without admitting it (any provisional declarations
    /// a check needs are withdrawn again before it returns).
    fn check_decl(&mut self, decl: &Decl) -> KResult<()> {
        match decl {
            Decl::Axiom { header, is_unsafe } => {
                let safety = if *is_unsafe {
                    Safety::Unsafe
                } else {
                    Safety::Safe
                };
                self.with_decl(header.lparams.clone(), safety, |k| {
                    k.check_constant_val(header, false)
                })?;
                Ok(())
            }
            Decl::Defn(d) => self.check_definition(d),
            Decl::Thm { header, value } => {
                self.with_decl(header.lparams.clone(), Safety::Safe, |k| {
                    let level = k.check_constant_val(header, false)?;
                    if !k.s.levels.is_always_zero(level) {
                        return Err(KernelFailure::reject("theorem type is not a proposition")
                            .with_detail(format!("its type is {}", k.show(header.ty))));
                    }
                    k.check_value(header, *value)
                })
            }
            Decl::Opaque { header, value, .. } => {
                self.with_decl(header.lparams.clone(), Safety::Safe, |k| {
                    k.check_constant_val(header, false)?;
                    k.check_value(header, *value)
                })
            }
            Decl::Mutual(ds) => self.check_mutual(ds),
            Decl::Quot { .. } => unreachable!("quotients are admitted by add_quot"),
        }
    }

    fn admit(&mut self, decl: Decl) {
        let defn = |d: DefnDecl| Const {
            name: d.header.name,
            lparams: d.header.lparams,
            ty: d.header.ty,
            kind: Kind::Defn {
                value: d.value,
                hint: d.hint,
                safety: d.safety,
            },
        };
        match decl {
            Decl::Axiom { header, is_unsafe } => self.env.add(Const {
                name: header.name,
                lparams: header.lparams,
                ty: header.ty,
                kind: Kind::Axiom { is_unsafe },
            }),
            Decl::Defn(d) => self.env.add(defn(d)),
            Decl::Thm { header, value } => self.env.add(Const {
                name: header.name,
                lparams: header.lparams,
                ty: header.ty,
                kind: Kind::Thm { value },
            }),
            Decl::Opaque {
                header,
                value,
                is_unsafe,
            } => self.env.add(Const {
                name: header.name,
                lparams: header.lparams,
                ty: header.ty,
                kind: Kind::Opaque { value, is_unsafe },
            }),
            Decl::Mutual(ds) => {
                for d in ds {
                    self.env.add(defn(d));
                }
            }
            Decl::Quot { .. } => unreachable!("quotients are admitted by add_quot"),
        }
    }

    fn check_definition(&mut self, d: &DefnDecl) -> KResult<()> {
        let h = &d.header;
        if d.safety == Safety::Unsafe {
            self.with_decl(h.lparams.clone(), Safety::Unsafe, |k| {
                k.check_constant_val(h, false)
            })?;
            self.env.add_provisional(Const {
                name: h.name,
                lparams: h.lparams.clone(),
                ty: h.ty,
                kind: Kind::Axiom { is_unsafe: true },
            });
            let r = self.with_decl(h.lparams.clone(), Safety::Unsafe, |k| {
                k.check_value(h, d.value)
            });
            self.env.remove(h.name);
            r
        } else {
            self.with_decl(h.lparams.clone(), Safety::Safe, |k| {
                let allow = k.check_primitive_def(d)?;
                k.check_constant_val(h, allow)?;
                k.check_value(h, d.value)
            })
        }
    }

    fn check_mutual(&mut self, ds: &[DefnDecl]) -> KResult<()> {
        let Some(first) = ds.first() else {
            return Err(KernelFailure::reject("empty mutual definition"));
        };
        if first.safety == Safety::Safe {
            return Err(KernelFailure::reject(
                "invalid mutual definition, declaration is not tagged as unsafe/partial",
            ));
        }
        let safety = first.safety;
        let lparams = first.header.lparams.clone();
        self.with_decl(lparams.clone(), safety, |k| {
            let mut seen = Vec::new();
            for d in ds {
                if d.safety != safety {
                    return Err(KernelFailure::reject(
                        "mutual declarations must have the same safety annotation",
                    ));
                }
                if d.header.lparams != lparams {
                    return Err(KernelFailure::reject(
                        "mutual declarations must have the same universe parameters",
                    ));
                }
                if seen.contains(&d.header.name) {
                    return Err(KernelFailure::reject(
                        "duplicate declaration name in mutual block",
                    ));
                }
                seen.push(d.header.name);
                k.check_constant_val(&d.header, false)?;
            }
            Ok(())
        })?;
        for d in ds {
            self.env.add_provisional(Const {
                name: d.header.name,
                lparams: d.header.lparams.clone(),
                ty: d.header.ty,
                kind: Kind::Axiom {
                    is_unsafe: safety == Safety::Unsafe,
                },
            });
        }
        let r = self.with_decl(lparams, safety, |k| {
            for d in ds {
                k.check_value(&d.header, d.value)?;
            }
            Ok(())
        });
        for d in ds {
            self.env.remove(d.header.name);
        }
        r
    }

    // ---- quotients ----

    /// `eq` is the canonical equality: one inductive `Π (α : Sort u), α → α → Prop` with one
    /// constructor `Π (α : Sort u) (a : α), eq α a a`.
    fn check_eq_type(&mut self, eq: NameId) -> KResult<()> {
        let fail =
            |m: &str| KernelFailure::reject(format!("failed to initialize quot module, {m}"));
        let c = self
            .env
            .get(eq)
            .cloned()
            .ok_or_else(|| fail("environment does not have 'Eq' type"))?;
        let Kind::Induct(info) = &c.kind else {
            return Err(fail("'Eq' is not an inductive type"));
        };
        if c.lparams.len() != 1 {
            return Err(fail("unexpected number of universe params at 'Eq' type"));
        }
        if info.ctors.len() != 1 {
            return Err(fail("unexpected number of constructors for 'Eq' type"));
        }
        let u = self.s.levels.param(c.lparams[0]);
        let sort_u = self.s.sort(u);
        let alpha = self.push_local(sort_u, None);
        let prop = self.s.prop();
        let inner = self.s.pi(alpha, prop);
        let arrow = self.s.pi(alpha, inner);
        let expected = self.mk_pi(&[alpha], arrow);
        if c.ty != expected {
            return Err(fail("'Eq' has an unexpected type"));
        }
        let refl = self.get_const(info.ctors[0])?;
        if refl.lparams.len() != 1 {
            return Err(fail(
                "unexpected number of universe params at 'Eq' type constructor",
            ));
        }
        let u = self.s.levels.param(refl.lparams[0]);
        let sort_u = self.s.sort(u);
        let alpha = self.push_local(sort_u, None);
        let a = self.push_local(alpha, None);
        let eq_c = self.s.cnst_levels(eq, &[u]);
        let body = self.s.mk_app_n(eq_c, &[alpha, a, a]);
        let expected = self.mk_pi(&[alpha, a], body);
        if refl.ty != expected {
            return Err(fail("unexpected type for 'Eq' type constructor"));
        }
        Ok(())
    }

    /// The canonical type of each quotient constant, over level parameters `ps`. Built over
    /// free variables, so a non-dependent arrow is a plain `Pi` whose body is closed.
    fn quot_canonical(
        &mut self,
        kind: QuotKind,
        ps: &[NameId],
        set: QuotNames,
        eq: NameId,
    ) -> KResult<ExprId> {
        let need = if kind == QuotKind::Lift { 2 } else { 1 };
        if ps.len() != need {
            return Err(KernelFailure::reject(
                "unexpected number of universe parameters for a quotient constant",
            ));
        }
        let u = self.s.levels.param(ps[0]);
        let sort_u = self.s.sort(u);
        let prop = self.s.prop();
        let alpha = self.push_local(sort_u, None);
        let a_prop = self.s.pi(alpha, prop);
        let rel_ty = self.s.pi(alpha, a_prop);
        let ul = self.s.levels.list(&[u]);
        let quot = self.s.cnst(set.ty, ul);
        Ok(match kind {
            QuotKind::Type => {
                let r = self.push_local(rel_ty, None);
                self.mk_pi(&[alpha, r], sort_u)
            }
            QuotKind::Ctor => {
                let r = self.push_local(rel_ty, None);
                let a = self.push_local(alpha, None);
                let qr = self.s.mk_app_n(quot, &[alpha, r]);
                self.mk_pi(&[alpha, r, a], qr)
            }
            QuotKind::Lift => {
                let v = self.s.levels.param(ps[1]);
                let sort_v = self.s.sort(v);
                let r = self.push_local(rel_ty, None);
                let beta = self.push_local(sort_v, None);
                let f_ty = self.s.pi(alpha, beta);
                let f = self.push_local(f_ty, None);
                let a = self.push_local(alpha, None);
                let b = self.push_local(alpha, None);
                let rab = self.s.mk_app_n(r, &[a, b]);
                let vl = self.s.levels.list(&[v]);
                let eqv = self.s.cnst(eq, vl);
                let fa = self.s.app(f, a);
                let fb = self.s.app(f, b);
                let feq = self.s.mk_app_n(eqv, &[beta, fa, fb]);
                let rab_to = self.s.pi(rab, feq);
                let sanity = self.mk_pi(&[a, b], rab_to);
                let qr = self.s.mk_app_n(quot, &[alpha, r]);
                let qr_to_beta = self.s.pi(qr, beta);
                let body = self.s.pi(sanity, qr_to_beta);
                self.mk_pi(&[alpha, r, beta, f], body)
            }
            QuotKind::Ind => {
                let r = self.push_local(rel_ty, None);
                let qr = self.s.mk_app_n(quot, &[alpha, r]);
                let qr_prop = self.s.pi(qr, prop);
                let beta = self.push_local(qr_prop, None);
                let a = self.push_local(alpha, None);
                let mk = self.s.cnst(set.mk, ul);
                let mk_a = self.s.mk_app_n(mk, &[alpha, r, a]);
                let b_mk = self.s.app(beta, mk_a);
                let all_quot = self.mk_pi(&[a], b_mk);
                let q = self.push_local(qr, None);
                let bq = self.s.app(beta, q);
                let concl = self.mk_pi(&[q], bq);
                let body = self.s.pi(all_quot, concl);
                self.mk_pi(&[alpha, r, beta], body)
            }
        })
    }

    /// A quotient constant whose lift states its respect hypothesis with the equality `eq`: the
    /// theory's own quotient, or under LOGOS's theory the one a kernel installs.
    pub(crate) fn add_quotient(&mut self, h: Header, kind: QuotKind, eq: NameId) -> KResult<()> {
        let name = h.name;
        self.add_quot(h, kind, eq)
            .map_err(|e| e.in_declaration(self.s.names.display(name)))
    }

    fn add_quot(&mut self, h: Header, kind: QuotKind, eq: NameId) -> KResult<()> {
        let named = |set: &QuotNames| match kind {
            QuotKind::Type => set.ty,
            QuotKind::Ctor => set.mk,
            QuotKind::Lift => set.lift,
            QuotKind::Ind => set.ind,
        };
        let set = [Some(self.wk.quot_names()), self.wk.installed_quot]
            .into_iter()
            .flatten()
            .find(|set| named(set) == h.name)
            .ok_or_else(|| {
                KernelFailure::reject("quotient declaration with a non-canonical name")
            })?;
        if kind != QuotKind::Type && !self.env.contains(set.ty) {
            return Err(KernelFailure::reject(format!(
                "quotient constant declared before '{}'",
                self.s.names.display(set.ty)
            )));
        }
        self.with_decl(h.lparams.clone(), Safety::Safe, |k| {
            k.check_name(h.name, false)?;
            k.check_duplicate_lparams(&h.lparams)?;
            k.check_eq_type(eq)?;
            let canonical = k.quot_canonical(kind, &h.lparams, set, eq)?;
            if canonical != h.ty {
                return Err(
                    KernelFailure::reject("quotient constant has a non-canonical type")
                        .with_detail(format!(
                            "expected {}\n  found {}",
                            k.show(canonical),
                            k.show(h.ty)
                        )),
                );
            }
            Ok(())
        })?;
        self.env.add(Const {
            name: h.name,
            lparams: h.lparams,
            ty: h.ty,
            kind: Kind::Quot(kind),
        });
        if [set.ty, set.mk, set.lift, set.ind]
            .iter()
            .all(|n| self.env.contains(*n))
            && !self.env.quotients.contains(&set)
        {
            self.env.quotients.push(set);
        }
        self.env.quot_init = !self.env.quotients.is_empty();
        Ok(())
    }

    pub fn const_levels_of(&mut self, ps: &[NameId]) -> LevelsId {
        let ls: Vec<LevelId> = ps.iter().map(|p| self.s.levels.param(*p)).collect();
        self.s.levels.list(&ls)
    }

    pub fn is_sort(&self, e: ExprId) -> bool {
        matches!(self.s.node(e), Node::Sort(_))
    }
}
