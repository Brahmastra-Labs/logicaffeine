//! Inductive families: Lean's `add_inductive`, including nested-inductive elimination and
//! recursor generation.
//!
//! The kernel is handed the family's types and constructor types and derives everything else
//! itself: parameter/index split, universe constraints, strict positivity, recursive and
//! reflexive flags, large elimination, the K flag, and every recursor with its reduction rules.
//! An export's claimed inductive, constructor and recursor declarations are admitted only if they
//! are identical to what was derived, so a recursor with a forged rule, a missing induction
//! hypothesis or an unjustified K flag cannot enter the environment.

use super::env::{Const, CtorInfo, InductiveInfo, Kind, RecInfo, RecRule, Safety};
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Node};
use super::level::{LevelId, LevelNode, LevelsId};
use super::name::NameId;
use super::tc::Kernel;
use rustc_hash::FxHashMap as FxMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct IndType {
    pub name: NameId,
    pub ty: ExprId,
    pub ctors: Vec<(NameId, ExprId)>,
}

#[derive(Clone, Debug)]
pub struct InductiveDecl {
    pub lparams: Rc<[NameId]>,
    pub nparams: u32,
    pub types: Vec<IndType>,
    pub is_unsafe: bool,
}

struct Stats {
    levels: LevelsId,
    result_level: LevelId,
    nindices: Vec<u32>,
    ind_consts: Vec<ExprId>,
    ind_names: Vec<NameId>,
    params: Vec<ExprId>,
    is_not_zero: bool,
}

#[derive(Clone)]
struct RecTmp {
    motive: ExprId,
    minors: Vec<ExprId>,
    indices: Vec<ExprId>,
    major: ExprId,
}

struct Nested {
    params: Vec<ExprId>,
    aux2nested: Vec<(NameId, ExprId)>,
    types: Vec<IndType>,
}

struct NestState {
    new_types: Vec<IndType>,
    nested_aux: Vec<(ExprId, NameId)>,
    next_idx: u64,
    lvls: LevelsId,
}

fn reject(m: impl Into<String>) -> KernelFailure {
    KernelFailure::reject(m)
}

impl Kernel {
    /// Admits an inductive family whose declarations an export claims are `claimed`.
    pub fn add_inductive(&mut self, decl: InductiveDecl, claimed: Vec<Const>) -> KResult<()> {
        let first = decl.types.first().map(|t| t.name);
        let safety = if decl.is_unsafe {
            Safety::Unsafe
        } else {
            Safety::Safe
        };
        let r = self.with_decl(decl.lparams.clone(), safety, |k| {
            let generated = k.generate_inductive(&decl)?;
            k.compare_claimed(&generated, &claimed)
        });
        r.map_err(|e| {
            e.in_declaration(first.map(|n| self.s.names.display(n)).unwrap_or_default())
        })?;
        for c in claimed {
            self.env.add(c);
        }
        Ok(())
    }

    fn compare_claimed(&mut self, generated: &[Const], claimed: &[Const]) -> KResult<()> {
        let mut by_name: FxMap<NameId, &Const> = FxMap::default();
        for c in claimed {
            if by_name.insert(c.name, c).is_some() {
                return Err(reject(format!(
                    "'{}' is declared twice in one inductive block",
                    self.s.names.display(c.name)
                )));
            }
        }
        for g in generated {
            let Some(c) = by_name.get(&g.name) else {
                return Err(reject(format!(
                    "the inductive block does not declare '{}', which the kernel derives",
                    self.s.names.display(g.name)
                )));
            };
            if *c != g {
                return Err(reject(format!(
                    "the exported '{}' differs from the declaration the kernel derives",
                    self.s.names.display(g.name)
                ))
                .with_detail(self.explain_difference(g, c)));
            }
        }
        if claimed.len() != generated.len() {
            let extra: Vec<String> = claimed
                .iter()
                .filter(|c| !generated.iter().any(|g| g.name == c.name))
                .map(|c| self.s.names.display(c.name))
                .collect();
            return Err(reject(format!(
                "the inductive block declares constants the kernel does not derive: {}",
                extra.join(", ")
            )));
        }
        Ok(())
    }

    fn explain_difference(&self, g: &Const, c: &Const) -> String {
        if g.lparams != c.lparams {
            return "universe parameters differ".into();
        }
        if g.ty != c.ty {
            return format!(
                "derived type: {}\n  exported type: {}",
                self.show(g.ty),
                self.show(c.ty)
            );
        }
        match (&g.kind, &c.kind) {
            (Kind::Rec(a), Kind::Rec(b)) => {
                if a.k != b.k {
                    return format!("K flag: derived {}, exported {}", a.k, b.k);
                }
                if a.rules.len() != b.rules.len() {
                    return format!(
                        "derived {} rules, exported {}",
                        a.rules.len(),
                        b.rules.len()
                    );
                }
                for (x, y) in a.rules.iter().zip(b.rules.iter()) {
                    if x != y {
                        return format!(
                            "rule for '{}': derived {} with {} fields\n  exported {} with {} fields",
                            self.s.names.display(x.ctor),
                            self.show(x.rhs),
                            x.nfields,
                            self.show(y.rhs),
                            y.nfields
                        );
                    }
                }
                format!("derived {a:?}\n  exported {b:?}")
            }
            (a, b) => format!("derived {a:?}\n  exported {b:?}"),
        }
    }

    pub(crate) fn rec_name(&mut self, n: NameId) -> NameId {
        self.s.names.str(n, "rec")
    }

    /// Lean's `Expr.consumeTypeAnnotations`.
    fn consume_type_annotations(&self, mut e: ExprId) -> ExprId {
        loop {
            let w = &self.wk;
            if self.is_app_of_arity(e, w.opt_param, 2) || self.is_app_of_arity(e, w.auto_param, 2) {
                let Node::App(f, _) = self.s.node(e) else {
                    unreachable!()
                };
                let Node::App(_, t) = self.s.node(f) else {
                    unreachable!()
                };
                e = t;
            } else if self.is_app_of_arity(e, w.out_param, 1)
                || self.is_app_of_arity(e, w.semi_out_param, 1)
            {
                let Node::App(_, t) = self.s.node(e) else {
                    unreachable!()
                };
                e = t;
            } else {
                return e;
            }
        }
    }

    fn local_type(&self, fv: ExprId) -> ExprId {
        self.lctx[self.fvar_index(fv) as usize].ty
    }

    fn has_ind_occ(&self, names: &[NameId], e: ExprId) -> bool {
        self.s.mentions_const(e, names)
    }

    fn generate_inductive(&mut self, decl: &InductiveDecl) -> KResult<Vec<Const>> {
        if decl.types.is_empty() {
            return Err(reject("empty inductive declaration"));
        }
        for t in &decl.types {
            if self.s.has_fvar(t.ty) || self.s.has_loose_bvars(t.ty) {
                return Err(reject("inductive type contains free variables"));
            }
            for (_, cty) in &t.ctors {
                if self.s.has_fvar(*cty) || self.s.has_loose_bvars(*cty) {
                    return Err(reject("constructor type contains free variables"));
                }
                self.check_no_nested_aux(*cty)?;
            }
        }
        let allow_primitive = self.check_primitive_inductive(decl)?;
        let nested = self.elim_nested(decl)?;
        let num_nested = nested.aux2nested.len() as u32;
        let mut provisional = Vec::new();
        let r = self.add_inductive_core(
            decl,
            &nested.types,
            num_nested,
            allow_primitive,
            &mut provisional,
        );
        let r = match r {
            Ok(generated) if num_nested == 0 => Ok(generated),
            Ok(generated) => self.restore_nested(decl, &nested, generated),
            Err(e) => Err(e),
        };
        for n in provisional {
            self.env.remove(n);
        }
        r
    }

    fn check_no_nested_aux(&self, e: ExprId) -> KResult<()> {
        let p = self.wk.nested_prefix;
        let bad = self.s.any_node(e, &mut |s, x| match s.node(x) {
            Node::Const(c, _) | Node::Proj(c, _, _) => s.names.is_prefix_of(p, c),
            _ => false,
        });
        if bad {
            return Err(reject("declaration uses the reserved prefix '_nested'"));
        }
        Ok(())
    }

    /// Lean's `checkPrimitiveInductive`: may this family take the names `Nat`/`Bool`?
    fn check_primitive_inductive(&mut self, decl: &InductiveDecl) -> KResult<bool> {
        if decl.is_unsafe || !decl.lparams.is_empty() || decl.nparams != 0 || decl.types.len() != 1
        {
            return Ok(false);
        }
        let t = &decl.types[0];
        let one = self.s.levels.of_nat(1);
        if t.ty != self.s.sort(one) {
            return Ok(false);
        }
        let fail = |k: &Kernel| {
            reject(format!(
                "invalid form for primitive inductive '{}'",
                k.s.names.display(t.name)
            ))
        };
        let w = (
            self.wk.bool_,
            self.wk.bool_false,
            self.wk.bool_true,
            self.wk.nat,
            self.wk.nat_zero,
            self.wk.nat_succ,
        );
        if t.name == w.0 {
            let b = self.s.cnst(w.0, LevelsId::EMPTY);
            if t.ctors != [(w.1, b), (w.2, b)] {
                return Err(fail(self));
            }
        } else if t.name == w.3 {
            let n = self.s.cnst(w.3, LevelsId::EMPTY);
            let nn = self.s.pi(n, n);
            if t.ctors != [(w.4, n), (w.5, nn)] {
                return Err(fail(self));
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    // ---- AddInductive ----

    fn add_inductive_core(
        &mut self,
        decl: &InductiveDecl,
        types: &[IndType],
        num_nested: u32,
        allow_primitive: bool,
        provisional: &mut Vec<NameId>,
    ) -> KResult<Vec<Const>> {
        let nparams = decl.nparams as usize;
        let is_unsafe = decl.is_unsafe;
        self.check_duplicate_lparams(&decl.lparams)?;
        let levels = self.const_levels_of(&decl.lparams);
        let mut stats = Stats {
            levels,
            result_level: LevelId::ZERO,
            nindices: Vec::new(),
            ind_consts: Vec::new(),
            ind_names: types.iter().map(|t| t.name).collect(),
            params: Vec::new(),
            is_not_zero: false,
        };
        // checkInductiveTypes
        for (d, it) in types.iter().enumerate() {
            self.check(it.ty)?;
            let mut t = self.whnf(it.ty)?;
            let mut i = 0usize;
            let mut nindices = 0u32;
            while let Node::Pi(dom, body) = self.s.node(t) {
                if i < nparams {
                    if d == 0 {
                        let ty = self.consume_type_annotations(dom);
                        let p = self.push_local(ty, None);
                        stats.params.push(p);
                        let b = self.s.instantiate1(body, p);
                        t = self.whnf(b)?;
                    } else {
                        let p = stats.params[i];
                        let pt = self.local_type(p);
                        if !self.is_def_eq(dom, pt)? {
                            return Err(reject("parameters of all inductive datatypes must match"));
                        }
                        let b = self.s.instantiate1(body, p);
                        t = self.whnf(b)?;
                    }
                    i += 1;
                } else {
                    let ty = self.consume_type_annotations(dom);
                    let a = self.push_local(ty, None);
                    let b = self.s.instantiate1(body, a);
                    t = self.whnf(b)?;
                    nindices += 1;
                }
            }
            if i != nparams {
                return Err(reject(
                    "number of parameters mismatch in inductive datatype declaration",
                ));
            }
            let sort = self.ensure_sort(t)?;
            let rl = self.sort_level(sort);
            if d == 0 {
                stats.result_level = rl;
                stats.is_not_zero = self.s.levels.is_never_zero(rl);
            } else if !self.s.levels.equiv(rl, stats.result_level) {
                return Err(reject(
                    "mutually inductive types must live in the same universe",
                ));
            }
            stats.nindices.push(nindices);
            let c = self.s.cnst(it.name, stats.levels);
            stats.ind_consts.push(c);
        }
        let all: Rc<[NameId]> = types.iter().map(|t| t.name).collect::<Vec<_>>().into();
        let is_rec = self.syntactic_occurrence(types, &stats.ind_names, false);
        let is_reflexive = self.syntactic_occurrence(types, &stats.ind_names, true);
        let mut out = Vec::new();
        // declareInductiveTypes
        for (d, it) in types.iter().enumerate() {
            self.check_name(it.name, allow_primitive)?;
            let c = Const {
                name: it.name,
                lparams: decl.lparams.clone(),
                ty: it.ty,
                kind: Kind::Induct(InductiveInfo {
                    num_params: nparams as u32,
                    num_indices: stats.nindices[d],
                    all: all.clone(),
                    ctors: it.ctors.iter().map(|c| c.0).collect::<Vec<_>>().into(),
                    num_nested,
                    is_rec,
                    is_reflexive,
                    is_unsafe,
                    rec: self.rec_name(it.name),
                }),
            };
            self.env.add_provisional(c.clone());
            provisional.push(it.name);
            out.push(c);
        }
        // checkConstructors
        for (d, it) in types.iter().enumerate() {
            let mut found: Vec<NameId> = Vec::new();
            for (cn, cty) in &it.ctors {
                if found.contains(cn) {
                    return Err(reject(format!(
                        "duplicate constructor name '{}'",
                        self.s.names.display(*cn)
                    )));
                }
                found.push(*cn);
                self.check(*cty)?;
                let mut t = *cty;
                let mut i = 0usize;
                loop {
                    let Node::Pi(dom, body) = self.s.node(t) else {
                        if !self.is_valid_ind_app_idx(&stats, t, d) {
                            return Err(reject(format!(
                                "invalid return type for '{}'",
                                self.s.names.display(*cn)
                            )));
                        }
                        break;
                    };
                    if i < nparams {
                        let p = stats.params[i];
                        let pt = self.local_type(p);
                        if !self.is_def_eq(dom, pt)? {
                            return Err(reject(format!(
                                "arg #{} of '{}' does not match inductive datatype parameters",
                                i + 1,
                                self.s.names.display(*cn)
                            )));
                        }
                        t = self.s.instantiate1(body, p);
                    } else {
                        let dt = self.infer(dom, true)?;
                        let s = self.ensure_sort(dt)?;
                        let sl = self.sort_level(s);
                        let rl = stats.result_level;
                        if !(self.s.levels.is_always_zero(rl) || self.s.levels.leq(sl, rl)) {
                            return Err(reject(format!(
                                "universe level of type_of(arg #{}) of '{}' is too big for the corresponding inductive datatype",
                                i + 1,
                                self.s.names.display(*cn)
                            )));
                        }
                        if !is_unsafe {
                            self.check_positivity(&stats, dom, *cn, i)?;
                        }
                        let ty = self.consume_type_annotations(dom);
                        let a = self.push_local(ty, None);
                        t = self.s.instantiate1(body, a);
                    }
                    i += 1;
                }
            }
        }
        // declareConstructors
        for it in types {
            for (cidx, (cn, cty)) in it.ctors.iter().enumerate() {
                let mut arity = 0u32;
                let mut t = *cty;
                while let Node::Pi(_, b) = self.s.node(t) {
                    arity += 1;
                    t = b;
                }
                self.check_name(*cn, allow_primitive)?;
                let c = Const {
                    name: *cn,
                    lparams: decl.lparams.clone(),
                    ty: *cty,
                    kind: Kind::Ctor(CtorInfo {
                        induct: it.name,
                        cidx: cidx as u32,
                        num_params: nparams as u32,
                        num_fields: arity - nparams as u32,
                        is_unsafe,
                    }),
                };
                if arity < nparams as u32 {
                    return Err(reject(
                        "constructor has fewer arguments than the inductive has parameters",
                    ));
                }
                self.env.add_provisional(c.clone());
                provisional.push(*cn);
                out.push(c);
            }
        }
        // recursors
        let elim_level = self.get_elim_level(&stats, types, &decl.lparams)?;
        let k = self.is_k_target(&stats, types);
        let rec_infos = self.mk_rec_infos(&stats, types, elim_level)?;
        let motives: Vec<ExprId> = rec_infos.iter().map(|r| r.motive).collect();
        let minors: Vec<ExprId> = rec_infos
            .iter()
            .flat_map(|r| r.minors.iter().copied())
            .collect();
        let (rec_lparams, rec_levels) = match self.s.levels.node(elim_level) {
            LevelNode::Param(u) => {
                let mut ps = vec![u];
                ps.extend(decl.lparams.iter().copied());
                let mut ls = vec![elim_level];
                ls.extend(self.s.levels.get_list(stats.levels).to_vec());
                let lsid = self.s.levels.list(&ls);
                (Rc::from(ps), lsid)
            }
            _ => (decl.lparams.clone(), stats.levels),
        };
        let rec_names: Vec<NameId> = types.iter().map(|t| self.rec_name(t.name)).collect();
        let mut minor_idx = 0usize;
        for (d, it) in types.iter().enumerate() {
            let info = &rec_infos[d];
            let mut binders = stats.params.clone();
            binders.extend(&motives);
            binders.extend(&minors);
            binders.extend(&info.indices);
            binders.push(info.major);
            let mi = self.s.mk_app_n(info.motive, &info.indices);
            let concl = self.s.app(mi, info.major);
            let ty = self.mk_pi(&binders, concl);
            let rules = self.mk_rec_rules(
                &stats,
                types,
                &rec_names,
                rec_levels,
                d,
                &motives,
                &minors,
                &mut minor_idx,
            )?;
            let name = rec_names[d];
            self.check_name(name, allow_primitive)?;
            let major_induct = it.name;
            out.push(Const {
                name,
                lparams: rec_lparams.clone(),
                ty,
                kind: Kind::Rec(RecInfo {
                    all: all.clone(),
                    num_params: nparams as u32,
                    num_indices: stats.nindices[d],
                    num_motives: motives.len() as u32,
                    num_minors: minors.len() as u32,
                    rules: rules.into(),
                    k,
                    is_unsafe,
                    major_induct,
                }),
            });
        }
        Ok(out)
    }

    /// `isRec` (`reflexive = false`) and `isReflexive` (`reflexive = true`), read off the
    /// constructor types syntactically as Lean does.
    fn syntactic_occurrence(&self, types: &[IndType], names: &[NameId], reflexive: bool) -> bool {
        types.iter().any(|t| {
            t.ctors.iter().any(|(_, cty)| {
                let mut e = *cty;
                while let Node::Pi(dom, body) = self.s.node(e) {
                    let occ = self.has_ind_occ(names, dom);
                    if occ && (!reflexive || matches!(self.s.node(dom), Node::Pi(..))) {
                        return true;
                    }
                    e = body;
                }
                false
            })
        })
    }

    fn is_valid_ind_app_idx(&self, stats: &Stats, t: ExprId, i: usize) -> bool {
        let (head, args) = self.s.app_args(t);
        if head != stats.ind_consts[i]
            || args.len() != stats.params.len() + stats.nindices[i] as usize
        {
            return false;
        }
        if args.iter().zip(&stats.params).any(|(a, p)| a != p) {
            return false;
        }
        !args[stats.params.len()..]
            .iter()
            .any(|a| self.has_ind_occ(&stats.ind_names, *a))
    }

    fn is_valid_ind_app(&self, stats: &Stats, t: ExprId) -> Option<usize> {
        (0..stats.ind_consts.len()).find(|i| self.is_valid_ind_app_idx(stats, t, *i))
    }

    fn is_rec_arg(&mut self, stats: &Stats, mut t: ExprId) -> KResult<Option<usize>> {
        loop {
            t = self.whnf(t)?;
            let Node::Pi(dom, body) = self.s.node(t) else {
                return Ok(self.is_valid_ind_app(stats, t));
            };
            let ty = self.consume_type_annotations(dom);
            let a = self.push_local(ty, None);
            t = self.s.instantiate1(body, a);
        }
    }

    fn check_positivity(
        &mut self,
        stats: &Stats,
        mut t: ExprId,
        ctor: NameId,
        idx: usize,
    ) -> KResult<()> {
        loop {
            t = self.whnf(t)?;
            if !self.has_ind_occ(&stats.ind_names, t) {
                return Ok(());
            }
            if let Node::Pi(dom, body) = self.s.node(t) {
                if self.has_ind_occ(&stats.ind_names, dom) {
                    return Err(reject(format!(
                        "arg #{} of '{}' has a non positive occurrence of the datatypes being declared",
                        idx + 1,
                        self.s.names.display(ctor)
                    )));
                }
                let ty = self.consume_type_annotations(dom);
                let a = self.push_local(ty, None);
                t = self.s.instantiate1(body, a);
            } else if self.is_valid_ind_app(stats, t).is_none() {
                return Err(reject(format!(
                    "arg #{} of '{}' has a non valid occurrence of the datatypes being declared",
                    idx + 1,
                    self.s.names.display(ctor)
                )));
            } else {
                return Ok(());
            }
        }
    }

    fn is_large_eliminator(&mut self, stats: &Stats, types: &[IndType]) -> KResult<bool> {
        if stats.is_not_zero {
            return Ok(true);
        }
        if types.len() != 1 {
            return Ok(false);
        }
        match types[0].ctors.len() {
            0 => Ok(true),
            1 => {
                let mut t = types[0].ctors[0].1;
                let mut i = 0;
                let mut to_check = Vec::new();
                while let Node::Pi(dom, body) = self.s.node(t) {
                    let ty = self.consume_type_annotations(dom);
                    let a = self.push_local(ty, None);
                    if i >= stats.params.len() {
                        let dt = self.infer(dom, true)?;
                        let s = self.ensure_sort(dt)?;
                        let sl = self.sort_level(s);
                        if !self.s.levels.is_always_zero(sl) {
                            to_check.push(a);
                        }
                    }
                    t = self.s.instantiate1(body, a);
                    i += 1;
                }
                let (_, args) = self.s.app_args(t);
                Ok(to_check.iter().all(|a| args.contains(a)))
            }
            _ => Ok(false),
        }
    }

    fn get_elim_level(
        &mut self,
        stats: &Stats,
        types: &[IndType],
        lparams: &[NameId],
    ) -> KResult<LevelId> {
        if !self.is_large_eliminator(stats, types)? {
            return Ok(LevelId::ZERO);
        }
        let mut u = self.wk.u;
        let mut i = 1;
        while lparams.contains(&u) {
            u = self.s.names.append_index_after(self.wk.u, i);
            i += 1;
        }
        Ok(self.s.levels.param(u))
    }

    fn is_k_target(&mut self, stats: &Stats, types: &[IndType]) -> bool {
        if types.len() != 1
            || !self.s.levels.is_always_zero(stats.result_level)
            || types[0].ctors.len() != 1
        {
            return false;
        }
        let mut t = types[0].ctors[0].1;
        let mut i = 0;
        while let Node::Pi(_, b) = self.s.node(t) {
            if i >= stats.params.len() {
                return false;
            }
            i += 1;
            t = b;
        }
        true
    }

    fn mk_rec_infos(
        &mut self,
        stats: &Stats,
        types: &[IndType],
        elim_level: LevelId,
    ) -> KResult<Vec<RecTmp>> {
        let mut infos = Vec::new();
        for (d, it) in types.iter().enumerate() {
            let mut t = self.whnf(it.ty)?;
            let mut i = 0;
            let mut indices = Vec::new();
            while let Node::Pi(dom, body) = self.s.node(t) {
                if i < stats.params.len() {
                    let b = self.s.instantiate1(body, stats.params[i]);
                    t = self.whnf(b)?;
                    i += 1;
                } else {
                    let ty = self.consume_type_annotations(dom);
                    let a = self.push_local(ty, None);
                    indices.push(a);
                    let b = self.s.instantiate1(body, a);
                    t = self.whnf(b)?;
                }
            }
            let tp = self.s.mk_app_n(stats.ind_consts[d], &stats.params);
            let t_ty = self.s.mk_app_n(tp, &indices);
            let t_ty = self.consume_type_annotations(t_ty);
            let major = self.push_local(t_ty, None);
            let sort = self.s.sort(elim_level);
            let mut bs = indices.clone();
            bs.push(major);
            let motive_ty = self.mk_pi(&bs, sort);
            let motive_ty = self.consume_type_annotations(motive_ty);
            let motive = self.push_local(motive_ty, None);
            infos.push(RecTmp {
                motive,
                minors: Vec::new(),
                indices,
                major,
            });
        }
        for (d, it) in types.iter().enumerate() {
            for (cn, cty) in &it.ctors {
                let (t, bu, u) = self.loop_ctor_args(stats, *cty)?;
                let (it_idx, it_indices) = self.get_i_indices(stats, t)?;
                let c = self.s.cnst(*cn, stats.levels);
                let cp = self.s.mk_app_n(c, &stats.params);
                let intro = self.s.mk_app_n(cp, &bu);
                let mi = self.s.mk_app_n(infos[it_idx].motive, &it_indices);
                let motive_app = self.s.app(mi, intro);
                let mut v = Vec::new();
                for ui in &u {
                    let (ui_ty, xs) = self.loop_u_args(*ui)?;
                    let (j, j_indices) = self.get_i_indices(stats, ui_ty)?;
                    let mj = self.s.mk_app_n(infos[j].motive, &j_indices);
                    let uxs = self.s.mk_app_n(*ui, &xs);
                    let body = self.s.app(mj, uxs);
                    let vi_ty = self.mk_pi(&xs, body);
                    let vi_ty = self.consume_type_annotations(vi_ty);
                    v.push(self.push_local(vi_ty, None));
                }
                let mut bs = bu.clone();
                bs.extend(&v);
                let minor_ty = self.mk_pi(&bs, motive_app);
                let minor_ty = self.consume_type_annotations(minor_ty);
                let minor = self.push_local(minor_ty, None);
                infos[d].minors.push(minor);
            }
        }
        Ok(infos)
    }

    /// The constructor's result type, its field locals, and the fields that are recursive.
    fn loop_ctor_args(
        &mut self,
        stats: &Stats,
        mut t: ExprId,
    ) -> KResult<(ExprId, Vec<ExprId>, Vec<ExprId>)> {
        let mut i = 0;
        let mut bu = Vec::new();
        let mut u = Vec::new();
        while let Node::Pi(dom, body) = self.s.node(t) {
            if i < stats.params.len() {
                t = self.s.instantiate1(body, stats.params[i]);
            } else {
                let ty = self.consume_type_annotations(dom);
                let a = self.push_local(ty, None);
                bu.push(a);
                if self.is_rec_arg(stats, dom)?.is_some() {
                    u.push(a);
                }
                t = self.s.instantiate1(body, a);
            }
            i += 1;
        }
        Ok((t, bu, u))
    }

    fn loop_u_args(&mut self, ui: ExprId) -> KResult<(ExprId, Vec<ExprId>)> {
        let ut = self.infer(ui, true)?;
        let mut t = self.whnf(ut)?;
        let mut xs = Vec::new();
        while let Node::Pi(dom, body) = self.s.node(t) {
            let ty = self.consume_type_annotations(dom);
            let x = self.push_local(ty, None);
            xs.push(x);
            let b = self.s.instantiate1(body, x);
            t = self.whnf(b)?;
        }
        Ok((t, xs))
    }

    fn get_i_indices(&self, stats: &Stats, t: ExprId) -> KResult<(usize, Vec<ExprId>)> {
        let idx = self
            .is_valid_ind_app(stats, t)
            .ok_or_else(|| reject("ill-formed recursive occurrence"))?;
        let (_, args) = self.s.app_args(t);
        Ok((idx, args[stats.params.len()..].to_vec()))
    }

    #[allow(clippy::too_many_arguments)]
    fn mk_rec_rules(
        &mut self,
        stats: &Stats,
        types: &[IndType],
        rec_names: &[NameId],
        rec_levels: LevelsId,
        d: usize,
        motives: &[ExprId],
        minors: &[ExprId],
        minor_idx: &mut usize,
    ) -> KResult<Vec<RecRule>> {
        let mut rules = Vec::new();
        for (cn, cty) in &types[d].ctors {
            let (_, bu, u) = self.loop_ctor_args(stats, *cty)?;
            let mut v = Vec::new();
            for ui in &u {
                let (ui_ty, xs) = self.loop_u_args(*ui)?;
                let (j, j_indices) = self.get_i_indices(stats, ui_ty)?;
                let rc = self.s.cnst(rec_names[j], rec_levels);
                let mut val = self.s.mk_app_n(rc, &stats.params);
                val = self.s.mk_app_n(val, motives);
                val = self.s.mk_app_n(val, minors);
                val = self.s.mk_app_n(val, &j_indices);
                let uxs = self.s.mk_app_n(*ui, &xs);
                let body = self.s.app(val, uxs);
                v.push(self.mk_lambda(&xs, body));
            }
            let m = self.s.mk_app_n(minors[*minor_idx], &bu);
            let m = self.s.mk_app_n(m, &v);
            let mut bs = stats.params.clone();
            bs.extend(motives);
            bs.extend(minors);
            bs.extend(&bu);
            let rhs = self.mk_lambda(&bs, m);
            rules.push(RecRule {
                ctor: *cn,
                nfields: bu.len() as u32,
                rhs,
            });
            *minor_idx += 1;
        }
        Ok(rules)
    }

    // ---- ElimNestedInductive ----

    fn with_params(&mut self, mut e: ExprId, n: usize) -> KResult<(Vec<ExprId>, ExprId)> {
        let mut ps = Vec::new();
        for _ in 0..n {
            let Node::Pi(d, b) = self.s.node(e) else {
                return Err(reject(
                    "invalid inductive datatype declaration, incorrect number of parameters",
                ));
            };
            let p = self.push_local(d, None);
            ps.push(p);
            e = self.s.instantiate1(b, p);
        }
        Ok((ps, e))
    }

    fn replace_params(&mut self, params: &[ExprId], e: ExprId, as_: &[ExprId]) -> ExprId {
        let idx: Vec<u32> = as_.iter().map(|a| self.fvar_index(*a)).collect();
        let abs = self.s.abstract_fvars(e, &idx);
        self.s.instantiate_rev(abs, params)
    }

    fn instantiate_forall_params(
        &mut self,
        mut e: ExprId,
        hi: usize,
        args: &[ExprId],
    ) -> KResult<ExprId> {
        for _ in 0..hi {
            let Node::Pi(_, b) = self.s.node(e) else {
                return Err(reject(
                    "invalid nested inductive datatype, ill-formed declaration",
                ));
            };
            e = b;
        }
        Ok(self.s.instantiate_rev(e, &args[..hi]))
    }

    fn elim_nested(&mut self, decl: &InductiveDecl) -> KResult<Nested> {
        let nparams = decl.nparams as usize;
        let (params, _) = self.with_params(decl.types[0].ty, nparams)?;
        let lvls = self.const_levels_of(&decl.lparams);
        let mut st = NestState {
            new_types: decl.types.clone(),
            nested_aux: Vec::new(),
            next_idx: 1,
            lvls,
        };
        let mut i = 0;
        while i < st.new_types.len() {
            let ctors = st.new_types[i].ctors.clone();
            let mut new_ctors = Vec::with_capacity(ctors.len());
            for (cn, cty) in ctors {
                let (as_, body) = self.with_params(cty, nparams)?;
                let mut memo = FxMap::default();
                let replaced = self.replace_all_nested(&params, &as_, body, &mut st, &mut memo)?;
                new_ctors.push((cn, self.mk_pi(&as_, replaced)));
            }
            st.new_types[i].ctors = new_ctors;
            i += 1;
        }
        let aux2nested = st.nested_aux.iter().map(|(e, n)| (*n, *e)).collect();
        Ok(Nested {
            params,
            aux2nested,
            types: st.new_types,
        })
    }

    fn replace_all_nested(
        &mut self,
        params: &[ExprId],
        as_: &[ExprId],
        e: ExprId,
        st: &mut NestState,
        memo: &mut FxMap<ExprId, ExprId>,
    ) -> KResult<ExprId> {
        if let Some(r) = memo.get(&e) {
            return Ok(*r);
        }
        let r = if let Some(r) = self.replace_if_nested(params, as_, e, st)? {
            r
        } else {
            match self.s.node(e) {
                Node::App(f, a) => {
                    let f2 = self.replace_all_nested(params, as_, f, st, memo)?;
                    let a2 = self.replace_all_nested(params, as_, a, st, memo)?;
                    self.s.app(f2, a2)
                }
                Node::Lam(t, b) => {
                    let t2 = self.replace_all_nested(params, as_, t, st, memo)?;
                    let b2 = self.replace_all_nested(params, as_, b, st, memo)?;
                    self.s.lam(t2, b2)
                }
                Node::Pi(t, b) => {
                    let t2 = self.replace_all_nested(params, as_, t, st, memo)?;
                    let b2 = self.replace_all_nested(params, as_, b, st, memo)?;
                    self.s.pi(t2, b2)
                }
                Node::Let(t, v, b) => {
                    let t2 = self.replace_all_nested(params, as_, t, st, memo)?;
                    let v2 = self.replace_all_nested(params, as_, v, st, memo)?;
                    let b2 = self.replace_all_nested(params, as_, b, st, memo)?;
                    self.s.let_(t2, v2, b2)
                }
                Node::Proj(n, i, x) => {
                    let x2 = self.replace_all_nested(params, as_, x, st, memo)?;
                    self.s.proj(n, i, x2)
                }
                _ => e,
            }
        };
        memo.insert(e, r);
        Ok(r)
    }

    fn replace_if_nested(
        &mut self,
        params: &[ExprId],
        as_: &[ExprId],
        e: ExprId,
        st: &mut NestState,
    ) -> KResult<Option<ExprId>> {
        if !matches!(self.s.node(e), Node::App(..)) {
            return Ok(None);
        }
        let (head, args) = self.s.app_args(e);
        let Node::Const(fname, i_lvls) = self.s.node(head) else {
            return Ok(None);
        };
        let Some(i_val) = self.env.inductive(fname).cloned() else {
            return Ok(None);
        };
        let i_np = i_val.num_params as usize;
        if i_np > args.len() {
            return Ok(None);
        }
        let new_names: Vec<NameId> = st.new_types.iter().map(|t| t.name).collect();
        let mut is_nested = false;
        let mut loose = false;
        for a in &args[..i_np] {
            if self.s.has_loose_bvars(*a) {
                loose = true;
            }
            if self.s.mentions_const(*a, &new_names) {
                is_nested = true;
            }
        }
        if !is_nested {
            return Ok(None);
        }
        if loose {
            return Err(reject(format!(
                "invalid nested inductive datatype '{}', nested inductive datatypes parameters cannot contain local variables",
                self.s.names.display(fname)
            )));
        }
        let i_as = self.s.mk_app_n(head, &args[..i_np]);
        let i_params = self.replace_params(params, i_as, as_);
        if let Some((_, aux)) = st.nested_aux.iter().find(|(x, _)| *x == i_params) {
            let c = self.s.cnst(*aux, st.lvls);
            let ca = self.s.mk_app_n(c, as_);
            return Ok(Some(self.s.mk_app_n(ca, &args[i_np..])));
        }
        let mut result = None;
        let i_lvl_list = self.s.levels.get_list(i_lvls).to_vec();
        for &j_name in i_val.all.iter() {
            let j_const = self.get_const(j_name)?;
            let Some(j_info) = self.env.inductive(j_name).cloned() else {
                return Err(reject(
                    "nested inductive refers to a non-inductive in its block",
                ));
            };
            let j = self.s.cnst(j_name, i_lvls);
            let j_as = self.s.mk_app_n(j, &args[..i_np]);
            let base = self.s.names.append(self.wk.nested_prefix, j_name);
            let aux_j = self.mk_unique_name(base, st);
            let aux_ty = self
                .s
                .instantiate_lparams(j_const.ty, &j_const.lparams, &i_lvl_list);
            let aux_ty = self.instantiate_forall_params(aux_ty, i_np, &args)?;
            let aux_ty = self.mk_pi(as_, aux_ty);
            let j_as2 = self.replace_params(params, j_as, as_);
            st.nested_aux.push((j_as2, aux_j));
            if j_name == fname {
                let c = self.s.cnst(aux_j, st.lvls);
                let ca = self.s.mk_app_n(c, as_);
                result = Some(self.s.mk_app_n(ca, &args[i_np..]));
            }
            let mut ctors = Vec::new();
            for &jc in j_info.ctors.iter() {
                let jc_const = self.get_const(jc)?;
                let name = self.s.names.replace_prefix(jc, j_name, aux_j);
                let t = self
                    .s
                    .instantiate_lparams(jc_const.ty, &jc_const.lparams, &i_lvl_list);
                let t = self.instantiate_forall_params(t, i_np, &args)?;
                ctors.push((name, self.mk_pi(as_, t)));
            }
            st.new_types.push(IndType {
                name: aux_j,
                ty: aux_ty,
                ctors,
            });
        }
        result
            .map(Some)
            .ok_or_else(|| reject("nested inductive datatype is not in its own block"))
    }

    fn mk_unique_name(&mut self, base: NameId, st: &mut NestState) -> NameId {
        let mut i = st.next_idx;
        loop {
            let r = self.s.names.append_index_after(base, i);
            if !self.env.contains(r) {
                st.next_idx = i + 1;
                return r;
            }
            i += 1;
        }
    }

    fn restore_nested(
        &mut self,
        decl: &InductiveDecl,
        nested: &Nested,
        generated: Vec<Const>,
    ) -> KResult<Vec<Const>> {
        let nparams = decl.nparams as usize;
        let ntypes = decl.types.len();
        let main = decl.types[0].name;
        let Some(Kind::Induct(main_info)) = generated
            .iter()
            .find(|c| c.name == main)
            .map(|c| c.kind.clone())
        else {
            return Err(reject("nested elaboration lost the main inductive"));
        };
        let all_orig: Rc<[NameId]> = decl.types.iter().map(|t| t.name).collect::<Vec<_>>().into();
        let main_rec = self.rec_name(main);
        let mut rec_map: Vec<(NameId, NameId)> = Vec::new();
        for (i, &ind) in main_info.all.iter().skip(ntypes).enumerate() {
            let old = self.rec_name(ind);
            let new = self.s.names.append_index_after(main_rec, i as u64 + 1);
            rec_map.push((old, new));
        }
        let aux_of: FxMap<NameId, ExprId> = nested.aux2nested.iter().copied().collect();
        let param_idx: Vec<u32> = nested.params.iter().map(|p| self.fvar_index(*p)).collect();
        let gen_by: FxMap<NameId, Const> = generated.iter().map(|c| (c.name, c.clone())).collect();
        let mut out = Vec::new();
        let process_rec = |k: &mut Kernel, rec: NameId, out: &mut Vec<Const>| -> KResult<()> {
            let new_name = rec_map
                .iter()
                .find(|(o, _)| *o == rec)
                .map_or(rec, |(_, n)| *n);
            let g = gen_by
                .get(&rec)
                .ok_or_else(|| reject("missing generated recursor"))?
                .clone();
            let Kind::Rec(info) = &g.kind else {
                unreachable!()
            };
            let ty =
                k.restore_expr(g.ty, nparams, &nested.params, &param_idx, &aux_of, &rec_map)?;
            let mut rules = Vec::new();
            for r in info.rules.iter() {
                let rhs = k.restore_expr(
                    r.rhs,
                    nparams,
                    &nested.params,
                    &param_idx,
                    &aux_of,
                    &rec_map,
                )?;
                let ctor = if new_name == rec {
                    r.ctor
                } else {
                    k.restore_ctor_name(r.ctor, &aux_of, &param_idx, &nested.params)?
                };
                rules.push(RecRule {
                    ctor,
                    nfields: r.nfields,
                    rhs,
                });
            }
            let major_induct = k.major_induct_of(ty, info.major_idx())?;
            out.push(Const {
                name: new_name,
                lparams: g.lparams.clone(),
                ty,
                kind: Kind::Rec(RecInfo {
                    all: all_orig.clone(),
                    rules: rules.into(),
                    major_induct,
                    ..info.clone()
                }),
            });
            Ok(())
        };
        for t in &decl.types {
            let g = gen_by
                .get(&t.name)
                .ok_or_else(|| reject("missing generated inductive"))?
                .clone();
            let Kind::Induct(info) = &g.kind else {
                unreachable!()
            };
            out.push(Const {
                kind: Kind::Induct(InductiveInfo {
                    all: all_orig.clone(),
                    ..info.clone()
                }),
                ..g.clone()
            });
            for &cn in info.ctors.iter() {
                let gc = gen_by
                    .get(&cn)
                    .ok_or_else(|| reject("missing generated constructor"))?
                    .clone();
                let ty =
                    self.restore_expr(gc.ty, nparams, &nested.params, &param_idx, &aux_of, &[])?;
                out.push(Const { ty, ..gc });
            }
            let r = self.rec_name(t.name);
            process_rec(self, r, &mut out)?;
        }
        for (old, _) in rec_map.clone() {
            process_rec(self, old, &mut out)?;
        }
        for (_, e) in &nested.aux2nested {
            self.check(*e)?;
        }
        Ok(out)
    }

    fn major_induct_of(&self, mut ty: ExprId, major_idx: usize) -> KResult<NameId> {
        for _ in 0..major_idx {
            let Node::Pi(_, b) = self.s.node(ty) else {
                return Err(reject("ill-formed recursor type"));
            };
            ty = b;
        }
        let Node::Pi(dom, _) = self.s.node(ty) else {
            return Err(reject("ill-formed recursor type"));
        };
        self.s
            .const_head(dom)
            .map(|(n, _)| n)
            .ok_or_else(|| reject("ill-formed recursor major premise"))
    }

    fn nested_of_aux_ctor(
        &self,
        c: NameId,
        aux_of: &FxMap<NameId, ExprId>,
    ) -> Option<(ExprId, NameId)> {
        let ci = self.env.ctor(c)?;
        aux_of.get(&ci.induct).map(|e| (*e, ci.induct))
    }

    fn restore_ctor_name(
        &mut self,
        c: NameId,
        aux_of: &FxMap<NameId, ExprId>,
        _param_idx: &[u32],
        _params: &[ExprId],
    ) -> KResult<NameId> {
        let (nested, aux_name) = self
            .nested_of_aux_ctor(c, aux_of)
            .ok_or_else(|| reject("unknown auxiliary constructor"))?;
        let (i, _) = self
            .s
            .const_head(nested)
            .ok_or_else(|| reject("ill-formed nested occurrence"))?;
        Ok(self.s.names.replace_prefix(c, aux_name, i))
    }

    #[allow(clippy::too_many_arguments)]
    fn restore_expr(
        &mut self,
        mut e: ExprId,
        nparams: usize,
        params: &[ExprId],
        param_idx: &[u32],
        aux_of: &FxMap<NameId, ExprId>,
        rec_map: &[(NameId, NameId)],
    ) -> KResult<ExprId> {
        let pi = matches!(self.s.node(e), Node::Pi(..));
        let mut as_ = Vec::new();
        for _ in 0..nparams {
            match self.s.node(e) {
                Node::Pi(d, b) | Node::Lam(d, b) => {
                    let a = self.push_local(d, None);
                    as_.push(a);
                    e = self.s.instantiate1(b, a);
                }
                _ => return Err(reject("ill-formed nested inductive expression")),
            }
        }
        let _ = params;
        let mut memo = FxMap::default();
        let r = self.restore_go(e, nparams, param_idx, &as_, aux_of, rec_map, &mut memo)?;
        Ok(if pi {
            self.mk_pi(&as_, r)
        } else {
            self.mk_lambda(&as_, r)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn restore_go(
        &mut self,
        t: ExprId,
        nparams: usize,
        param_idx: &[u32],
        as_: &[ExprId],
        aux_of: &FxMap<NameId, ExprId>,
        rec_map: &[(NameId, NameId)],
        memo: &mut FxMap<ExprId, ExprId>,
    ) -> KResult<ExprId> {
        if let Some(r) = memo.get(&t) {
            return Ok(*r);
        }
        let replaced = 'rep: {
            if let Node::Const(c, ls) = self.s.node(t) {
                if let Some((_, n)) = rec_map.iter().find(|(o, _)| *o == c) {
                    break 'rep Some(self.s.cnst(*n, ls));
                }
            }
            let (head, args) = self.s.app_args(t);
            let Node::Const(c, _) = self.s.node(head) else {
                break 'rep None;
            };
            if let Some(nested) = aux_of.get(&c).copied() {
                if args.len() < nparams {
                    return Err(reject("ill-formed nested occurrence"));
                }
                let abs = self.s.abstract_fvars(nested, param_idx);
                let inst = self.s.instantiate_rev(abs, as_);
                break 'rep Some(self.s.mk_app_n(inst, &args[nparams..]));
            }
            if let Some((nested, aux_name)) = self.nested_of_aux_ctor(c, aux_of) {
                if args.len() < nparams {
                    return Err(reject("ill-formed nested constructor occurrence"));
                }
                let abs = self.s.abstract_fvars(nested, param_idx);
                let inst = self.s.instantiate_rev(abs, as_);
                let (ih, i_args) = self.s.app_args(inst);
                let Node::Const(i_c, i_ls) = self.s.node(ih) else {
                    return Err(reject("ill-formed nested occurrence"));
                };
                let new_c = self.s.names.replace_prefix(c, aux_name, i_c);
                let cc = self.s.cnst(new_c, i_ls);
                let ci = self.s.mk_app_n(cc, &i_args);
                break 'rep Some(self.s.mk_app_n(ci, &args[nparams..]));
            }
            None
        };
        let r = match replaced {
            Some(r) => r,
            None => match self.s.node(t) {
                Node::App(f, a) => {
                    let f2 = self.restore_go(f, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    let a2 = self.restore_go(a, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    self.s.app(f2, a2)
                }
                Node::Lam(d, b) => {
                    let d2 = self.restore_go(d, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    let b2 = self.restore_go(b, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    self.s.lam(d2, b2)
                }
                Node::Pi(d, b) => {
                    let d2 = self.restore_go(d, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    let b2 = self.restore_go(b, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    self.s.pi(d2, b2)
                }
                Node::Let(ty, v, b) => {
                    let t2 = self.restore_go(ty, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    let v2 = self.restore_go(v, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    let b2 = self.restore_go(b, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    self.s.let_(t2, v2, b2)
                }
                Node::Proj(n, i, x) => {
                    let x2 = self.restore_go(x, nparams, param_idx, as_, aux_of, rec_map, memo)?;
                    self.s.proj(n, i, x2)
                }
                _ => t,
            },
        };
        memo.insert(t, r);
        Ok(r)
    }
}
