//! A tiny proof-script language over the [`crate::tactic`] combinators — proofs as
//! TEXT, the way a Lean/Coq proof reads. This is the self-contained seam toward a
//! full English vernacular: it has no dependency on the surface language, so it
//! compiles a script string straight into a composed [`Tactic`].
//!
//! Grammar (precedence low → high):
//!
//! ```text
//! script  := seq
//! seq     := chain (';' chain)*          // run in order
//! chain   := atom ('<;>' atom)*          // `t1 <;> t2`: t2 on every goal t1 spawns
//! atom    := '(' script ')'
//!          | 'first' '[' script ('|' script)* ']'   // first that applies (backtracks)
//!          | 'try' atom                              // run if it applies, else no-op
//!          | 'repeat' atom                           // apply until it stops applying
//!          | NAME ARG?                               // a primitive tactic
//! ```
//!
//! Primitive tactics: `intro h`, `exact h`, `cases h`, `rewrite h`, `exists W`,
//! `assumption`, `split`, `left`, `right`, `auto`, `induction`.

use std::collections::HashMap;

use crate::tactic::combinators as c;
use crate::tactic::{ProofState, Tactic};
use crate::verify::{citation_order, LibraryResult, LibraryTheorem};
use crate::{ProofExpr, ProofTerm};

/// A script parse error, with a human-readable reason.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError(pub String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tactic script error: {}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Semi,
    ThenAll,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Bar,
}

fn lex(src: &str) -> Result<Vec<Tok>, ScriptError> {
    let mut toks = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            c if c.is_whitespace() => i += 1,
            // Sentence punctuation reads as a step separator, so prose flows: a proof
            // can be written `Assume h. By cases on h, …` and parse the same as `;`.
            ';' | '.' | ',' => {
                toks.push(Tok::Semi);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(Tok::RBracket);
                i += 1;
            }
            '|' => {
                toks.push(Tok::Bar);
                i += 1;
            }
            '<' if src[i..].starts_with("<;>") => {
                toks.push(Tok::ThenAll);
                i += 3;
            }
            c if c.is_alphanumeric() || c == '_' => {
                let start = i;
                while i < bytes.len() {
                    let cc = bytes[i] as char;
                    if cc.is_alphanumeric() || cc == '_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                toks.push(Tok::Word(src[start..i].to_string()));
            }
            other => return Err(ScriptError(format!("unexpected character {other:?}"))),
        }
    }
    Ok(toks)
}

/// A registry of USER-DEFINED tactics (M) — the metaprogramming seam. A user names a
/// composite tactic (built from primitives and combinators) with [`TacticEnv::define`], and
/// may then reference it BY NAME in any later script, including from within other
/// user-defined tactics. This is "write your own tactic in the language", without Rust —
/// the definitions are stored as source and re-expanded (depth-bounded, so a recursive
/// definition is rejected rather than looping) wherever the name appears.
#[derive(Clone, Default)]
pub struct TacticEnv {
    defs: std::collections::HashMap<String, String>,
}

impl TacticEnv {
    pub fn new() -> Self {
        Self::default()
    }
    /// Define a named tactic from a script source. Names are case-insensitive.
    pub fn define(&mut self, name: &str, script: &str) {
        self.defs.insert(name.to_ascii_lowercase(), script.to_string());
    }
    fn get(&self, name: &str) -> Option<&String> {
        self.defs.get(&name.to_ascii_lowercase())
    }
}

/// Depth bound on user-tactic expansion — a recursive definition (`t := … t …`) is refused
/// at this depth rather than looping forever.
const MAX_TACTIC_DEPTH: usize = 128;

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    env: TacticEnv,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, t: &Tok) -> Result<(), ScriptError> {
        match self.bump() {
            Some(got) if &got == t => Ok(()),
            other => Err(ScriptError(format!("expected {t:?}, found {other:?}"))),
        }
    }

    /// True if the next token ends a step: `;`/`.`/`,` (lexed to `Semi`) or the word
    /// `then`. Lets prose read `Assume h, then by cases on h.`
    fn at_separator(&self) -> bool {
        matches!(self.peek(), Some(Tok::Semi))
            || matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("then"))
    }

    /// Skip purely connective filler words (`by`, `the`, `on`, …) so a tactic can be
    /// stated as `by cases on h` and mean `cases h`.
    fn skip_filler(&mut self) {
        while matches!(self.peek(), Some(Tok::Word(w)) if is_filler(w)) {
            self.bump();
        }
    }

    /// `script := chain (sep+ chain)*` — a *run* of separators (`.`/`,`/`;`/`then`,
    /// e.g. the `, then` in `Assume h, then …`) counts as one.
    fn parse_seq(&mut self) -> Result<Tactic, ScriptError> {
        let mut steps = vec![self.parse_chain()?];
        loop {
            let mut consumed = false;
            while self.at_separator() {
                self.bump();
                consumed = true;
            }
            if !consumed {
                break;
            }
            // Allow a trailing separator (a closing `.`) before `)`/`]`/`|`/EOF.
            if matches!(self.peek(), None | Some(Tok::RParen) | Some(Tok::RBracket) | Some(Tok::Bar)) {
                break;
            }
            steps.push(self.parse_chain()?);
        }
        Ok(if steps.len() == 1 {
            steps.into_iter().next().unwrap()
        } else {
            c::seq(steps)
        })
    }

    /// `chain := atom ('<;>' atom)*` — left-associative `then_all`.
    fn parse_chain(&mut self) -> Result<Tactic, ScriptError> {
        let mut acc = self.parse_atom()?;
        while matches!(self.peek(), Some(Tok::ThenAll)) {
            self.bump();
            let rhs = self.parse_atom()?;
            acc = c::then_all(acc, rhs);
        }
        Ok(acc)
    }

    fn parse_atom(&mut self) -> Result<Tactic, ScriptError> {
        self.skip_filler();
        match self.peek() {
            Some(Tok::LParen) => {
                self.bump();
                let inner = self.parse_seq()?;
                self.expect(&Tok::RParen)?;
                Ok(inner)
            }
            Some(Tok::Word(w)) if w == "first" => {
                self.bump();
                self.expect(&Tok::LBracket)?;
                let mut alts = vec![self.parse_seq()?];
                while matches!(self.peek(), Some(Tok::Bar)) {
                    self.bump();
                    alts.push(self.parse_seq()?);
                }
                self.expect(&Tok::RBracket)?;
                Ok(c::first(alts))
            }
            Some(Tok::Word(w)) if w == "try" => {
                self.bump();
                Ok(c::try_(self.parse_atom()?))
            }
            Some(Tok::Word(w)) if w == "repeat" => {
                self.bump();
                Ok(c::repeat(self.parse_atom()?))
            }
            Some(Tok::Word(_)) => self.parse_primitive(),
            other => Err(ScriptError(format!("expected a tactic, found {other:?}"))),
        }
    }

    fn parse_primitive(&mut self) -> Result<Tactic, ScriptError> {
        let raw = match self.bump() {
            Some(Tok::Word(w)) => w,
            other => return Err(ScriptError(format!("expected a tactic name, found {other:?}"))),
        };
        // A USER-DEFINED tactic (M) takes PRECEDENCE — an explicit `define` is authoritative,
        // so a user tactic may even shadow a built-in alias (`both`, `split`, …). Expand its
        // stored source in place, one level deeper (recursion is depth-bounded).
        if let Some(src) = self.env.get(&raw).cloned() {
            if self.depth >= MAX_TACTIC_DEPTH {
                return Err(ScriptError(format!(
                    "user tactic `{raw}` expands too deeply (recursive definition?)"
                )));
            }
            return parse_with_env(&src, &self.env, self.depth + 1);
        }
        let name = canonical(&raw)
            .ok_or_else(|| ScriptError(format!("unknown tactic `{raw}`")))?;
        // Filler may sit between the verb and its object: `cases on h`, `rewrite with eq`.
        let mut arg = || -> Result<String, ScriptError> {
            self.skip_filler();
            match self.peek() {
                Some(Tok::Word(w)) => {
                    let w = w.clone();
                    self.bump();
                    Ok(w)
                }
                _ => Err(ScriptError(format!("tactic `{name}` expects an argument"))),
            }
        };
        match name {
            "intro" => Ok(c::intro(&arg()?)),
            "exact" => Ok(c::exact(&arg()?)),
            "cases" => Ok(c::cases(&arg()?)),
            "rewrite" => Ok(c::rewrite(&arg()?)),
            "exists" => Ok(c::exists(ProofTerm::Constant(arg()?))),
            "assumption" => Ok(c::assumption()),
            "simp" => Ok(c::simp()),
            "decide" => Ok(c::decide()),
            "omega" => Ok(c::omega()),
            "crush" => Ok(c::crush()),
            "split" => Ok(c::split()),
            "left" => Ok(c::left()),
            "right" => Ok(c::right()),
            "auto" => Ok(c::auto()),
            "induction" => Ok(c::induction()),
            _ => Err(ScriptError(format!("unknown tactic `{name}`"))),
        }
    }
}

/// Connective filler words that carry no tactic meaning — skipped so a step can be
/// phrased as prose (`by cases on h`, `then by assumption`).
fn is_filler(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "by" | "the" | "on" | "that" | "now" | "with" | "we" | "it" | "a" | "an" | "of" | "to" | "and"
    )
}

/// Map an English-esque verb to its canonical tactic name. `Suppose`/`assume`/`let`
/// all mean `intro`; `automatically`/`trivially` mean `auto`; and so on — the
/// vocabulary that lets a proof read like prose. Case-insensitive.
fn canonical(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "intro" | "assume" | "suppose" | "let" | "introduce" | "given" | "fix" => Some("intro"),
        "exact" | "from" | "because" => Some("exact"),
        "cases" | "destruct" | "consider" | "casework" => Some("cases"),
        "rewrite" | "rw" | "substitute" | "subst" => Some("rewrite"),
        "exists" | "use" | "witness" | "choose" | "provide" => Some("exists"),
        "assumption" | "done" | "trivial" | "established" | "hypothesis" => Some("assumption"),
        "simp" | "simplify" | "normalize" | "simplification" => Some("simp"),
        "decide" | "compute" | "evaluate" | "calculate" => Some("decide"),
        "omega" | "arithmetic" | "linarith" | "integers" => Some("omega"),
        "crush" | "grind" | "blast" => Some("crush"),
        "split" | "constructor" | "both" | "conjunction" => Some("split"),
        "left" => Some("left"),
        "right" => Some("right"),
        "auto" | "tauto" | "automatically" | "automation" | "trivially" | "directly" => Some("auto"),
        "induction" | "induct" => Some("induction"),
        _ => None,
    }
}

/// Compile a proof-script string into a composed [`Tactic`] (no user-defined tactics).
pub fn parse_script(src: &str) -> Result<Tactic, ScriptError> {
    parse_with_env(src, &TacticEnv::new(), 0)
}

/// Compile a proof-script string with USER-DEFINED tactics resolved from `env` (M).
pub fn parse_script_with_env(src: &str, env: &TacticEnv) -> Result<Tactic, ScriptError> {
    parse_with_env(src, env, 0)
}

fn parse_with_env(src: &str, env: &TacticEnv, depth: usize) -> Result<Tactic, ScriptError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ScriptError("empty script".to_string()));
    }
    let mut p = Parser { toks, pos: 0, env: env.clone(), depth };
    let t = p.parse_seq()?;
    if p.pos != p.toks.len() {
        return Err(ScriptError(format!(
            "trailing tokens after the script: {:?}",
            &p.toks[p.pos..]
        )));
    }
    Ok(t)
}

impl ProofState {
    /// Parse and run a proof script against this state. The state is unchanged on a
    /// parse error; a tactic that fails mid-run leaves whatever it had committed.
    pub fn run_script(&mut self, src: &str) -> Result<&mut Self, ScriptError> {
        let tactic = parse_script(src)?;
        tactic(self).map_err(|e| ScriptError(format!("{e:?}")))?;
        Ok(self)
    }

    /// Parse and run a proof script that may reference USER-DEFINED tactics from `env` (M).
    pub fn run_script_with_env(
        &mut self,
        src: &str,
        env: &TacticEnv,
    ) -> Result<&mut Self, ScriptError> {
        let tactic = parse_script_with_env(src, env)?;
        tactic(self).map_err(|e| ScriptError(format!("{e:?}")))?;
        Ok(self)
    }
}

// =============================================================================
// The certified scripted library (ROOT R6) — a Mathlib-analog seam: theorems
// proved BY TACTIC SCRIPTS accumulate into a dependency-ordered library, each
// proved lemma becoming a citable premise for later ones, every step kernel-checked.
// =============================================================================

/// One theorem of a scripted library: proved from its own `premises` plus the
/// conclusions of the theorems it `cites`, by running `script` to a kernel-checked
/// proof. The unit the [`prove_scripted_library`] driver discharges in citation order.
pub struct ScriptedTheorem {
    pub name: String,
    pub premises: Vec<ProofExpr>,
    pub goal: ProofExpr,
    /// An English-esque proof script (see [`parse_script`]).
    pub script: String,
    /// Names of earlier theorems whose conclusions this proof relies on.
    pub cites: Vec<String>,
    /// `[simp]`-tagged: once proved, this conclusion joins the simp pool and
    /// is in scope for every LATER theorem's `simp` even without a citation.
    pub simp: bool,
}

/// Discharge a library of scripted theorems in citation order, on a shared `axioms`
/// base. Each theorem is proved by its [`ScriptedTheorem::script`] from its premises,
/// the axioms, and the conclusions of the theorems it cites (already proved); a proved
/// conclusion becomes a citable lemma for later theorems — the scraped-Euclid-graph
/// discipline, now driven by tactic scripts. Results come back in INPUT order.
pub fn prove_scripted_library(
    axioms: &[ProofExpr],
    theorems: &[ScriptedTheorem],
) -> Vec<LibraryResult> {
    // `citation_order` works off names + cites; stub the rest.
    let stubs: Vec<LibraryTheorem> = theorems
        .iter()
        .map(|t| LibraryTheorem {
            name: t.name.clone(),
            premises: Vec::new(),
            goal: t.goal.clone(),
            cites: t.cites.clone(),
        })
        .collect();

    let mut proved: HashMap<String, ProofExpr> = HashMap::new();
    let mut by_name: HashMap<String, LibraryResult> = HashMap::new();
    // Conclusions of proved `[simp]`-tagged theorems, in proof order: in scope
    // for every later theorem without an explicit citation.
    let mut simp_pool: Vec<ProofExpr> = Vec::new();

    for &i in &citation_order(&stubs) {
        let t = &theorems[i];
        let mut premises = axioms.to_vec();
        premises.extend(t.premises.iter().cloned());
        for cite in &t.cites {
            if let Some(goal) = proved.get(cite) {
                premises.push(goal.clone());
            }
        }
        for lemma in &simp_pool {
            if !premises.contains(lemma) {
                premises.push(lemma.clone());
            }
        }

        let mut st = ProofState::start(premises, t.goal.clone());
        let (verified, error) = match st.run_script(&t.script).err() {
            Some(e) => (false, Some(e.0)),
            None => match st.qed() {
                Ok(vp) => (vp.verified, vp.verification_error),
                Err(e) => (false, Some(format!("{e:?}"))),
            },
        };
        if verified {
            proved.insert(t.name.clone(), t.goal.clone());
            if t.simp {
                simp_pool.push(t.goal.clone());
            }
        }
        by_name.insert(
            t.name.clone(),
            LibraryResult { name: t.name.clone(), verified, verification_error: error },
        );
    }

    theorems
        .iter()
        .map(|t| by_name.remove(&t.name).expect("every theorem produces a result"))
        .collect()
}
