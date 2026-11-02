//! Every quantifying determiner reads the same way in every construction it heads. The determiner
//! fixes two things — the quantifier it binds and how its restrictor meets its scope: `R → S` under
//! `all` and `all but finitely many`, `R → ¬S` under `no`, `R ∧ S` under every other determiner —
//! and the construction fixes the rest. So whether the verb phrase is plain, carries an auxiliary,
//! a negated auxiliary, a root or epistemic modal, a presupposition trigger, a quantified object,
//! do-support, a copula, an adverb or a relative clause, the reading under any determiner is the
//! reading under `some` (or `all`) with only the quantifier changed.

use logicaffeine_language::ast::{LogicExpr, QuantifierKind};
use logicaffeine_language::token::TokenType;
use logicaffeine_language::visitor::{walk_expr, Visitor};
use logicaffeine_language::{compile, compile_kripke_with, Interner};

/// How a quantifier's body joins its restrictor to its scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    /// `R → S`
    Implies,
    /// `R → ¬S`
    ImpliesNot,
    /// `R ∧ S`
    And,
}

/// The determiners, each with the quantifier it binds, its join, and the symbol the Unicode output
/// writes for it in place of `some`'s `∃` (or `all`'s `∀`).
const DETERMINERS: [(&str, QuantifierKind, Join, &str); 13] = [
    ("All", QuantifierKind::Universal, Join::Implies, "∀"),
    ("Some", QuantifierKind::Existential, Join::And, "∃"),
    ("No", QuantifierKind::Universal, Join::ImpliesNot, "∀"),
    ("Most", QuantifierKind::Most, Join::And, "MOST "),
    ("Few", QuantifierKind::Few, Join::And, "FEW "),
    ("Many", QuantifierKind::Many, Join::And, "MANY "),
    ("Three", QuantifierKind::Cardinal(3), Join::And, "∃=3."),
    (
        "At least three",
        QuantifierKind::AtLeast(3),
        Join::And,
        "∃≥3",
    ),
    ("At most three", QuantifierKind::AtMost(3), Join::And, "∃≤3"),
    (
        "Infinitely many",
        QuantifierKind::InfinitelyMany,
        Join::And,
        "∃^∞",
    ),
    (
        "Finitely many",
        QuantifierKind::FinitelyMany,
        Join::And,
        "∃^<∞",
    ),
    (
        "Only finitely many",
        QuantifierKind::FinitelyMany,
        Join::And,
        "∃^<∞",
    ),
    (
        "All but finitely many",
        QuantifierKind::AllButFinitelyMany,
        Join::Implies,
        "∀^∞",
    ),
];

/// The constructions a determiner heads, each completing `<Determiner> `.
const CONSTRUCTIONS: [&str; 11] = [
    "dogs bark.",
    "dogs will bark.",
    "dogs did not bark.",
    "dogs can bark.",
    "dogs might bark.",
    "dogs stopped barking.",
    "dogs chase a cat.",
    "dogs do not bark.",
    "dogs are happy.",
    "dogs bark loudly.",
    "dogs that bark bite.",
];

/// The quantifier binding `x` nearest the root, with its body's connective and whether the
/// connective's right side is a negation.
struct SubjectBinding<'i> {
    interner: &'i Interner,
    found: Option<(QuantifierKind, Option<TokenType>, bool)>,
}

impl<'a, 'i> Visitor<'a> for SubjectBinding<'i> {
    fn visit_expr(&mut self, expr: &'a LogicExpr<'a>) {
        if self.found.is_some() {
            return;
        }
        if let LogicExpr::Quantifier {
            kind,
            variable,
            body,
            ..
        } = expr
        {
            if self.interner.resolve(*variable) == "x" {
                self.found = Some(match body {
                    LogicExpr::BinaryOp { op, right, .. } => (
                        *kind,
                        Some(op.clone()),
                        matches!(
                            right,
                            LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                ..
                            }
                        ),
                    ),
                    _ => (*kind, None, false),
                });
                return;
            }
        }
        walk_expr(self, expr);
    }
}

fn subject_binding(sentence: &str) -> Result<(QuantifierKind, Option<TokenType>, bool), String> {
    compile_kripke_with(sentence, |expr, interner| {
        let mut binding = SubjectBinding {
            interner,
            found: None,
        };
        binding.visit_expr(expr);
        binding.found
    })
    .map_err(|e| format!("does not parse: {e:?}"))?
    .ok_or_else(|| "binds no x".to_string())
}

fn join_holds(join: Join, op: &Option<TokenType>, negated_scope: bool) -> bool {
    match join {
        Join::Implies => matches!(op, Some(TokenType::Implies)),
        Join::ImpliesNot => matches!(op, Some(TokenType::Implies)) && negated_scope,
        Join::And => matches!(op, Some(TokenType::And)),
    }
}

#[test]
fn every_determiner_binds_its_own_quantifier_and_join_in_every_construction() {
    let mut wrong = Vec::new();
    for (words, kind, join, _) in DETERMINERS {
        for construction in CONSTRUCTIONS {
            let sentence = format!("{words} {construction}");
            match subject_binding(&sentence) {
                Ok((found, op, negated)) => {
                    if found != kind || !join_holds(join, &op, negated) {
                        wrong.push(format!(
                            "{sentence:40} binds {found:?} joined by {op:?} (negated scope: {negated}); \
                             expected {kind:?} joined {join:?}"
                        ));
                    }
                }
                Err(e) => wrong.push(format!("{sentence:40} {e}")),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} readings disagree with their determiner:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The reading under `determiner` predicted from the reading under `some` (conjunctive
/// determiners) or `all` (implicative ones): the subject quantifier's symbol replaced, and under
/// `no` the scope negated.
fn predicted(words: &str, symbol: &str, join: Join, construction: &str) -> String {
    match join {
        Join::And => compile(&format!("Some {construction}"))
            .expect("the `some` reading parses")
            .replacen("∃x(", &format!("{symbol}x("), 1),
        Join::Implies => compile(&format!("All {construction}"))
            .expect("the `all` reading parses")
            .replacen("∀x(", &format!("{symbol}x("), 1),
        Join::ImpliesNot => {
            assert_eq!(words, "No");
            compile(&format!("All {construction}"))
                .expect("the `all` reading parses")
                .replacen("→ ", "→ ¬", 1)
        }
    }
}

#[test]
fn a_determiner_changes_only_the_quantifier_of_the_reading() {
    let mut wrong = Vec::new();
    for (words, _, join, symbol) in DETERMINERS {
        for construction in CONSTRUCTIONS {
            let sentence = format!("{words} {construction}");
            let expected = predicted(words, symbol, join, construction);
            match compile(&sentence) {
                Ok(found) if found == expected => {}
                Ok(found) => wrong.push(format!(
                    "{sentence}\n    found    {found}\n    expected {expected}"
                )),
                Err(e) => wrong.push(format!("{sentence}\n    does not parse: {e:?}")),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} readings differ from their determiner's:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Object and agent positions: each template with the determiner's slot, the variable the object
/// binds, and the object's restrictor predicate.
const OBJECT_CONSTRUCTIONS: [(&str, &str, &str); 5] = [
    ("Every cat chases {} dogs.", "y", "Dogs"),
    ("Every cat will chase {} dogs.", "y", "Dogs"),
    ("Every cat saw {} dogs.", "y", "Dogs"),
    ("Every book was read by {} students.", "y", "Students"),
    ("John chases {} dogs.", "x", "Dogs"),
];

#[test]
fn a_determiner_in_object_or_agent_position_changes_only_its_quantifier() {
    let mut wrong = Vec::new();
    for (words, _, join, symbol) in DETERMINERS {
        let words = words.to_lowercase();
        for (template, var, restrictor) in OBJECT_CONSTRUCTIONS {
            let sentence = template.replace("{}", &words);
            let expected = match join {
                Join::And => compile(&template.replace("{}", "some"))
                    .expect("the `some` reading parses")
                    .replacen(&format!("∃{var}("), &format!("{symbol}{var}("), 1),
                Join::Implies => compile(&template.replace("{}", "all"))
                    .expect("the `all` reading parses")
                    .replacen(&format!("∀{var}("), &format!("{symbol}{var}("), 1),
                Join::ImpliesNot => {
                    let restricted = format!("∀{var}(({restrictor}({var}) → ");
                    compile(&template.replace("{}", "all"))
                        .expect("the `all` reading parses")
                        .replacen(&restricted, &format!("{restricted}¬"), 1)
                }
            };
            match compile(&sentence) {
                Ok(found) if found == expected => {}
                Ok(found) => wrong.push(format!(
                    "{sentence}\n    found    {found}\n    expected {expected}"
                )),
                Err(e) => wrong.push(format!("{sentence}\n    does not parse: {e:?}")),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} object readings differ from their determiner's:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_passive_agent_keeps_its_determiner() {
    assert_eq!(
        compile("Every book was read by no students.").unwrap(),
        "∀x((Book(x) → ∀y((Students(y) → ¬∃e(Read(e) ∧ Theme(e, x) ∧ Agent(e, y) ∧ Past(e))))))"
    );
    assert_eq!(
        compile("Every book was read by three students.").unwrap(),
        "∀x((Book(x) → ∃=3.y((Students(y) ∧ ∃e(Read(e) ∧ Theme(e, x) ∧ Agent(e, y) ∧ Past(e))))))"
    );
}

#[test]
fn many_under_an_auxiliary_is_many() {
    assert_eq!(
        compile("Many dogs will bark.").unwrap(),
        "MANY x((Dogs(x) ∧ ∃e(Bark(e) ∧ Agent(e, x) ∧ Future(e))))"
    );
    assert_eq!(
        compile("Many dogs did not bark.").unwrap(),
        "MANY x((Dogs(x) ∧ ¬∃e(Bark(e) ∧ Agent(e, x) ∧ Past(e))))"
    );
}

#[test]
fn no_over_a_presupposition_trigger_negates_its_scope() {
    assert_eq!(
        compile("No dog stopped barking.").unwrap(),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ¬¬∃e(Bark(e) ∧ Agent(e, x))))"
    );
}

#[test]
fn any_with_a_quantified_object_restricts_by_implication() {
    assert_eq!(
        compile("Any dog chases a cat.").unwrap(),
        compile("Every dog chases a cat.").unwrap()
    );
    assert_eq!(
        compile("Any dog chases a cat.").unwrap(),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ∃y((((Cat(y) ∧ Animal(y)) ∧ Mammal(y)) ∧ ∃e(Chase(e) ∧ Agent(e, x) ∧ Theme(e, y))))))"
    );
}

#[test]
fn any_under_negation_is_existential_with_an_auxiliary_too() {
    assert_eq!(
        compile("Not any dogs will run.").unwrap(),
        "¬∃x((Dogs(x) ∧ ∃e(Run(e) ∧ Agent(e, x) ∧ Future(e))))"
    );
    assert_eq!(
        compile("Not any dogs run.").unwrap(),
        "¬∃x((Dogs(x) ∧ ∃e(Run(e) ∧ Agent(e, x))))"
    );
}

#[test]
fn do_support_keeps_the_determiners_quantifier() {
    assert_eq!(
        compile("No dog does bark.").unwrap(),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ¬Bark(x)))"
    );
    assert_eq!(
        compile("Most dogs do not bark.").unwrap(),
        "MOST x((Dogs(x) ∧ ¬Bark(x)))"
    );
    assert_eq!(
        compile("Three dogs do not bark.").unwrap(),
        "∃=3.x((Dogs(x) ∧ ¬Bark(x)))"
    );
    assert_eq!(
        compile("Every dog does not bark.").unwrap(),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ¬Bark(x)))"
    );
}

#[test]
fn any_reads_universally_outside_negation_in_every_construction() {
    for (any, every) in [
        ("Any dog barks.", "Every dog barks."),
        ("Any dog will bark.", "Every dog will bark."),
        ("Any dog can bark.", "Every dog can bark."),
        ("Any dog stopped barking.", "Every dog stopped barking."),
        ("Any dog chases a cat.", "Every dog chases a cat."),
        ("Any dog is happy.", "Every dog is happy."),
    ] {
        assert_eq!(
            compile(any).unwrap(),
            compile(every).unwrap(),
            "{any} reads as {every}"
        );
    }
}
