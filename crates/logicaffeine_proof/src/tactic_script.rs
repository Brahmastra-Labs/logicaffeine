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
//! chain   := atom ('<;>' atom)*          // `t1 <;> t2`: t2 on the goals t1 spawned
//! atom    := '(' script ')'
//!          | 'first' '[' script ('|' script)* ']'   // first that applies (backtracks)
//!          | 'try' atom                              // run if it applies, else no-op
//!          | 'repeat' atom                           // apply until it stops applying
//!          | 'all_goals' atom | 'any_goals' atom     // every / at least one open goal
//!          | 'focus' atom                            // Lean's bullet: must close its goal
//!          | NAME ARG*                               // a primitive tactic
//! term    := factor (('+'|'-') factor)*  // the argument of exact/use/have/refine
//! factor  := app ('*' app)*
//! app     := primary primary*            // juxtaposition: application
//! primary := NAME | '?' NAME? | '(' term (',' term)* ')'
//! ```
//!
//! Primitive tactics: `intro a b`, `exact <term>`, `apply h`, `rfl`, `cases h with a b`,
//! `rewrite h [at <k> | at <hyp> [<k>]]`, `exists <term>`, `assumption`, `simp only [h]`,
//! `decide`, `omega`, `linarith`, `ring`, `crush`, `calc h1 h2 …`, `by_rule <Rule>`, `split`,
//! `left`, `right`, `auto`, `induction n`, `by_contra h`, `have h from <term>`,
//! `refine <term>`.
//!
//! `rewrite`'s `at` clause carries both aspects of positional rewriting: `at <k>` is Lean's
//! `nth_rewrite` (the POSITION, in the goal) and `at <hyp>` is Lean's `rw … at h` (the
//! LOCATION); `at <hyp> <k>` is both.

use std::collections::HashMap;

use crate::tactic::combinators as c;
use crate::tactic::{PTerm, ProofState, Tactic};
use crate::verify::{citation_order, LibraryResult, LibraryTheorem};
use crate::ProofExpr;

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
    /// An arithmetic infix operator, by the `ProofTerm` function it builds.
    Op(&'static str),
    /// The `?` that opens a `refine` hole.
    Question,
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
            // Arithmetic, so a witness can be a compound term (`use (n + 1)`) rather than
            // only a bare name.
            '+' => {
                toks.push(Tok::Op("add"));
                i += 1;
            }
            '-' => {
                toks.push(Tok::Op("sub"));
                i += 1;
            }
            '*' => {
                toks.push(Tok::Op("mul"));
                i += 1;
            }
            '?' => {
                toks.push(Tok::Question);
                i += 1;
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
        self.defs
            .insert(name.to_ascii_lowercase(), script.to_string());
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

    /// Read one IDENTIFIER, looking past connective filler — but never past the last word.
    ///
    /// `skip_filler` used to run unconditionally inside the argument reader, so thirteen
    /// ordinary identifiers (`a`, `an`, `it`, `we`, `on`, `of`, `to`, `the`, `that`, `now`,
    /// `with`, `by`, `and`) were swallowed before the tactic ever saw them: `exists A` and
    /// `cases it` were unspellable, and the reported error blamed the tactic for having no
    /// argument rather than the vocabulary for eating it. A filler word is filler only when
    /// ANOTHER word still follows to be the argument; the last word standing is the
    /// argument, whatever it is spelled.
    fn ident(&mut self) -> Option<String> {
        let mut i = self.pos;
        while matches!(self.toks.get(i), Some(Tok::Word(w)) if is_filler(w)) {
            if matches!(self.toks.get(i + 1), Some(Tok::Word(_))) {
                i += 1;
            } else {
                break;
            }
        }
        match self.toks.get(i) {
            Some(Tok::Word(w)) => {
                let w = w.clone();
                self.pos = i + 1;
                Some(w)
            }
            _ => None,
        }
    }

    /// True if the next token could start ANOTHER name in a list (`intro hp hq`,
    /// `cases h with hp hq`). A word that names a tactic ends the list instead, so a script
    /// that omits a separator still reads the way it always did.
    fn at_extra_name(&self) -> bool {
        match self.peek() {
            Some(Tok::Word(w)) => {
                !self.at_separator() && canonical(w).is_none() && !is_combinator_word(w)
            }
            _ => false,
        }
    }

    /// The rest of a name list, after its first name has been read.
    fn extra_names(&mut self) -> Vec<String> {
        let mut names = Vec::new();
        while self.at_extra_name() {
            match self.bump() {
                Some(Tok::Word(w)) => names.push(w),
                _ => unreachable!("at_extra_name saw a word"),
            }
        }
        names
    }

    /// An optional `at k` clause naming ONE occurrence to rewrite (0-based). Consumed only
    /// when both words are there and the second is a number, so `rewrite h` and a following
    /// step are untouched.
    fn occurrence_clause(&mut self) -> Option<usize> {
        match self.where_clause() {
            Some(RewriteWhere::Goal(k)) => Some(k),
            _ => None,
        }
    }

    /// An optional `at …` clause saying WHERE a rewrite lands.
    ///
    /// Positional rewriting has two aspects and Lean spells them separately — `nth_rewrite 2 [h]`
    /// picks the POSITION, `rw [h] at hp` picks the LOCATION — so this one clause reads both:
    ///
    /// * `at 2` — the goal, at occurrence 2. A number is never a hypothesis name in practice.
    /// * `at hp` — the hypothesis `hp`, at its first occurrence.
    /// * `at hp 2` — the hypothesis `hp`, at occurrence 2.
    ///
    /// `at` is not a filler word and `rewrite` takes no second name of its own, so the position
    /// is unambiguous; anything else after the equation is left for the next step.
    fn where_clause(&mut self) -> Option<RewriteWhere> {
        if !matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("at")) {
            return None;
        }
        let Some(Tok::Word(first)) = self.toks.get(self.pos + 1).cloned() else {
            return None;
        };
        if let Ok(index) = first.parse::<usize>() {
            self.pos += 2;
            return Some(RewriteWhere::Goal(index));
        }
        // `at <name>` — a hypothesis, with an optional occurrence after it.
        let index = match self.toks.get(self.pos + 2) {
            Some(Tok::Word(w)) => w.parse::<usize>().ok(),
            _ => None,
        };
        self.pos += if index.is_some() { 3 } else { 2 };
        Some(RewriteWhere::Hyp(first, index.unwrap_or(0)))
    }

    /// An optional `with a b …` clause naming what an elimination binds.
    fn with_clause(&mut self) -> Vec<String> {
        if !matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("with")) {
            return Vec::new();
        }
        self.bump();
        let mut names = Vec::new();
        if let Some(first) = self.ident() {
            names.push(first);
            names.extend(self.extra_names());
        }
        names
    }

    /// `term := factor (('+'|'-') factor)*` — the argument of `exact`/`use`/`have`/`refine`.
    fn parse_term(&mut self) -> Result<PTerm, ScriptError> {
        let mut lhs = self.parse_factor()?;
        while let Some(Tok::Op(op)) = self.peek() {
            if *op == "mul" {
                break;
            }
            let op = *op;
            self.bump();
            let rhs = self.parse_factor()?;
            lhs = PTerm::Op(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `factor := application ('*' application)*`
    fn parse_factor(&mut self) -> Result<PTerm, ScriptError> {
        let mut lhs = self.parse_application()?;
        while matches!(self.peek(), Some(Tok::Op("mul"))) {
            self.bump();
            let rhs = self.parse_application()?;
            lhs = PTerm::Op("mul", Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `application := primary primary*` — juxtaposition, as in Lean.
    fn parse_application(&mut self) -> Result<PTerm, ScriptError> {
        let head = self.parse_primary()?;
        let mut args = Vec::new();
        while self.at_primary() {
            args.push(self.parse_primary()?);
        }
        Ok(if args.is_empty() {
            head
        } else {
            PTerm::App(Box::new(head), args)
        })
    }

    fn at_primary(&self) -> bool {
        match self.peek() {
            Some(Tok::LParen) | Some(Tok::Question) => true,
            Some(Tok::Word(_)) => !self.at_separator(),
            _ => false,
        }
    }

    /// `primary := NAME | '?' NAME? | '(' term (',' term)* ')'`
    fn parse_primary(&mut self) -> Result<PTerm, ScriptError> {
        match self.peek() {
            Some(Tok::Question) => {
                self.bump();
                // `?_` and `?h` name the same thing — a goal the block leaves open.
                if matches!(self.peek(), Some(Tok::Word(_))) {
                    self.bump();
                }
                Ok(PTerm::Hole)
            }
            Some(Tok::LParen) => {
                self.bump();
                let mut items = vec![self.parse_term()?];
                // `,` lexes as a step separator everywhere else; INSIDE a term it is the
                // anonymous constructor's comma.
                while matches!(self.peek(), Some(Tok::Semi)) {
                    self.bump();
                    if matches!(self.peek(), Some(Tok::RParen)) {
                        break;
                    }
                    items.push(self.parse_term()?);
                }
                self.expect(&Tok::RParen)?;
                Ok(if items.len() == 1 {
                    items.into_iter().next().expect("one item")
                } else {
                    PTerm::Tuple(items)
                })
            }
            Some(Tok::Word(_)) => self
                .ident()
                .map(PTerm::Ident)
                .ok_or_else(|| ScriptError("expected a term".to_string())),
            other => Err(ScriptError(format!("expected a term, found {other:?}"))),
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
            if matches!(
                self.peek(),
                None | Some(Tok::RParen) | Some(Tok::RBracket) | Some(Tok::Bar)
            ) {
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
            // `all_goals <tactic>` — a tactic-TAKING combinator, so it parses like `try`/`repeat`
            // rather than through `parse_primitive`. `combinators::all_goals` has existed and
            // certified all along; it simply had no spelling.
            Some(Tok::Word(w)) if w == "all_goals" || w == "everywhere" => {
                self.bump();
                Ok(c::all_goals(self.parse_atom()?))
            }
            Some(Tok::Word(w)) if w == "any_goals" || w == "somewhere" => {
                self.bump();
                Ok(c::any_goals(self.parse_atom()?))
            }
            // Lean's bullet. A focused block must CLOSE the goal it opened, which is the
            // whole capability: without it the blocks of a flat script silently misalign.
            Some(Tok::Word(w)) if w == "focus" || w == "next" || w == "bullet" => {
                self.bump();
                Ok(c::focus(self.parse_atom()?))
            }
            Some(Tok::Word(_)) => self.parse_primitive(),
            other => Err(ScriptError(format!("expected a tactic, found {other:?}"))),
        }
    }

    fn parse_primitive(&mut self) -> Result<Tactic, ScriptError> {
        let raw = match self.bump() {
            Some(Tok::Word(w)) => w,
            other => {
                return Err(ScriptError(format!(
                    "expected a tactic name, found {other:?}"
                )))
            }
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
        let name = canonical(&raw).ok_or_else(|| ScriptError(format!("unknown tactic `{raw}`")))?;
        // Filler may sit between the verb and its object: `cases on h`, `rewrite with eq`.
        macro_rules! arg {
            () => {
                self.ident()
                    .ok_or_else(|| ScriptError(format!("tactic `{name}` expects an argument")))?
            };
        }
        match name {
            // Lean's `intro a b c` introduces three binders in ONE step; the one-argument
            // atom read `b` as the next tactic name and killed the whole script at parse
            // time, discarding the proof before any step ran.
            "intro" => {
                let first = arg!();
                let rest = self.extra_names();
                Ok(if rest.is_empty() {
                    c::intro(&first)
                } else {
                    c::seq(
                        std::iter::once(first)
                            .chain(rest)
                            .map(|n| c::intro(&n))
                            .collect(),
                    )
                })
            }
            "exact" => Ok(c::exact_term(self.parse_term()?)),
            "apply" => Ok(c::apply(&arg!())),
            "rfl" => Ok(c::rfl()),
            "cases" => {
                let target = arg!();
                let names = self.with_clause();
                Ok(if names.is_empty() {
                    c::cases(&target)
                } else {
                    c::cases_with(&target, names)
                })
            }
            "rewrite" => {
                let hypothesis = arg!();
                // `rewrite h at 2` is Lean's occurrence argument and `rewrite h at hp` is its
                // `rw [h] at hp` — the POSITION and the LOCATION, the two aspects `conv`
                // exists to express. See `Parser::where_clause`.
                match self.where_clause() {
                    Some(RewriteWhere::Goal(k)) => Ok(c::rewrite_at(&hypothesis, k)),
                    Some(RewriteWhere::Hyp(target, k)) => {
                        Ok(c::rewrite_hyp_at(&hypothesis, &target, k))
                    }
                    None => Ok(c::rewrite(&hypothesis)),
                }
            }
            "exists" => Ok(c::exists_term(self.parse_term()?)),
            "assumption" => Ok(c::assumption()),
            "simp" => {
                // `simp only [h₁, h₂]` names its rule set exactly; a bare `simp` uses the
                // whole scope. A list WITHOUT `only` is Lean's "these as well", and since
                // our default set is already the whole scope, naming them adds nothing —
                // but the names are still checked, so a typo is not silently ignored.
                let only =
                    matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("only"));
                if only {
                    self.bump();
                }
                let names = self.bracketed_names()?;
                Ok(match (only, names) {
                    (_, None) => c::simp(),
                    (true, Some(names)) => c::simp_only(names),
                    (false, Some(names)) => c::simp_with(names),
                })
            }
            // `calc h1 h2 …` — the chain, by the names of its steps. The relation may CHANGE
            // along it (`=` then `≤`), which is what `calc` and Lean's `Trans` are for.
            "calc" => {
                let first = arg!();
                let mut names = vec![first];
                names.extend(self.extra_names());
                Ok(c::calc(names))
            }
            // `by_rule R` — the goal is discharged by the NAMED inference rule, in one step.
            // This is the script spelling of the language AST's `ProofStrategy::ByRule`,
            // whose documented example (`Proof: ModusPonens.`) had no image in the proof
            // layer at all.
            "by_rule" | "byrule" => Ok(c::by_rule(&arg!())),
            "decide" => Ok(c::decide()),
            "omega" => Ok(c::omega()),
            "linarith" => Ok(c::linarith()),
            "ring" => Ok(c::ring()),
            "crush" => Ok(c::crush()),
            "split" => Ok(c::split()),
            "left" => Ok(c::left()),
            "right" => Ok(c::right()),
            "auto" => Ok(c::auto()),
            "by_contra" => Ok(c::by_contra(&arg!())),
            "have" => {
                let bound = arg!();
                // `have h from e` / `have h by e` — the connective is optional.
                if matches!(self.peek(), Some(Tok::Word(w)) if is_have_connective(w)) {
                    self.bump();
                }
                Ok(c::have(&bound, self.parse_term()?))
            }
            "refine" => {
                let term = self.parse_term()?;
                if term == PTerm::Hole {
                    return Err(ScriptError(
                        "refine with a bare hole leaves the goal exactly as it was".to_string(),
                    ));
                }
                Ok(c::refine(term))
            }
            // `induction n` NAMES its target, the way Lean's does. Argumentless, it inducts
            // on whatever binder happens to be outermost — which against `∀ m n, Q m n`
            // proves a DIFFERENT theorem, certified, with nothing downstream able to tell.
            "induction" => Ok(match self.at_extra_name().then(|| self.ident()).flatten() {
                Some(variable) => c::induction_on(&variable),
                None => c::induction(),
            }),
            _ => Err(ScriptError(format!("unknown tactic `{name}`"))),
        }
    }

    /// An optional `[a, b, c]` name list.
    fn bracketed_names(&mut self) -> Result<Option<Vec<String>>, ScriptError> {
        if !matches!(self.peek(), Some(Tok::LBracket)) {
            return Ok(None);
        }
        self.bump();
        let mut names = Vec::new();
        loop {
            while matches!(self.peek(), Some(Tok::Semi)) {
                self.bump();
            }
            if matches!(self.peek(), Some(Tok::RBracket)) {
                break;
            }
            match self.bump() {
                Some(Tok::Word(w)) => names.push(w),
                other => {
                    return Err(ScriptError(format!(
                        "expected a lemma name, found {other:?}"
                    )))
                }
            }
        }
        self.expect(&Tok::RBracket)?;
        Ok(Some(names))
    }
}

/// Words that may stand between a `have`'s name and its proof.
fn is_have_connective(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "from" | "by" | "using" | "via" | "proved"
    )
}

/// Words that introduce a tactic-TAKING combinator rather than naming a binder.
fn is_combinator_word(w: &str) -> bool {
    matches!(
        w,
        "first"
            | "try"
            | "repeat"
            | "all_goals"
            | "everywhere"
            | "any_goals"
            | "somewhere"
            | "focus"
            | "next"
            | "bullet"
    )
}

/// Connective filler words that carry no tactic meaning — skipped so a step can be
/// phrased as prose (`by cases on h`, `then by assumption`).
fn is_filler(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "by" | "the"
            | "on"
            | "that"
            | "now"
            | "with"
            | "we"
            | "it"
            | "a"
            | "an"
            | "of"
            | "to"
            | "and"
    )
}

/// Where a `rewrite` lands — see [`Parser::where_clause`].
enum RewriteWhere {
    /// The goal, at this occurrence.
    Goal(usize),
    /// This hypothesis, at this occurrence.
    Hyp(String, usize),
}

/// Map an English-esque verb to its canonical tactic name. `Suppose`/`assume`/`let`
/// all mean `intro`; `automatically`/`trivially` mean `auto`; and so on — the
/// vocabulary that lets a proof read like prose. Case-insensitive. Public so an editor
/// highlights exactly the words the engine accepts.
pub fn canonical(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "intro" | "assume" | "suppose" | "let" | "introduce" | "given" | "fix" => Some("intro"),
        "exact" | "from" | "because" => Some("exact"),
        // `apply` reaches `ProofState::apply_named`; the capability is modus ponens against a
        // named rule and has always certified — only the spelling was missing.
        "apply" | "via" => Some("apply"),
        // `rfl` names the reflexivity closer that previously only `simp` could reach.
        "rfl" | "reflexivity" => Some("rfl"),
        "cases" | "destruct" | "consider" | "casework" => Some("cases"),
        "rewrite" | "rw" | "substitute" | "subst" => Some("rewrite"),
        "exists" | "use" | "witness" | "choose" | "provide" => Some("exists"),
        "assumption" | "done" | "trivial" | "established" | "hypothesis" => Some("assumption"),
        "simp" | "simplify" | "normalize" | "simplification" => Some("simp"),
        "decide" | "compute" | "evaluate" | "calculate" => Some("decide"),
        // `linarith` is NO LONGER an alias for `omega`. `omega` tightens `a < b` to `a + 1 ≤ b`,
        // which is sound over ℤ and unsound over an ordered field — and `ProofExpr` is untyped, so
        // nothing records the carrier. They are different tactics and now have different names.
        "omega" | "arithmetic" | "integers" => Some("omega"),
        "linarith" | "rationals" => Some("linarith"),
        "crush" | "grind" | "blast" => Some("crush"),
        "split" | "constructor" | "both" | "conjunction" => Some("split"),
        "left" => Some("left"),
        "right" => Some("right"),
        "auto" | "tauto" | "automatically" | "automation" | "trivially" | "directly" => {
            Some("auto")
        }
        "induction" | "induct" => Some("induction"),
        // `by_contra h` is a STEP that assumes ¬G and leaves ⊥, so the twenty steps of a
        // classical proof can be written; the classical reductio was reachable only as one
        // opaque call inside `auto`'s search before.
        "by_contra" | "bycontra" | "contra" | "contradiction" => Some("by_contra"),
        // `ring` was reachable only from the literate kernel surface; the same
        // commutative-ring oracle now has a name on this one.
        "ring" | "polynomial" => Some("ring"),
        // `calc` — a chain whose relation may change along it. Named steps, like
        // `simp only [h]` names its rule set. ONE spelling deliberately: a canonical word
        // stops an argument list (`at_extra_name`), so every alias added here is an ordinary
        // identifier taken out of a user's hands — and `chain`/`compose` are ordinary words.
        "calc" => Some("calc"),
        // `by_rule R` — one step, by the name of the rule that takes it.
        "by_rule" | "byrule" => Some("by_rule"),
        "have" | "note" | "observe" => Some("have"),
        "refine" | "partially" => Some("refine"),
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
    let mut p = Parser {
        toks,
        pos: 0,
        env: env.clone(),
        depth,
    };
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
            LibraryResult {
                name: t.name.clone(),
                verified,
                verification_error: error,
            },
        );
    }

    theorems
        .iter()
        .map(|t| {
            by_name
                .remove(&t.name)
                .expect("every theorem produces a result")
        })
        .collect()
}
