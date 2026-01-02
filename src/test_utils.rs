use crate::ast::{AspectOperator, ModalDomain, ModalVector, QuantifierKind, TemporalOperator};
use crate::token::{FocusKind, TokenType};
use crate::view::{ExprView, NounPhraseView, TermView};
use crate::lexicon::Definiteness;

#[macro_export]
macro_rules! assert_snapshot {
    ($name:expr, $actual:expr) => {{
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set");
        let snapshot_dir = std::path::Path::new(&manifest_dir)
            .join("tests")
            .join("snapshots");
        let snapshot_path = snapshot_dir.join(format!("{}.txt", $name));

        if !snapshot_dir.exists() {
            std::fs::create_dir_all(&snapshot_dir).expect("Failed to create snapshot dir");
        }

        let actual_str = $actual.trim();
        let force_update = std::env::var("UPDATE_SNAPSHOTS").is_ok();

        if force_update || !snapshot_path.exists() {
            std::fs::write(&snapshot_path, actual_str).expect("Failed to write snapshot");
            println!("Snapshot created/updated: {:?}", snapshot_path);
        } else {
            let expected = std::fs::read_to_string(&snapshot_path)
                .expect("Failed to read snapshot");
            let expected_str = expected.trim();

            if actual_str != expected_str {
                panic!(
                    "\nSnapshot Mismatch: {}\n\nExpected:\n{}\n\nActual:\n{}\n\n\
                    Run `UPDATE_SNAPSHOTS=1 cargo test` to update.\n",
                    $name, expected_str, actual_str
                );
            }
        }
    }};
}

#[macro_export]
macro_rules! parse {
    ($input:expr) => {{
        use $crate::{Arena, AstContext, LogicExpr, Interner, Lexer, NounPhrase, Parser, Resolve, Symbol, Term, ThematicRole};

        let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
        let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
        let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
        let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
        let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
        let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
        let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));

        let ctx = AstContext::new(
            expr_arena,
            term_arena,
            np_arena,
            sym_arena,
            role_arena,
            pp_arena,
        );

        let mut lexer = Lexer::new($input, interner);
        let tokens = lexer.tokenize();

        let mut parser = Parser::new(tokens, interner, ctx);

        let ast = parser.parse().unwrap();
        ast.resolve(interner)
    }};
}

pub mod dsl {
    use super::*;

    fn b<T>(t: T) -> Box<T> {
        Box::new(t)
    }

    // === Terms ===
    pub fn c(name: &'static str) -> TermView<'static> {
        TermView::Constant(name)
    }

    pub fn v(name: &'static str) -> TermView<'static> {
        TermView::Variable(name)
    }

    pub fn func(name: &'static str, args: Vec<TermView<'static>>) -> TermView<'static> {
        TermView::Function(name, args)
    }

    pub fn group(members: Vec<TermView<'static>>) -> TermView<'static> {
        TermView::Group(members)
    }

    pub fn possessed(possessor: TermView<'static>, possessed: &'static str) -> TermView<'static> {
        TermView::Possessed {
            possessor: b(possessor),
            possessed,
        }
    }

    // === Atoms & Predicates ===
    pub fn atom(s: &'static str) -> ExprView<'static> {
        ExprView::Atom(s)
    }

    pub fn pred(name: &'static str, args: Vec<TermView<'static>>) -> ExprView<'static> {
        ExprView::Predicate { name, args }
    }

    pub fn pred1(name: &'static str, arg: &'static str) -> ExprView<'static> {
        pred(name, vec![c(arg)])
    }

    pub fn pred1v(name: &'static str, var: &'static str) -> ExprView<'static> {
        pred(name, vec![v(var)])
    }

    pub fn pred2(name: &'static str, a1: &'static str, a2: &'static str) -> ExprView<'static> {
        pred(name, vec![c(a1), c(a2)])
    }

    pub fn pred2v(name: &'static str, v1: &'static str, v2: &'static str) -> ExprView<'static> {
        pred(name, vec![v(v1), v(v2)])
    }

    // === Identity ===
    pub fn identity(left: TermView<'static>, right: TermView<'static>) -> ExprView<'static> {
        ExprView::Identity { left, right }
    }

    // === Quantifiers ===
    pub fn forall(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::Universal,
            variable: var,
            body: b(body),
        }
    }

    pub fn exists(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::Existential,
            variable: var,
            body: b(body),
        }
    }

    pub fn most(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::Most,
            variable: var,
            body: b(body),
        }
    }

    pub fn few(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::Few,
            variable: var,
            body: b(body),
        }
    }

    pub fn cardinal(n: u32, var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::Cardinal(n),
            variable: var,
            body: b(body),
        }
    }

    pub fn at_least(n: u32, var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::AtLeast(n),
            variable: var,
            body: b(body),
        }
    }

    pub fn at_most(n: u32, var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Quantifier {
            kind: QuantifierKind::AtMost(n),
            variable: var,
            body: b(body),
        }
    }

    // === Temporal ===
    pub fn past(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Temporal {
            operator: TemporalOperator::Past,
            body: b(body),
        }
    }

    pub fn future(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Temporal {
            operator: TemporalOperator::Future,
            body: b(body),
        }
    }

    // === Aspect ===
    pub fn prog(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Aspectual {
            operator: AspectOperator::Progressive,
            body: b(body),
        }
    }

    pub fn perf(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Aspectual {
            operator: AspectOperator::Perfect,
            body: b(body),
        }
    }

    // === Modal ===
    pub fn necessity(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Modal {
            vector: ModalVector {
                domain: ModalDomain::Alethic,
                force: 1.0,
                flavor: crate::ast::ModalFlavor::Root,
            },
            operand: b(body),
        }
    }

    pub fn possibility(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Modal {
            vector: ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.0,
                flavor: crate::ast::ModalFlavor::Root,
            },
            operand: b(body),
        }
    }

    pub fn obligation(force: f32, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Modal {
            vector: ModalVector {
                domain: ModalDomain::Deontic,
                force,
                flavor: crate::ast::ModalFlavor::Root,
            },
            operand: b(body),
        }
    }

    pub fn modal(domain: ModalDomain, force: f32, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Modal {
            vector: ModalVector { domain, force, flavor: crate::ast::ModalFlavor::Root },
            operand: b(body),
        }
    }

    // === Binary Ops ===
    pub fn and(left: ExprView<'static>, right: ExprView<'static>) -> ExprView<'static> {
        ExprView::BinaryOp {
            left: b(left),
            op: TokenType::And,
            right: b(right),
        }
    }

    pub fn or(left: ExprView<'static>, right: ExprView<'static>) -> ExprView<'static> {
        ExprView::BinaryOp {
            left: b(left),
            op: TokenType::Or,
            right: b(right),
        }
    }

    pub fn implies(left: ExprView<'static>, right: ExprView<'static>) -> ExprView<'static> {
        ExprView::BinaryOp {
            left: b(left),
            op: TokenType::If,
            right: b(right),
        }
    }

    pub fn iff(left: ExprView<'static>, right: ExprView<'static>) -> ExprView<'static> {
        ExprView::BinaryOp {
            left: b(left),
            op: TokenType::Iff,
            right: b(right),
        }
    }

    // === Unary Ops ===
    pub fn not(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::UnaryOp {
            op: TokenType::Not,
            operand: b(body),
        }
    }

    // === Lambda & Application ===
    pub fn lambda(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Lambda {
            variable: var,
            body: b(body),
        }
    }

    pub fn app(func: ExprView<'static>, arg: ExprView<'static>) -> ExprView<'static> {
        ExprView::App {
            function: b(func),
            argument: b(arg),
        }
    }

    // === Questions ===
    pub fn wh_question(var: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Question {
            wh_variable: var,
            body: b(body),
        }
    }

    pub fn yes_no_question(body: ExprView<'static>) -> ExprView<'static> {
        ExprView::YesNoQuestion { body: b(body) }
    }

    // === Intensional ===
    pub fn intensional(op: &'static str, content: ExprView<'static>) -> ExprView<'static> {
        ExprView::Intensional {
            operator: op,
            content: b(content),
        }
    }

    // === Event ===
    pub fn event(pred: ExprView<'static>, adverbs: Vec<&'static str>) -> ExprView<'static> {
        ExprView::Event {
            predicate: b(pred),
            adverbs,
        }
    }

    // === Imperative ===
    pub fn imperative(action: ExprView<'static>) -> ExprView<'static> {
        ExprView::Imperative { action: b(action) }
    }

    // === Speech Act ===
    pub fn speech_act(
        performer: &'static str,
        act_type: &'static str,
        content: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::SpeechAct {
            performer,
            act_type,
            content: b(content),
        }
    }

    // === Counterfactual ===
    pub fn counterfactual(
        antecedent: ExprView<'static>,
        consequent: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Counterfactual {
            antecedent: b(antecedent),
            consequent: b(consequent),
        }
    }

    // === Comparative & Superlative ===
    pub fn comparative(
        adj: &'static str,
        subject: TermView<'static>,
        object: TermView<'static>,
    ) -> ExprView<'static> {
        ExprView::Comparative {
            adjective: adj,
            subject,
            object,
            difference: None,
        }
    }

    pub fn superlative(
        adj: &'static str,
        subject: TermView<'static>,
        domain: &'static str,
    ) -> ExprView<'static> {
        ExprView::Superlative {
            adjective: adj,
            subject,
            domain,
        }
    }

    // === Scopal ===
    pub fn scopal(op: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::Scopal {
            operator: op,
            body: b(body),
        }
    }

    // === Control ===
    pub fn control(
        verb: &'static str,
        subject: TermView<'static>,
        object: Option<TermView<'static>>,
        infinitive: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Control {
            verb,
            subject,
            object,
            infinitive: b(infinitive),
        }
    }

    // === Presupposition ===
    pub fn presupposition(
        assertion: ExprView<'static>,
        presup: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Presupposition {
            assertion: b(assertion),
            presupposition: b(presup),
        }
    }

    // === Focus ===
    pub fn focus_only(
        focused: TermView<'static>,
        scope: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Focus {
            kind: FocusKind::Only,
            focused,
            scope: b(scope),
        }
    }

    pub fn focus_even(
        focused: TermView<'static>,
        scope: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Focus {
            kind: FocusKind::Even,
            focused,
            scope: b(scope),
        }
    }

    pub fn focus_just(
        focused: TermView<'static>,
        scope: ExprView<'static>,
    ) -> ExprView<'static> {
        ExprView::Focus {
            kind: FocusKind::Just,
            focused,
            scope: b(scope),
        }
    }

    // === Temporal Anchor ===
    pub fn temporal_anchor(anchor: &'static str, body: ExprView<'static>) -> ExprView<'static> {
        ExprView::TemporalAnchor {
            anchor,
            body: b(body),
        }
    }

    // === Categorical (legacy support) ===
    pub fn categorical(
        quantifier: TokenType,
        subject: NounPhraseView<'static>,
        copula_negative: bool,
        predicate: NounPhraseView<'static>,
    ) -> ExprView<'static> {
        ExprView::Categorical {
            quantifier,
            subject,
            copula_negative,
            predicate,
        }
    }

    // === Relation (legacy support) ===
    pub fn relation(
        subject: NounPhraseView<'static>,
        verb: &'static str,
        object: NounPhraseView<'static>,
    ) -> ExprView<'static> {
        ExprView::Relation {
            subject,
            verb,
            object,
        }
    }

    // === NounPhrase builders ===
    pub fn np(noun: &'static str) -> NounPhraseView<'static> {
        NounPhraseView {
            definiteness: None,
            adjectives: vec![],
            noun,
            possessor: None,
            pps: vec![],
            superlative: None,
        }
    }

    pub fn np_def(definiteness: Definiteness, noun: &'static str) -> NounPhraseView<'static> {
        NounPhraseView {
            definiteness: Some(definiteness),
            adjectives: vec![],
            noun,
            possessor: None,
            pps: vec![],
            superlative: None,
        }
    }

    pub fn np_adj(
        adjectives: Vec<&'static str>,
        noun: &'static str,
    ) -> NounPhraseView<'static> {
        NounPhraseView {
            definiteness: None,
            adjectives,
            noun,
            possessor: None,
            pps: vec![],
            superlative: None,
        }
    }

    pub fn np_full(
        definiteness: Option<Definiteness>,
        adjectives: Vec<&'static str>,
        noun: &'static str,
        possessor: Option<NounPhraseView<'static>>,
    ) -> NounPhraseView<'static> {
        NounPhraseView {
            definiteness,
            adjectives,
            noun,
            possessor: possessor.map(Box::new),
            pps: vec![],
            superlative: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dsl::*;
    use crate::ast::{ModalDomain, QuantifierKind, TemporalOperator};
    use crate::token::TokenType;
    use crate::view::{ExprView, TermView};

    #[test]
    fn dsl_constant_term() {
        let term = c("John");
        assert_eq!(term, TermView::Constant("John"));
    }

    #[test]
    fn dsl_variable_term() {
        let term = v("x");
        assert_eq!(term, TermView::Variable("x"));
    }

    #[test]
    fn dsl_atom() {
        let expr = atom("P");
        assert_eq!(expr, ExprView::Atom("P"));
    }

    #[test]
    fn dsl_pred1() {
        let expr = pred1("Run", "John");
        assert_eq!(
            expr,
            ExprView::Predicate {
                name: "Run",
                args: vec![TermView::Constant("John")],
            }
        );
    }

    #[test]
    fn dsl_pred2() {
        let expr = pred2("Love", "John", "Mary");
        assert_eq!(
            expr,
            ExprView::Predicate {
                name: "Love",
                args: vec![
                    TermView::Constant("John"),
                    TermView::Constant("Mary")
                ],
            }
        );
    }

    #[test]
    fn dsl_forall() {
        let expr = forall("x", pred1v("P", "x"));
        assert_eq!(
            expr,
            ExprView::Quantifier {
                kind: QuantifierKind::Universal,
                variable: "x",
                body: Box::new(ExprView::Predicate {
                    name: "P",
                    args: vec![TermView::Variable("x")],
                }),
            }
        );
    }

    #[test]
    fn dsl_exists() {
        let expr = exists("x", pred1v("P", "x"));
        assert_eq!(
            expr,
            ExprView::Quantifier {
                kind: QuantifierKind::Existential,
                variable: "x",
                body: Box::new(ExprView::Predicate {
                    name: "P",
                    args: vec![TermView::Variable("x")],
                }),
            }
        );
    }

    #[test]
    fn dsl_past() {
        let expr = past(pred1("Run", "John"));
        assert_eq!(
            expr,
            ExprView::Temporal {
                operator: TemporalOperator::Past,
                body: Box::new(ExprView::Predicate {
                    name: "Run",
                    args: vec![TermView::Constant("John")],
                }),
            }
        );
    }

    #[test]
    fn dsl_and() {
        let expr = and(atom("P"), atom("Q"));
        assert_eq!(
            expr,
            ExprView::BinaryOp {
                left: Box::new(ExprView::Atom("P")),
                op: TokenType::And,
                right: Box::new(ExprView::Atom("Q")),
            }
        );
    }

    #[test]
    fn dsl_implies() {
        let expr = implies(atom("P"), atom("Q"));
        assert_eq!(
            expr,
            ExprView::BinaryOp {
                left: Box::new(ExprView::Atom("P")),
                op: TokenType::If,
                right: Box::new(ExprView::Atom("Q")),
            }
        );
    }

    #[test]
    fn dsl_not() {
        let expr = not(atom("P"));
        assert_eq!(
            expr,
            ExprView::UnaryOp {
                op: TokenType::Not,
                operand: Box::new(ExprView::Atom("P")),
            }
        );
    }

    #[test]
    fn dsl_lambda() {
        let expr = lambda("x", pred1v("P", "x"));
        assert_eq!(
            expr,
            ExprView::Lambda {
                variable: "x",
                body: Box::new(ExprView::Predicate {
                    name: "P",
                    args: vec![TermView::Variable("x")],
                }),
            }
        );
    }

    #[test]
    fn dsl_modal_necessity() {
        let expr = necessity(atom("Rain"));
        if let ExprView::Modal { vector, operand } = expr {
            assert_eq!(vector.domain, ModalDomain::Alethic);
            assert_eq!(vector.force, 1.0);
            assert_eq!(*operand, ExprView::Atom("Rain"));
        } else {
            panic!("Expected Modal");
        }
    }

    #[test]
    fn dsl_complex_nested() {
        let expr = forall(
            "x",
            implies(pred1v("Human", "x"), pred1v("Mortal", "x")),
        );

        assert_eq!(
            expr,
            ExprView::Quantifier {
                kind: QuantifierKind::Universal,
                variable: "x",
                body: Box::new(ExprView::BinaryOp {
                    left: Box::new(ExprView::Predicate {
                        name: "Human",
                        args: vec![TermView::Variable("x")],
                    }),
                    op: TokenType::If,
                    right: Box::new(ExprView::Predicate {
                        name: "Mortal",
                        args: vec![TermView::Variable("x")],
                    }),
                }),
            }
        );
    }

    #[test]
    fn dsl_box_is_hidden() {
        let expr = past(pred1("Run", "John"));
        if let ExprView::Temporal { body, .. } = expr {
            assert!(matches!(*body, ExprView::Predicate { .. }));
        }
    }

    #[test]
    fn parse_macro_returns_static_view() {
        let view = crate::parse!("John ran.");
        assert!(
            matches!(view, ExprView::NeoEvent { .. }) || matches!(view, ExprView::Temporal { .. }),
            "Expected NeoEvent or Temporal, got {:?}", view
        );
    }

    #[test]
    fn snapshot_macro_creates_file() {
        let output = "test output";
        crate::assert_snapshot!("test_snapshot_macro", output);
    }
}
