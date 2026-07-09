//! A formal-development parser: the body of a `## Theory` block — a sequence of `Axiom`
//! and `Theorem` declarations in formal notation — into the axioms and
//! [`LibraryTheorem`](crate::verify::LibraryTheorem)s the multi-theorem driver consumes.
//!
//! This is the surface for an axiomatic development like Tarski geometry: a shared axiom
//! base plus dependency-ordered theorems, each citing earlier ones, all discharged and
//! kernel-certified by [`prove_library_with_axioms`](crate::verify::prove_library_with_axioms).
//!
//! ```text
//! Axiom flip: for all a b, Cong(a, b, b, a).
//! Axiom inner_trans: for all a b c d e f,
//!     if Cong(a,b,c,d) and Cong(a,b,e,f) then Cong(c,d,e,f).
//! Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
//! Theorem symmetry cites reflexivity:
//!     prove for all a b c d, if Cong(a,b,c,d) then Cong(c,d,a,b).
//! Theorem null_segment: given Cong(P, Q, R, R); prove P = Q.
//! ```
//!
//! Grammar (statements terminated by `.`):
//! * `Axiom <name> : <formula>`
//! * `Theorem <name> [cites <n1>, <n2>, …] : [given <formula> ;]* prove <formula>`
//!
//! Clauses within a theorem are separated by `;`; cited lemma names by `,`. Formulas are
//! handed to [`parse_formula`](crate::formula::parse_formula).

use crate::formula::{parse_formula, FormulaError};
use crate::verify::{prove_library_with_axioms, LibraryResult, LibraryTheorem};
use crate::ProofExpr;

/// A parsed formal development: a shared axiom base and the theorems built on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Development {
    /// Named axioms (the name is for diagnostics/citation; the prover uses the formula).
    pub axioms: Vec<(String, ProofExpr)>,
    /// Theorems, in source order, each with its premises, goal, and cited lemma names.
    pub theorems: Vec<LibraryTheorem>,
    /// Names of `[simp]`-tagged declarations (axioms and theorems), in source
    /// order — the development's default rewrite-rule set.
    pub simp_tagged: Vec<String>,
}

impl Development {
    /// The axiom formulas, in declaration order — the shared base for the driver.
    pub fn axiom_exprs(&self) -> Vec<ProofExpr> {
        self.axioms.iter().map(|(_, e)| e.clone()).collect()
    }

    /// The names tagged `[simp]`, in source order.
    pub fn simp_lemmas(&self) -> &[String] {
        &self.simp_tagged
    }
}

/// Parse a `## Theory` body into a [`Development`].
pub fn parse_development(body: &str) -> Result<Development, FormulaError> {
    let mut axioms = Vec::new();
    let mut theorems = Vec::new();

    let mut simp_tagged = Vec::new();

    for stmt in split_on_top_level(body, '.') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        let words: Vec<&str> = stmt.split_whitespace().collect();
        match words.first().map(|w| w.to_lowercase()).as_deref() {
            Some("axiom") => {
                let (decl, simp) = parse_axiom_decl(stmt)?;
                if simp {
                    simp_tagged.push(decl.0.clone());
                }
                axioms.push(decl);
            }
            Some("theorem") => {
                let (thm, simp) = parse_theorem_decl(stmt)?;
                if simp {
                    simp_tagged.push(thm.name.clone());
                }
                theorems.push(thm);
            }
            other => {
                return Err(FormulaError::new(format!(
                    "expected a declaration starting with 'Axiom' or 'Theorem', found {other:?}"
                )))
            }
        }
    }

    Ok(Development { axioms, theorems, simp_tagged })
}

/// Parse a `## Theory` body and discharge every theorem against the development's axioms
/// (and cited lemmas), in citation order — each kernel-certified. Results are in source
/// order, paired with the theorem name.
pub fn prove_development(body: &str) -> Result<Vec<(String, LibraryResult)>, FormulaError> {
    let dev = parse_development(body)?;
    let axioms = dev.axiom_exprs();
    let results = prove_library_with_axioms(&axioms, &dev.theorems);
    Ok(dev
        .theorems
        .iter()
        .map(|t| t.name.clone())
        .zip(results)
        .collect())
}

// ---------------------------------------------------------------------------
// Declaration parsing
// ---------------------------------------------------------------------------

/// The `[simp]` attribute token, written between a declaration's name and the
/// rest of its header. Returns the header words with the attribute removed and
/// whether it was present.
fn strip_simp_attr<'a>(words: &[&'a str]) -> (Vec<&'a str>, bool) {
    let mut simp = false;
    let kept = words
        .iter()
        .filter(|w| {
            if w.eq_ignore_ascii_case("[simp]") {
                simp = true;
                false
            } else {
                true
            }
        })
        .copied()
        .collect();
    (kept, simp)
}

/// `Axiom <name> [simp] : <formula>`
fn parse_axiom_decl(stmt: &str) -> Result<((String, ProofExpr), bool), FormulaError> {
    let (header, formula_src) = split_once_top_level(stmt, ':').ok_or_else(|| {
        FormulaError::new("an 'Axiom' declaration needs a ':' before its formula")
    })?;
    let hwords: Vec<&str> = header.split_whitespace().collect();
    let (hwords, simp) = strip_simp_attr(&hwords);
    let name = hwords
        .get(1)
        .ok_or_else(|| FormulaError::new("'Axiom' declaration is missing a name"))?
        .to_string();
    let formula = parse_formula(&formula_src)?;
    Ok(((name, formula), simp))
}

/// `Theorem <name> [simp] [cites <n1>, <n2>, …] : [given <formula> ;]* prove <formula>`
fn parse_theorem_decl(stmt: &str) -> Result<(LibraryTheorem, bool), FormulaError> {
    let (header, body) = split_once_top_level(stmt, ':').ok_or_else(|| {
        FormulaError::new("a 'Theorem' declaration needs a ':' before its clauses")
    })?;

    // Header: `Theorem <name> [simp] [cites/using/from <n1>, <n2>, …]`.
    let hwords: Vec<&str> = header.split_whitespace().collect();
    let (hwords, simp) = strip_simp_attr(&hwords);
    let name = hwords
        .get(1)
        .ok_or_else(|| FormulaError::new("'Theorem' declaration is missing a name"))?
        .to_string();
    let mut cites = Vec::new();
    if let Some(kw) = hwords.get(2) {
        if matches!(kw.to_lowercase().as_str(), "cites" | "using" | "from") {
            for w in &hwords[3..] {
                for part in w.split(',') {
                    let part = part.trim();
                    if !part.is_empty() {
                        cites.push(part.to_string());
                    }
                }
            }
        }
    }

    // Clauses: `given <formula>` (zero or more) then `prove <formula>`. A new clause
    // begins at each `given`/`assume`/`prove`/`show` KEYWORD — no separator punctuation is
    // required, so this is robust to the surface NL lexer dropping a `;` (and any stray
    // `;`/`.` left in a formula is absorbed by the formula tokenizer). Formal formulas never
    // contain these keywords, so keyword-delimited splitting is unambiguous.
    let is_clause_kw = |w: &str| matches!(w, "given" | "assume" | "prove" | "show");
    let words: Vec<&str> = body.split_whitespace().collect();
    let mut premises = Vec::new();
    let mut goal: Option<ProofExpr> = None;
    let mut i = 0;
    while i < words.len() {
        let kw = words[i].to_lowercase();
        if !is_clause_kw(&kw) {
            // A stray token before the first clause keyword (e.g. a lone `;`): skip it.
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < words.len() && !is_clause_kw(&words[i].to_lowercase()) {
            i += 1;
        }
        let formula_src = words[start..i].join(" ");
        match kw.as_str() {
            "given" | "assume" => premises.push(parse_formula(&formula_src)?),
            _ /* prove | show */ => {
                if goal.is_some() {
                    return Err(FormulaError::new(format!(
                        "theorem '{name}' has more than one 'prove' clause"
                    )));
                }
                goal = Some(parse_formula(&formula_src)?);
            }
        }
    }

    let goal = goal.ok_or_else(|| {
        FormulaError::new(format!("theorem '{name}' has no 'prove' clause"))
    })?;
    Ok((LibraryTheorem { name, premises, goal, cites }, simp))
}

// ---------------------------------------------------------------------------
// Delimiter splitting (paren-depth aware, so a delimiter inside `(…)` is ignored)
// ---------------------------------------------------------------------------

/// Split `s` on every top-level (paren-depth 0) occurrence of `delim`.
fn split_on_top_level(s: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            c if c == delim && depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Split `s` at the FIRST top-level occurrence of `delim` into `(before, after)`.
fn split_once_top_level(s: &str, delim: char) -> Option<(String, String)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == delim && depth == 0 => {
                return Some((s[..i].to_string(), s[i + c.len_utf8()..].to_string()));
            }
            _ => {}
        }
    }
    None
}
