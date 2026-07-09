//! REAL logic test of the propositional module (`06_propositional`, 114 exercises).
//!
//! Structural tests (see `learn_examples.rs`) prove the options are well-formed.
//! This file proves they are *logically correct*: for every exercise we
//!
//!   1. parse each LaTeX option (`\cdot \vee \supset \equiv \sim`) into a formula,
//!   2. parse the English prompt — written in the module's controlled,
//!      Quine-style grammar ("both A and B", "either A or B", "if A then B",
//!      "neither A nor B", "A unless B", "A only if B", "A iff B",
//!      "A is necessary/sufficient for B", "not A") — into a formula, and
//!   3. assert the *marked-correct* option is logically equivalent to the
//!      prompt's meaning by an exhaustive truth table over all variables.
//!
//! If the answer key is wrong (e.g. "tall and naive, or else dangerous" tagged
//! `(T∨N)∨D` instead of `(T·N)∨D`), the truth tables differ and this fails.
//! It also fails if any prompt no longer parses — so new content cannot quietly
//! slip past the logic check. This harness caught and drove the fixes for two
//! real answer-key bugs (C_6.19, C_6.101).

use std::collections::{BTreeSet, HashMap};

use logicaffeine_web::content::{ContentEngine, ExerciseType};

// ----------------------------- formula AST ---------------------------------

#[derive(Clone, Debug)]
enum F {
    Var(char),
    Not(Box<F>),
    Bin(char, Box<F>, Box<F>), // op ∈ {'&','|','>','='}
}

fn eval(f: &F, env: &HashMap<char, bool>) -> bool {
    match f {
        F::Var(c) => env[c],
        F::Not(a) => !eval(a, env),
        F::Bin(op, a, b) => {
            let (x, y) = (eval(a, env), eval(b, env));
            match op {
                '&' => x && y,
                '|' => x || y,
                '>' => !x || y,
                '=' => x == y,
                _ => unreachable!(),
            }
        }
    }
}

fn collect(f: &F, out: &mut BTreeSet<char>) {
    match f {
        F::Var(c) => {
            out.insert(*c);
        }
        F::Not(a) => collect(a, out),
        F::Bin(_, a, b) => {
            collect(a, out);
            collect(b, out);
        }
    }
}

/// Exhaustive truth-table equivalence over the union of variables.
fn equivalent(a: &F, b: &F) -> bool {
    let mut vs = BTreeSet::new();
    collect(a, &mut vs);
    collect(b, &mut vs);
    let vars: Vec<char> = vs.into_iter().collect();
    for mask in 0u32..(1u32 << vars.len()) {
        let env: HashMap<char, bool> = vars
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, (mask >> i) & 1 == 1))
            .collect();
        if eval(a, &env) != eval(b, &env) {
            return false;
        }
    }
    true
}

// --------------------------- LaTeX formula parser --------------------------

#[derive(Clone, PartialEq, Debug)]
enum FT {
    Op(char), // ( ) ~ & | > =
    Var(char),
}

fn ftok(s: &str) -> Result<Vec<FT>, String> {
    let mut s = s.trim().trim_matches('$').trim().to_string();
    for (a, b) in [
        ("\\sim", " ~ "),
        ("\\cdot", " & "),
        ("\\vee", " | "),
        ("\\supset", " > "),
        ("\\equiv", " = "),
        ("\\,", " "),
        ("\\ ", " "),
    ] {
        s = s.replace(a, b);
    }
    let mut out = Vec::new();
    for ch in s.chars() {
        if ch.is_whitespace() {
            continue;
        }
        match ch {
            '(' | ')' | '~' | '&' | '|' | '>' | '=' => out.push(FT::Op(ch)),
            c if c.is_ascii_alphabetic() => out.push(FT::Var(c.to_ascii_uppercase())),
            other => return Err(format!("bad char {other:?} in formula {s:?}")),
        }
    }
    Ok(out)
}

struct FParser {
    t: Vec<FT>,
    i: usize,
}
impl FParser {
    fn peek(&self) -> Option<&FT> {
        self.t.get(self.i)
    }
    fn parse(&mut self) -> Result<F, String> {
        let n = self.iff()?;
        if self.i != self.t.len() {
            return Err("trailing tokens".into());
        }
        Ok(n)
    }
    fn iff(&mut self) -> Result<F, String> {
        let mut l = self.imp()?;
        while self.peek() == Some(&FT::Op('=')) {
            self.i += 1;
            l = F::Bin('=', Box::new(l), Box::new(self.imp()?));
        }
        Ok(l)
    }
    fn imp(&mut self) -> Result<F, String> {
        let l = self.disj()?;
        if self.peek() == Some(&FT::Op('>')) {
            self.i += 1;
            return Ok(F::Bin('>', Box::new(l), Box::new(self.imp()?)));
        }
        Ok(l)
    }
    fn disj(&mut self) -> Result<F, String> {
        let mut l = self.conj()?;
        while self.peek() == Some(&FT::Op('|')) {
            self.i += 1;
            l = F::Bin('|', Box::new(l), Box::new(self.conj()?));
        }
        Ok(l)
    }
    fn conj(&mut self) -> Result<F, String> {
        let mut l = self.un()?;
        while self.peek() == Some(&FT::Op('&')) {
            self.i += 1;
            l = F::Bin('&', Box::new(l), Box::new(self.un()?));
        }
        Ok(l)
    }
    fn un(&mut self) -> Result<F, String> {
        match self.peek().cloned() {
            Some(FT::Op('~')) => {
                self.i += 1;
                Ok(F::Not(Box::new(self.un()?)))
            }
            Some(FT::Op('(')) => {
                self.i += 1;
                let n = self.iff()?;
                if self.peek() != Some(&FT::Op(')')) {
                    return Err("missing )".into());
                }
                self.i += 1;
                Ok(n)
            }
            Some(FT::Var(c)) => {
                self.i += 1;
                Ok(F::Var(c))
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}
fn parse_formula(s: &str) -> Result<F, String> {
    FParser { t: ftok(s)?, i: 0 }.parse()
}

// --------------------- controlled-English prompt parser --------------------

#[derive(Clone, PartialEq, Debug)]
enum E {
    Both,
    And,
    Either,
    Or,
    If,
    Then,
    Not,
    Neither,
    Nor,
    Unless,
    Iff,
    OnlyIf,
    NotAll,
    IffRev,
    SufFor,
    NecFor,
    Comma,
    Atom(char),
}

fn etokens(prompt: &str) -> Vec<E> {
    let mut s = prompt.to_lowercase();
    s = s.replace('.', " ").replace('"', " ");
    s = s
        .replace("you're", " ")
        .replace("you aren't", " not ")
        .replace("aren't", " not ")
        .replace("isn't", " not ")
        .replace(" you ", " ")
        .replace("it is false that", " NOT_ALL ")
        .replace("if and only if", " iff ")
        .replace("just if", " iff ")
        .replace("only if", " ONLYIF ")
        .replace("necessary and sufficient for", " IFFREV ")
        .replace("is sufficient for you to be", " SUFFOR ")
        .replace("is necessary for you to be", " NECFOR ")
        .replace("sufficient for you to be", " SUFFOR ")
        .replace("necessary for you to be", " NECFOR ")
        .replace("is sufficient for", " SUFFOR ")
        .replace("is necessary for", " NECFOR ")
        .replace("sufficient for", " SUFFOR ")
        .replace("necessary for", " NECFOR ")
        .replace("being", " ")
        .replace(" to be ", " ")
        .replace("provided that", " if ")
        .replace("or else", " or ")
        .replace(",", " , ");
    let mut out = Vec::new();
    for w in s.split_whitespace() {
        let tok = match w {
            "both" => E::Both,
            "and" => E::And,
            "either" => E::Either,
            "or" => E::Or,
            "if" => E::If,
            "then" => E::Then,
            "not" => E::Not,
            "neither" => E::Neither,
            "nor" => E::Nor,
            "unless" => E::Unless,
            "iff" => E::Iff,
            "ONLYIF" => E::OnlyIf,
            "NOT_ALL" => E::NotAll,
            "IFFREV" => E::IffRev,
            "SUFFOR" => E::SufFor,
            "NECFOR" => E::NecFor,
            "," => E::Comma,
            "is" | "are" | "a" | "that" | "to" | "be" => continue,
            other => {
                let c = other.chars().next().unwrap();
                if other.chars().all(|c| c.is_ascii_lowercase()) {
                    E::Atom(c.to_ascii_uppercase())
                } else {
                    continue;
                }
            }
        };
        out.push(tok);
    }
    out
}

/// Recursive-descent parser for the controlled grammar. Mirrors the validated
/// reference implementation; functions take a slice + start index and return
/// `(node, next_index)`, so the comma-split can recurse on sub-slices.
struct EParser;
impl EParser {
    fn top(t: &[E]) -> Result<F, String> {
        let (n, i) = Self::disj(t, 0)?;
        if i != t.len() {
            return Err(format!("trailing {:?}", &t[i..]));
        }
        Ok(n)
    }
    /// A top-level ", and"/", or" is the principal connective (both/either/neither
    /// open a scope; `if` does not).
    fn split_comma(t: &[E]) -> Option<usize> {
        let mut d = 0i32;
        for k in 0..t.len().saturating_sub(1) {
            match t[k] {
                E::Both | E::Either | E::Neither => d += 1,
                _ => {}
            }
            if t[k] == E::Comma && d == 0 && matches!(t[k + 1], E::And | E::Or) {
                return Some(k);
            }
        }
        None
    }
    fn disj(t: &[E], i: usize) -> Result<(F, usize), String> {
        if let Some(k) = Self::split_comma(t) {
            let (left, _) = Self::disj(&t[..k], 0)?;
            let op = if t[k + 1] == E::And { '&' } else { '|' };
            let (right, _) = Self::disj(&t[k + 2..], 0)?;
            return Ok((F::Bin(op, Box::new(left), Box::new(right)), t.len()));
        }
        if t.get(i) == Some(&E::NotAll) {
            let (n, j) = Self::disj(t, i + 1)?;
            return Ok((F::Not(Box::new(n)), j));
        }
        Self::iff(t, i)
    }
    fn iff(t: &[E], i: usize) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::imp(t, i)?;
        while matches!(t.get(i), Some(E::Iff) | Some(E::IffRev)) {
            i += 1;
            let (r, ni) = Self::imp(t, i)?;
            i = ni;
            l = F::Bin('=', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
    fn imp(t: &[E], i: usize) -> Result<(F, usize), String> {
        let (l, i) = Self::disju(t, i)?;
        match t.get(i) {
            Some(E::OnlyIf) => {
                let (r, ni) = Self::disju(t, i + 1)?;
                Ok((F::Bin('>', Box::new(l), Box::new(r)), ni)) // A only if B == A->B
            }
            Some(E::Unless) => {
                let (r, ni) = Self::disju(t, i + 1)?;
                Ok((F::Bin('|', Box::new(r), Box::new(l)), ni)) // A unless B == A∨B
            }
            Some(E::If) => {
                let (r, ni) = Self::disju(t, i + 1)?;
                Ok((F::Bin('>', Box::new(r), Box::new(l)), ni)) // A if B == B->A
            }
            Some(E::SufFor) => {
                let (r, ni) = Self::disju(t, i + 1)?;
                Ok((F::Bin('>', Box::new(l), Box::new(r)), ni)) // A sufficient for B == A->B
            }
            Some(E::NecFor) => {
                let (r, ni) = Self::disju(t, i + 1)?;
                Ok((F::Bin('>', Box::new(r), Box::new(l)), ni)) // A necessary for B == B->A
            }
            _ => Ok((l, i)),
        }
    }
    fn disju(t: &[E], i: usize) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::conj(t, i)?;
        while t.get(i) == Some(&E::Or) {
            i += 1;
            let (r, ni) = Self::conj(t, i)?;
            i = ni;
            l = F::Bin('|', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
    fn conj(t: &[E], i: usize) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::un(t, i)?;
        while t.get(i) == Some(&E::And) {
            i += 1;
            let (r, ni) = Self::un(t, i)?;
            i = ni;
            l = F::Bin('&', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
    fn un(t: &[E], i: usize) -> Result<(F, usize), String> {
        let Some(x) = t.get(i) else {
            return Err("unexpected end".into());
        };
        match x {
            E::Not => {
                let (n, ni) = Self::un(t, i + 1)?;
                Ok((F::Not(Box::new(n)), ni))
            }
            E::NotAll => {
                let (n, ni) = Self::disj(t, i + 1)?;
                Ok((F::Not(Box::new(n)), ni))
            }
            E::Unless => {
                let (l, mut i) = Self::disju_until(t, i + 1, true)?;
                if t.get(i) == Some(&E::Comma) {
                    i += 1;
                }
                let (r, ni) = Self::disju(t, i)?;
                Ok((F::Bin('|', Box::new(l), Box::new(r)), ni))
            }
            E::OnlyIf => {
                let (nec, mut i) = Self::un(t, i + 1)?;
                if t.get(i) == Some(&E::Comma) {
                    i += 1;
                }
                let (rest, ni) = Self::disju(t, i)?;
                Ok((F::Bin('>', Box::new(rest), Box::new(nec)), ni))
            }
            E::If => {
                let (ant, mut i) = Self::scope_if(t, i + 1)?;
                if t.get(i) == Some(&E::Comma) {
                    i += 1;
                }
                if t.get(i) == Some(&E::Then) {
                    i += 1;
                }
                let (cons, ni) = Self::disju(t, i)?;
                Ok((F::Bin('>', Box::new(ant), Box::new(cons)), ni))
            }
            E::Both => {
                let (l, i) = Self::conj_until(t, i + 1, true)?;
                if t.get(i) != Some(&E::And) {
                    return Err("both without and".into());
                }
                let (r, ni) = Self::un(t, i + 1)?;
                Ok((F::Bin('&', Box::new(l), Box::new(r)), ni))
            }
            E::Either => {
                let (l, i) = Self::conj_until(t, i + 1, false)?;
                if t.get(i) != Some(&E::Or) {
                    return Err("either without or".into());
                }
                let (r, ni) = Self::un(t, i + 1)?;
                Ok((F::Bin('|', Box::new(l), Box::new(r)), ni))
            }
            E::Neither => {
                let (l, i) = Self::conj_until(t, i + 1, false)?;
                if t.get(i) != Some(&E::Nor) {
                    return Err("neither without nor".into());
                }
                let (r, ni) = Self::un(t, i + 1)?;
                Ok((
                    F::Not(Box::new(F::Bin('|', Box::new(l), Box::new(r)))),
                    ni,
                ))
            }
            E::Atom(c) => Ok((F::Var(*c), i + 1)),
            other => Err(format!("cannot start a unit with {other:?}")),
        }
    }
    /// Left operand of both/either/neither: a conjunction, but for `both` we must
    /// NOT consume the separating `and`.
    fn conj_until(t: &[E], i: usize, stop_on_and: bool) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::un(t, i)?;
        while t.get(i) == Some(&E::And) && !stop_on_and {
            i += 1;
            let (r, ni) = Self::un(t, i)?;
            i = ni;
            l = F::Bin('&', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
    /// Antecedent of `if`: a disjunction that stops at `then`/comma.
    fn scope_if(t: &[E], i: usize) -> Result<(F, usize), String> {
        Self::disju_until(t, i, false)
    }
    fn disju_until(t: &[E], i: usize, _unused: bool) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::conj_until2(t, i)?;
        while t.get(i) == Some(&E::Or) {
            i += 1;
            let (r, ni) = Self::conj_until2(t, i)?;
            i = ni;
            l = F::Bin('|', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
    fn conj_until2(t: &[E], i: usize) -> Result<(F, usize), String> {
        let (mut l, mut i) = Self::un(t, i)?;
        while t.get(i) == Some(&E::And) {
            i += 1;
            let (r, ni) = Self::un(t, i)?;
            i = ni;
            l = F::Bin('&', Box::new(l), Box::new(r));
        }
        Ok((l, i))
    }
}
fn parse_english(prompt: &str) -> Result<F, String> {
    EParser::top(&etokens(prompt))
}

// --------------------------------- test ------------------------------------

/// Propositional exercises that are CONCEPT questions ("Which formula is a
/// tautology?", "P → Q is equivalent to…"), NOT English→formula translations.
/// The truth-table verifier below applies only to translation exercises, so
/// these are skipped here — their correctness is covered by structural checks
/// and review. The list is asserted to reference only real exercises, so it
/// can't silently rot, and every OTHER unparseable prompt still fails the test.
const CONCEPT_EXERCISE_IDS: &[&str] = &[
    "C_6.115", "C_6.116", "C_6.117", "C_6.118", "C_6.119", "C_6.120",
];

#[test]
fn every_propositional_answer_is_logically_correct() {
    let engine = ContentEngine::new();
    let module = engine
        .get_module("building-blocks", "propositional")
        .expect("propositional module");

    // The concept-exercise allowlist must reference exercises that actually exist.
    let present: std::collections::HashSet<&str> =
        module.exercises.iter().map(|e| e.id.as_str()).collect();
    let stale: Vec<&&str> = CONCEPT_EXERCISE_IDS
        .iter()
        .filter(|id| !present.contains(**id))
        .collect();
    assert!(
        stale.is_empty(),
        "CONCEPT_EXERCISE_IDS references exercises that no longer exist: {stale:?}"
    );

    let mut mismatches = Vec::new();
    let mut unparsed = Vec::new();
    let mut checked = 0usize;
    let mut concept = 0usize;

    for ex in &module.exercises {
        if ex.exercise_type != ExerciseType::MultipleChoice {
            continue;
        }
        if CONCEPT_EXERCISE_IDS.contains(&ex.id.as_str()) {
            concept += 1;
            continue;
        }
        let (Some(opts), Some(ci)) = (&ex.options, ex.correct) else {
            continue;
        };

        let english = match parse_english(&ex.prompt) {
            Ok(f) => f,
            Err(e) => {
                unparsed.push(format!("[{}] {:?} — english parse failed: {e}", ex.id, ex.prompt));
                continue;
            }
        };
        let marked = match parse_formula(&opts[ci]) {
            Ok(f) => f,
            Err(e) => {
                unparsed.push(format!("[{}] option {ci} {:?} — formula parse failed: {e}", ex.id, opts[ci]));
                continue;
            }
        };

        checked += 1;
        if !equivalent(&english, &marked) {
            // Which option(s) DO match the English meaning?
            let matches: Vec<usize> = opts
                .iter()
                .enumerate()
                .filter(|(_, o)| parse_formula(o).map(|f| equivalent(&english, &f)).unwrap_or(false))
                .map(|(i, _)| i)
                .collect();
            mismatches.push(format!(
                "[{}] {:?}\n     marked[{ci}] = {:?} is NOT equivalent to the prompt; \
                 the English actually means option(s) {:?}",
                ex.id, ex.prompt, opts[ci], matches
            ));
        }
    }

    eprintln!(
        "propositional logic: {checked} translation answers verified by truth table; \
         {concept} concept exercises skipped"
    );

    assert!(
        unparsed.is_empty(),
        "\n{} propositional prompt(s)/formula(s) no longer parse — extend the grammar or fix the content:\n  {}\n",
        unparsed.len(),
        unparsed.join("\n  ")
    );
    assert!(
        mismatches.is_empty(),
        "\n{} propositional answer key(s) are logically WRONG:\n  {}\n",
        mismatches.len(),
        mismatches.join("\n  ")
    );
}
