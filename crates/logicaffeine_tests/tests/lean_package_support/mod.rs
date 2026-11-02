//! Shared by the W05 package, certificate and reference-translator test files: a writer for small
//! adversarial Lean exports, and the import → capture path the package assembler takes.
#![allow(dead_code)]

use logicaffeine_kernel::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentErrorCode,
};
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::Term;
use std::collections::BTreeMap;

/// A tiny export writer. Indices are allocated in order, so every reference is backwards, exactly
/// as lean4export writes them.
pub struct Export {
    pub lines: Vec<String>,
    names: u64,
    levels: u64,
    exprs: u64,
}

impl Export {
    pub fn new() -> Self {
        Export {
            lines: vec![r#"{"meta":{"lean":{"version":"4.14.0"},"format":{"version":"3.1.0"},"exporter":{"version":"3.1.0","name":"lean4export"}}}"#.into()],
            names: 0,
            levels: 0,
            exprs: 0,
        }
    }
    pub fn name(&mut self, component: &str, pre: u64) -> u64 {
        self.names += 1;
        self.lines.push(format!(
            r#"{{"str":{{"str":{},"pre":{pre}}},"in":{}}}"#,
            serde_json::to_string(component).unwrap(),
            self.names
        ));
        self.names
    }
    pub fn num_name(&mut self, i: u64, pre: u64) -> u64 {
        self.names += 1;
        self.lines.push(format!(
            r#"{{"num":{{"i":{i},"pre":{pre}}},"in":{}}}"#,
            self.names
        ));
        self.names
    }
    pub fn succ(&mut self, of: u64) -> u64 {
        self.levels += 1;
        self.lines
            .push(format!(r#"{{"succ":{of},"il":{}}}"#, self.levels));
        self.levels
    }
    pub fn param(&mut self, name: u64) -> u64 {
        self.levels += 1;
        self.lines
            .push(format!(r#"{{"param":{name},"il":{}}}"#, self.levels));
        self.levels
    }
    pub fn expr(&mut self, body: &str) -> u64 {
        let id = self.exprs;
        self.exprs += 1;
        self.lines.push(format!(r#"{{"ie":{id},{body}}}"#));
        id
    }
    pub fn sort(&mut self, level: u64) -> u64 {
        self.expr(&format!(r#""sort":{level}"#))
    }
    pub fn bvar(&mut self, i: u64) -> u64 {
        self.expr(&format!(r#""bvar":{i}"#))
    }
    pub fn constant(&mut self, name: u64, us: &[u64]) -> u64 {
        self.expr(&format!(r#""const":{{"us":{us:?},"name":{name}}}"#))
    }
    pub fn app(&mut self, f: u64, a: u64) -> u64 {
        self.expr(&format!(r#""app":{{"fn":{f},"arg":{a}}}"#))
    }
    pub fn forall(&mut self, binder: u64, ty: u64, body: u64) -> u64 {
        self.expr(&format!(
            r#""forallE":{{"type":{ty},"name":{binder},"body":{body},"binderInfo":"default"}}"#
        ))
    }
    pub fn lam(&mut self, binder: u64, ty: u64, body: u64) -> u64 {
        self.expr(&format!(
            r#""lam":{{"type":{ty},"name":{binder},"body":{body},"binderInfo":"default"}}"#
        ))
    }
    pub fn let_in(&mut self, binder: u64, ty: u64, value: u64, body: u64) -> u64 {
        self.expr(&format!(
            r#""letE":{{"type":{ty},"name":{binder},"value":{value},"body":{body},"nonDep":false}}"#
        ))
    }
    pub fn raw(&mut self, line: String) {
        self.lines.push(line);
    }
    pub fn axiom(&mut self, name: u64, ty: u64, levels: &[u64]) {
        self.lines.push(format!(
            r#"{{"axiom":{{"name":{name},"levelParams":{levels:?},"type":{ty},"isUnsafe":false}}}}"#
        ));
    }
    pub fn def(&mut self, name: u64, ty: u64, value: u64) {
        self.lines.push(format!(
            r#"{{"def":{{"value":{value},"type":{ty},"safety":"safe","name":{name},"levelParams":[],"hints":"abbrev","all":[{name}]}}}}"#
        ));
    }
    pub fn thm(&mut self, name: u64, ty: u64, value: u64) {
        self.lines.push(format!(
            r#"{{"thm":{{"value":{value},"type":{ty},"name":{name},"levelParams":[],"all":[{name}]}}}}"#
        ));
    }
    pub fn text(&self) -> String {
        self.lines.join("\n") + "\n"
    }
}

/// `axiom P : Prop`, `axiom p : P`, and a theorem `T : P := p` — the smallest honest environment.
pub fn small_theorem() -> Export {
    let mut e = Export::new();
    let p = e.name("P", 0);
    let proof = e.name("p", 0);
    let t = e.name("T", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    e.axiom(proof, pc, &[]);
    let pv = e.constant(proof, &[]);
    e.thm(t, pc, pv);
    e
}

/// Lean's `sorryAx.{u} : (α : Sort u) → Bool → α`, with the `Bool` inductive it needs, exactly as
/// lean4export writes them. Returns the name indices of `sorryAx` and `Bool.false`.
pub fn declare_sorry(e: &mut Export) -> (u64, u64) {
    let bool_name = e.name("Bool", 0);
    let false_name = e.name("false", bool_name);
    let true_name = e.name("true", bool_name);
    let rec_name = e.name("rec", bool_name);
    let type0_level = e.succ(0);
    let type0 = e.sort(type0_level);
    let bool_c = e.constant(bool_name, &[]);
    // Bool.rec.{u} : {motive : Bool → Sort u} → motive false → motive true → (t : Bool) → motive t
    let u = e.name("u", 0);
    let lu = e.param(u);
    let sort_u = e.sort(lu);
    let motive_n = e.name("motive", 0);
    let t_n = e.name("t", 0);
    let motive_ty = e.forall(t_n, bool_c, sort_u);
    let false_c = e.constant(false_name, &[]);
    let true_c = e.constant(true_name, &[]);
    let m1 = e.bvar(0);
    let m_false = e.app(m1, false_c);
    let m2 = e.bvar(1);
    let m_true = e.app(m2, true_c);
    let m3 = e.bvar(3);
    let t_var = e.bvar(0);
    let m_t = e.app(m3, t_var);
    let last = e.forall(t_n, bool_c, m_t);
    let hf = e.name("false", 0);
    let ht = e.name("true", 0);
    let with_true = e.forall(ht, m_true, last);
    let with_false = e.forall(hf, m_false, with_true);
    let rec_ty = e.expr(&format!(
        r#""forallE":{{"type":{motive_ty},"name":{motive_n},"body":{with_false},"binderInfo":"implicit"}}"#
    ));
    // Rules: rhs for false: fun motive f t => f ; for true: fun motive f t => t
    let r_f = e.bvar(1);
    let lam_t_f = e.lam(ht, m_true, r_f);
    let lam_f_f = e.lam(hf, m_false, lam_t_f);
    let rhs_false = e.lam(motive_n, motive_ty, lam_f_f);
    let r_t = e.bvar(0);
    let lam_t_t = e.lam(ht, m_true, r_t);
    let lam_f_t = e.lam(hf, m_false, lam_t_t);
    let rhs_true = e.lam(motive_n, motive_ty, lam_f_t);
    e.raw(format!(
        concat!(
            r#"{{"inductive":{{"types":[{{"type":{type0},"numParams":0,"numNested":0,"numIndices":0,"name":{bool_name},"levelParams":[],"isUnsafe":false,"isReflexive":false,"isRec":false,"ctors":[{false_name},{true_name}],"all":[{bool_name}]}}],"#,
            r#""ctors":[{{"type":{bool_c},"numParams":0,"numFields":0,"name":{false_name},"levelParams":[],"isUnsafe":false,"induct":{bool_name},"cidx":0}},"#,
            r#"{{"type":{bool_c},"numParams":0,"numFields":0,"name":{true_name},"levelParams":[],"isUnsafe":false,"induct":{bool_name},"cidx":1}}],"#,
            r#""recs":[{{"type":{rec_ty},"rules":[{{"rhs":{rhs_false},"nfields":0,"ctor":{false_name}}},{{"rhs":{rhs_true},"nfields":0,"ctor":{true_name}}}],"numParams":0,"numMotives":1,"numMinors":2,"numIndices":0,"name":{rec_name},"levelParams":[{u}],"k":false,"isUnsafe":false,"all":[{bool_name}]}}]}}}}"#
        ),
        type0 = type0, bool_name = bool_name, false_name = false_name, true_name = true_name, bool_c = bool_c,
        rec_ty = rec_ty, rhs_false = rhs_false, rhs_true = rhs_true, rec_name = rec_name, u = u
    ));
    // sorryAx.{u} : (α : Sort u) → Bool → α
    let sorry = e.name("sorryAx", 0);
    let alpha = e.name("α", 0);
    let synthetic = e.name("synthetic", 0);
    let a1 = e.bvar(1);
    let inner = e.forall(synthetic, bool_c, a1);
    let sorry_ty = e.forall(alpha, sort_u, inner);
    e.axiom(sorry, sorry_ty, &[u]);
    (sorry, false_name)
}

/// Import `export`, then capture `target`'s environment certificate, retaining exactly the opaque
/// proofs capture asks for — the same loop the package assembler runs.
pub fn capture(export: &str, target: &str) -> EnvironmentCertificate {
    capture_result(export, target).unwrap_or_else(|e| panic!("capture {target}: {e}"))
}

pub fn capture_result(export: &str, target: &str) -> Result<EnvironmentCertificate, String> {
    let (ctx, _) = adapter::import_audited(export).map_err(|v| format!("import: {v:?}"))?;
    let mut proofs: BTreeMap<String, Term> = BTreeMap::new();
    loop {
        let request = CaptureRequest {
            context: &ctx,
            base: EnvironmentBase::Kernel,
            conversion: ConversionDiscipline::Exact,
            target,
            opaque_proofs: &proofs,
        };
        match EnvironmentCertificate::capture(&request) {
            Ok(certificate) => return Ok(certificate),
            Err(e) if e.code == EnvironmentErrorCode::MissingEvidence => {
                let name = e
                    .message
                    .split_whitespace()
                    .find(|w| w.starts_with('/') || w.starts_with("Nat"))
                    .map(str::to_string)
                    .ok_or_else(|| format!("unparseable missing-evidence report: {e}"))?;
                if proofs.contains_key(&name) {
                    return Err(format!("capture asked twice for {name}"));
                }
                let (retained, _) = adapter::import_audited_retaining_source(export, &name)
                    .map_err(|v| format!("retain {name}: {v:?}"))?;
                let (_, _, body) = retained.retained_source().expect("retained");
                proofs.insert(name, body.clone());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}
