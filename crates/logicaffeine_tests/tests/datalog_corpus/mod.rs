//! Datalog programs as the standard library's `datalog` module evaluates them, shared by every lock that holds a tier
//! to `logicaffeine_proof::datalog` on them (`datalog_in_logos_lock` for the interpreted tiers, AOT-Rust and C;
//! `wasm_aot_lock` for WASM execution), so the two cannot drift apart.
//!
//! Each [`Program`] becomes a procedure that builds its facts and rules with the module's builders, evaluates them with
//! `datalogEvaluate` and shows every fact of the model, or `refused: ` and the reason; [`logos_programs`] calls them in
//! turn, each after a line naming it. [`expected`] is what a procedure must print, read off the Rust engine: the
//! model's facts as a set (the LOGOS program prints them in the order it derives them), or the refusal in the Rust
//! engine's own words.

#![allow(dead_code)]

use logicaffeine_language::lexer::text_literal;
use logicaffeine_proof::datalog::{evaluate, Atom, Literal, Program, Rule, Term};
use std::collections::BTreeSet;

/// What the LOGOS program evaluating a Datalog program prints: every fact of its model, or its refusal.
#[derive(Debug, PartialEq, Eq)]
pub enum Printed {
    Model(BTreeSet<String>),
    Refused(String),
}

fn texts(values: &[String]) -> String {
    if values.is_empty() {
        return "a new Seq of Text".to_string();
    }
    format!("[{}]", values.iter().map(|v| text_literal(v)).collect::<Vec<_>>().join(", "))
}

fn atom(a: &Atom) -> String {
    let terms = if a.terms.is_empty() {
        "a new Seq of DatalogTerm".to_string()
    } else {
        let terms: Vec<String> = a
            .terms
            .iter()
            .map(|t| match t {
                Term::Var(v) => format!("datalogVar({})", text_literal(v)),
                Term::Const(c) => format!("datalogConst({})", text_literal(c)),
            })
            .collect();
        format!("[{}]", terms.join(", "))
    };
    format!("datalogAtom({}, {terms})", text_literal(&a.predicate))
}

fn rule(r: &Rule) -> String {
    let body = if r.body.is_empty() {
        "a new Seq of DatalogLiteral".to_string()
    } else {
        let literals: Vec<String> = r
            .body
            .iter()
            .map(|l| match l {
                Literal::Pos(a) => format!("datalogHolds({})", atom(a)),
                Literal::Neg(a) => format!("datalogNot({})", atom(a)),
            })
            .collect();
        format!("[{}]", literals.join(", "))
    };
    format!("datalogRule({}, {}, {body})", text_literal(&r.name), atom(&r.head))
}

/// The statements that evaluate `p` with the standard library and show what it derives, at `indent`.
fn statements(p: &Program, indent: &str) -> String {
    let mut lines = vec!["Let mutable facts be a new Seq of DatalogFact.".to_string()];
    for f in p.facts() {
        let values = f.ground().expect("a fact is ground");
        lines.push(format!("Push datalogFact({}, {}) to facts.", text_literal(&f.predicate), texts(&values)));
    }
    lines.push("Let mutable rules be a new Seq of DatalogRule.".to_string());
    for r in p.rules() {
        lines.push(format!("Push {} to rules.", rule(r)));
    }
    lines.extend(
        [
            "Let outcome be datalogEvaluate(facts, rules).",
            "Inspect outcome:",
            "    When DatalogModel (model):",
            "        Repeat for f in model:",
            "            Show datalogShowFact(f).",
            "    When DatalogRefusal (reason):",
            "        Show \"refused: \" + reason.",
        ]
        .map(str::to_string),
    );
    lines.iter().map(|l| format!("{indent}{l}\n")).collect()
}

/// The LOGOS program that evaluates each of `programs` in turn, each in a procedure of its own, after a line
/// `== name` naming it.
pub fn logos_programs(programs: &[(String, Program)]) -> String {
    let mut src = String::new();
    let mut main = String::from("## Main\n");
    for (i, (name, p)) in programs.iter().enumerate() {
        src.push_str(&format!("## To evaluate{i} ():\n{}\n", statements(p, "    ")));
        main.push_str(&format!("Show {}.\nCall evaluate{i}.\n", text_literal(&format!("== {name}"))));
    }
    src + &main
}

/// The sections of a run of [`logos_programs`], by name.
pub fn sections(output: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            out.push((name.to_string(), String::new()));
        } else if let Some((_, text)) = out.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    out
}

/// What the procedure evaluating `p` must print, from the Rust engine.
pub fn expected(p: &Program) -> Printed {
    match evaluate(p) {
        Ok(model) => Printed::Model(
            model.facts().into_iter().map(|(pred, args)| format!("{pred}({})", args.join(", "))).collect(),
        ),
        Err(e) => Printed::Refused(e.to_string()),
    }
}

/// What one section of a run of [`logos_programs`] printed, read back: the refusal, or the facts — each printed once, which is
/// asserted here, since a fact printed twice is a model the set would hide.
pub fn printed(output: &str) -> Printed {
    let lines: Vec<&str> = output.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    if let [only] = lines.as_slice() {
        if let Some(reason) = only.strip_prefix("refused: ") {
            return Printed::Refused(reason.to_string());
        }
    }
    let facts: BTreeSet<String> = lines.iter().map(|l| l.to_string()).collect();
    assert_eq!(facts.len(), lines.len(), "a fact was printed twice:\n{output}");
    Printed::Model(facts)
}

/// The given facts, each once, in the order given: the first lines a model prints.
pub fn given_facts(p: &Program) -> Vec<String> {
    let mut seen = BTreeSet::new();
    p.facts()
        .iter()
        .map(|f| format!("{}({})", f.predicate, f.ground().expect("ground").join(", ")))
        .filter(|line| seen.insert(line.clone()))
        .collect()
}

fn var(v: &str) -> Term {
    Term::var(v)
}

fn constant(c: &str) -> Term {
    Term::constant(c)
}

/// Whaley and Lam's four context-insensitive points-to rules (PLDI 2004, Algorithm 1) over the PDF's seven
/// statements.
pub fn points_to() -> Program {
    let fact = |p: &str, args: &[&str]| Atom::new(p, args.iter().map(|a| constant(a)).collect());
    let at = |p: &str, args: &[&str]| Atom::new(p, args.iter().map(|a| var(a)).collect());
    let facts = vec![
        fact("vP0", &["a", "h1"]),
        fact("vP0", &["b", "h2"]),
        fact("assign", &["c", "a"]),
        fact("store", &["c", "next", "b"]),
        fact("load", &["a", "next", "d"]),
        fact("vP0", &["e", "h3"]),
        fact("store", &["a", "next", "e"]),
    ];
    let rules = vec![
        Rule::new("alloc", at("vP", &["v", "h"]), vec![Literal::Pos(at("vP0", &["v", "h"]))]),
        Rule::new(
            "assign",
            at("vP", &["v1", "h"]),
            vec![Literal::Pos(at("assign", &["v1", "v2"])), Literal::Pos(at("vP", &["v2", "h"]))],
        ),
        Rule::new(
            "store",
            at("hP", &["h1", "f", "h2"]),
            vec![
                Literal::Pos(at("store", &["v1", "f", "v2"])),
                Literal::Pos(at("vP", &["v1", "h1"])),
                Literal::Pos(at("vP", &["v2", "h2"])),
            ],
        ),
        Rule::new(
            "load",
            at("vP", &["v2", "h2"]),
            vec![
                Literal::Pos(at("load", &["v1", "f", "v2"])),
                Literal::Pos(at("vP", &["v1", "h1"])),
                Literal::Pos(at("hP", &["h1", "f", "h2"])),
            ],
        ),
    ];
    Program::new(facts, rules)
}

/// Constants a netstring-free encoding of a tuple would confuse — `("1:x", "y")` and `("", "1:xy")` joined end to
/// end are the same text — with a separator, a brace, a quote and an empty one among them.
pub fn adversarial_constants() -> Program {
    let fact = |args: &[&str]| Atom::new("q", args.iter().map(|a| constant(a)).collect());
    let facts = vec![
        fact(&["1:x", "y"]),
        fact(&["", "1:xy"]),
        fact(&["a, b", "{c}"]),
        fact(&["a", " b, {c}"]),
        fact(&["\"", "\\"]),
    ];
    let rules = vec![Rule::new(
        "swap",
        Atom::new("r", vec![var("Y"), var("X")]),
        vec![Literal::Pos(Atom::new("q", vec![var("X"), var("Y")]))],
    )];
    Program::new(facts, rules)
}

/// The three ways a program has no stratified least model, each refused in the Rust engine's words: a predicate used
/// with two arities, a head variable nothing positive binds, and a negation inside a cycle.
pub fn refused() -> Vec<(&'static str, Program)> {
    let unary = |p: &str, v: &str| Atom::new(p, vec![var(v)]);
    vec![
        (
            "two arities",
            Program::new(
                vec![Atom::new("e", vec![constant("a")])],
                vec![Rule::new(
                    "r",
                    unary("p", "X"),
                    vec![Literal::Pos(Atom::new("e", vec![var("X"), var("Y")]))],
                )],
            ),
        ),
        (
            "an unbound head variable",
            Program::new(
                vec![Atom::new("e", vec![constant("a")])],
                vec![Rule::new("r", Atom::new("p", vec![var("X"), var("Y")]), vec![Literal::Pos(unary("e", "X"))])],
            ),
        ),
        (
            "an unbound negated variable",
            Program::new(
                vec![Atom::new("e", vec![constant("a")])],
                vec![Rule::new(
                    "r",
                    unary("p", "X"),
                    vec![Literal::Pos(unary("e", "X")), Literal::Neg(unary("f", "Z"))],
                )],
            ),
        ),
        (
            "a negation inside a cycle",
            Program::new(
                vec![Atom::new("e", vec![constant("a")])],
                vec![
                    Rule::new("r1", unary("p", "X"), vec![Literal::Pos(unary("e", "X")), Literal::Neg(unary("q", "X"))]),
                    Rule::new("r2", unary("q", "X"), vec![Literal::Pos(unary("p", "X")), Literal::Pos(unary("e", "X"))]),
                    Rule::new("r3", unary("s", "X"), vec![Literal::Pos(unary("q", "X"))]),
                ],
            ),
        ),
    ]
}

/// A deterministic stream of choices.
pub struct SplitMix(pub u64);

impl SplitMix {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A random stratified program: base relations `e` (binary) and `t` (ternary) and a nullary `z`, given as facts, and
/// derived relations `p0` … `p3` at levels 0 … 3, `p0` and `p2` binary, `p1` ternary, `p3` unary. A rule for `pᵢ`
/// reads base relations and derived ones at its own level or below (so recursion through `pᵢ` itself), denies only
/// base relations and derived ones strictly below it, and mixes constants and repeated variables into its atoms;
/// every head and negated variable is bound by a positive literal.
pub fn random_program(rng: &mut SplitMix) -> Program {
    const CONSTANTS: [&str; 5] = ["c0", "c1", "c2", "c3", "c4"];
    const VARS: [&str; 4] = ["W", "X", "Y", "Z"];
    let arity = |p: &str| match p {
        "z" => 0,
        "p3" => 1,
        "e" | "p0" | "p2" => 2,
        _ => 3,
    };
    let mut facts = Vec::new();
    for _ in 0..rng.below(10) + 3 {
        facts.push(Atom::new("e", vec![constant(CONSTANTS[rng.below(5)]), constant(CONSTANTS[rng.below(5)])]));
    }
    for _ in 0..rng.below(5) {
        let args = (0..3).map(|_| constant(CONSTANTS[rng.below(5)])).collect();
        facts.push(Atom::new("t", args));
    }
    if rng.below(2) == 0 {
        facts.push(Atom::new("z", Vec::new()));
    }
    let mut rules = Vec::new();
    for r in 0..rng.below(6) + 2 {
        let level = rng.below(4);
        let head = format!("p{level}");
        let readable: Vec<String> =
            ["e", "t", "z"].iter().map(|s| s.to_string()).chain((0..=level).map(|l| format!("p{l}"))).collect();
        let deniable: Vec<String> =
            ["e", "t", "z"].iter().map(|s| s.to_string()).chain((0..level).map(|l| format!("p{l}"))).collect();
        let term = |rng: &mut SplitMix| {
            if rng.below(5) == 0 {
                constant(CONSTANTS[rng.below(5)])
            } else {
                var(VARS[rng.below(4)])
            }
        };
        let mut body = Vec::new();
        for _ in 0..rng.below(3) {
            let pred = readable[rng.below(readable.len())].clone();
            let args = (0..arity(&pred)).map(|_| term(rng)).collect();
            body.push(Literal::Pos(Atom::new(&pred, args)));
        }
        // Bind every variable the head or a negation may use: a base literal over all four.
        body.push(Literal::Pos(Atom::new("e", vec![var("W"), var("X")])));
        body.push(Literal::Pos(Atom::new("e", vec![var("Y"), var("Z")])));
        if rng.below(2) == 0 {
            let pred = deniable[rng.below(deniable.len())].clone();
            let args = (0..arity(&pred)).map(|_| term(rng)).collect();
            body.push(Literal::Neg(Atom::new(&pred, args)));
        }
        let n = body.len();
        let at = rng.below(n);
        body.rotate_left(at);
        let args = (0..arity(&head)).map(|_| term(rng)).collect();
        rules.push(Rule::new(format!("r{r}"), Atom::new(&head, args), body));
    }
    Program::new(facts, rules)
}

/// The four rules, the adversarial constants, four random programs and the refused ones, by name.
pub fn sampled() -> Vec<(String, Program)> {
    let mut rng = SplitMix(0x5EED_DA7A_0002);
    let mut programs = vec![("points_to".to_string(), points_to()), ("adversarial".to_string(), adversarial_constants())];
    for i in 0..4 {
        programs.push((format!("random {i}"), random_program(&mut rng)));
    }
    programs.extend(refused().into_iter().map(|(n, p)| (n.to_string(), p)));
    programs
}
