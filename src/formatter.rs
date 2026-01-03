use std::fmt::Write;

use crate::ast::{AspectOperator, ModalDomain, QuantifierKind, TemporalOperator, Term, VoiceOperator};
use crate::intern::Interner;
use crate::registry::SymbolRegistry;
use crate::token::TokenType;

pub trait LogicFormatter {
    // Quantifiers
    fn quantifier(&self, kind: &QuantifierKind, var: &str, body: &str) -> String {
        let sym = match kind {
            QuantifierKind::Universal => self.universal(),
            QuantifierKind::Existential => self.existential(),
            QuantifierKind::Most => "MOST ".to_string(),
            QuantifierKind::Few => "FEW ".to_string(),
            QuantifierKind::Many => "MANY ".to_string(),
            QuantifierKind::Cardinal(n) => self.cardinal(*n),
            QuantifierKind::AtLeast(n) => self.at_least(*n),
            QuantifierKind::AtMost(n) => self.at_most(*n),
            QuantifierKind::Generic => "Gen ".to_string(),
        };
        format!("{}{}({})", sym, var, body)
    }

    fn universal(&self) -> String;
    fn existential(&self) -> String;
    fn cardinal(&self, n: u32) -> String;
    fn at_least(&self, n: u32) -> String;
    fn at_most(&self, n: u32) -> String;

    // Binary operators
    fn binary_op(&self, op: &TokenType, left: &str, right: &str) -> String {
        match op {
            TokenType::And => format!("({} {} {})", left, self.and(), right),
            TokenType::Or => format!("({} {} {})", left, self.or(), right),
            TokenType::If => format!("({} {} {})", left, self.implies(), right),
            TokenType::Iff => format!("({} {} {})", left, self.iff(), right),
            _ => String::new(),
        }
    }

    fn and(&self) -> &'static str;
    fn or(&self) -> &'static str;
    fn implies(&self) -> &'static str;
    fn iff(&self) -> &'static str;

    // Unary operators
    fn unary_op(&self, op: &TokenType, operand: &str) -> String {
        match op {
            TokenType::Not => format!("{}{}", self.not(), operand),
            _ => String::new(),
        }
    }

    fn not(&self) -> &'static str;

    // Identity operator (used in Identity expressions)
    fn identity(&self) -> &'static str {
        " = "
    }

    // Whether to wrap identity expressions in parentheses
    fn wrap_identity(&self) -> bool {
        false
    }

    // Modal operators
    fn modal(&self, domain: ModalDomain, force: f32, body: &str) -> String {
        let sym = match domain {
            ModalDomain::Alethic if force > 0.0 && force <= 0.5 => self.possibility(),
            ModalDomain::Alethic => self.necessity(),
            ModalDomain::Deontic if force <= 0.5 => "P",
            ModalDomain::Deontic => "O",
        };
        format!("{}_{{{:.1}}} {}", sym, force, body)
    }

    fn necessity(&self) -> &'static str;
    fn possibility(&self) -> &'static str;

    // Temporal operators
    fn temporal(&self, op: &TemporalOperator, body: &str) -> String {
        let sym = match op {
            TemporalOperator::Past => self.past(),
            TemporalOperator::Future => self.future(),
        };
        format!("{}({})", sym, body)
    }

    fn past(&self) -> &'static str;
    fn future(&self) -> &'static str;

    // Aspectual operators
    fn aspectual(&self, op: &AspectOperator, body: &str) -> String {
        let sym = match op {
            AspectOperator::Progressive => self.progressive(),
            AspectOperator::Perfect => self.perfect(),
            AspectOperator::Habitual => self.habitual(),
            AspectOperator::Iterative => self.iterative(),
        };
        format!("{}({})", sym, body)
    }

    fn progressive(&self) -> &'static str;
    fn perfect(&self) -> &'static str;
    fn habitual(&self) -> &'static str;
    fn iterative(&self) -> &'static str;

    // Voice operators
    fn voice(&self, op: &VoiceOperator, body: &str) -> String {
        let sym = match op {
            VoiceOperator::Passive => self.passive(),
        };
        format!("{}({})", sym, body)
    }

    fn passive(&self) -> &'static str;

    // Lambda
    fn lambda(&self, var: &str, body: &str) -> String;

    // Counterfactual
    fn counterfactual(&self, antecedent: &str, consequent: &str) -> String;

    // Superlative expansion
    fn superlative(&self, comp: &str, domain: &str, subject: &str) -> String;

    // Event quantification (uses existential + and)
    fn event_quantifier(&self, pred: &str, adverbs: &[String]) -> String {
        if adverbs.is_empty() {
            format!("{}e({})", self.existential(), pred)
        } else {
            let conj = self.and();
            format!(
                "{}e({} {} {})",
                self.existential(),
                pred,
                conj,
                adverbs.join(&format!(" {} ", conj))
            )
        }
    }

    // Categorical (legacy)
    fn categorical_all(&self) -> &'static str;
    fn categorical_no(&self) -> &'static str;
    fn categorical_some(&self) -> &'static str;
    fn categorical_not(&self) -> &'static str;

    // Sanitization hook for LaTeX special characters
    fn sanitize(&self, s: &str) -> String {
        s.to_string()
    }

    // Whether to use simple predicate form instead of event semantics
    fn use_simple_events(&self) -> bool {
        false
    }

    // Whether to use full predicate names instead of abbreviations
    fn use_full_names(&self) -> bool {
        false
    }

    // Whether to preserve original case (for code generation)
    fn preserve_case(&self) -> bool {
        false
    }

    // Whether to include world arguments in predicates (for Kripke semantics)
    fn include_world_arguments(&self) -> bool {
        false
    }

    /// Hook for customizing how comparatives are rendered.
    /// Default implementation uses standard logic notation: tallER(subj, obj) or tallER(subj, obj, diff)
    fn write_comparative<W: Write>(
        &self,
        w: &mut W,
        adjective: &str,
        subject: &str,
        object: &str,
        difference: Option<&str>,
    ) -> std::fmt::Result {
        if let Some(diff) = difference {
            write!(w, "{}er({}, {}, {})", adjective, subject, object, diff)
        } else {
            write!(w, "{}er({}, {})", adjective, subject, object)
        }
    }

    /// Hook for customizing how predicates are rendered.
    /// Default implementation uses standard logic notation: Name(Arg1, Arg2)
    fn write_predicate<W: Write>(
        &self,
        w: &mut W,
        name: &str,
        args: &[Term],
        registry: &mut SymbolRegistry,
        interner: &Interner,
    ) -> std::fmt::Result {
        write!(w, "{}(", self.sanitize(name))?;
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                write!(w, ", ")?;
            }
            if self.use_full_names() {
                arg.write_to_full(w, registry, interner)?;
            } else {
                arg.write_to(w, registry, interner)?;
            }
        }
        write!(w, ")")
    }
}

pub struct UnicodeFormatter;

impl LogicFormatter for UnicodeFormatter {
    fn universal(&self) -> String { "∀".to_string() }
    fn existential(&self) -> String { "∃".to_string() }
    fn cardinal(&self, n: u32) -> String { format!("∃={}.", n) }
    fn at_least(&self, n: u32) -> String { format!("∃≥{}", n) }
    fn at_most(&self, n: u32) -> String { format!("∃≤{}", n) }

    fn and(&self) -> &'static str { "∧" }
    fn or(&self) -> &'static str { "∨" }
    fn implies(&self) -> &'static str { "→" }
    fn iff(&self) -> &'static str { "↔" }
    fn not(&self) -> &'static str { "¬" }

    fn necessity(&self) -> &'static str { "□" }
    fn possibility(&self) -> &'static str { "◇" }

    fn past(&self) -> &'static str { "P" }
    fn future(&self) -> &'static str { "F" }

    fn progressive(&self) -> &'static str { "Prog" }
    fn perfect(&self) -> &'static str { "Perf" }
    fn habitual(&self) -> &'static str { "HAB" }
    fn iterative(&self) -> &'static str { "ITER" }
    fn passive(&self) -> &'static str { "Pass" }

    fn lambda(&self, var: &str, body: &str) -> String {
        format!("λ{}.{}", var, body)
    }

    fn counterfactual(&self, antecedent: &str, consequent: &str) -> String {
        format!("({} □→ {})", antecedent, consequent)
    }

    fn superlative(&self, comp: &str, domain: &str, subject: &str) -> String {
        format!(
            "∀x(({}(x) ∧ x ≠ {}) → {}({}, x))",
            domain, subject, comp, subject
        )
    }

    fn categorical_all(&self) -> &'static str { "∀" }
    fn categorical_no(&self) -> &'static str { "∀¬" }
    fn categorical_some(&self) -> &'static str { "∃" }
    fn categorical_not(&self) -> &'static str { "¬" }

    // Use full predicate names (e.g., "Wet" not "W")
    fn use_full_names(&self) -> bool { true }
}

pub struct LatexFormatter;

impl LogicFormatter for LatexFormatter {
    fn universal(&self) -> String { "\\forall ".to_string() }
    fn existential(&self) -> String { "\\exists ".to_string() }
    fn cardinal(&self, n: u32) -> String { format!("\\exists_{{={}}} ", n) }
    fn at_least(&self, n: u32) -> String { format!("\\exists_{{\\geq {}}} ", n) }
    fn at_most(&self, n: u32) -> String { format!("\\exists_{{\\leq {}}} ", n) }

    fn and(&self) -> &'static str { "\\cdot" }
    fn or(&self) -> &'static str { "\\vee" }
    fn implies(&self) -> &'static str { "\\supset" }
    fn iff(&self) -> &'static str { "\\equiv" }
    fn not(&self) -> &'static str { "\\sim " }

    fn necessity(&self) -> &'static str { "\\Box" }
    fn possibility(&self) -> &'static str { "\\Diamond" }

    fn past(&self) -> &'static str { "\\mathsf{P}" }
    fn future(&self) -> &'static str { "\\mathsf{F}" }

    fn progressive(&self) -> &'static str { "\\mathsf{Prog}" }
    fn perfect(&self) -> &'static str { "\\mathsf{Perf}" }
    fn habitual(&self) -> &'static str { "\\mathsf{HAB}" }
    fn iterative(&self) -> &'static str { "\\mathsf{ITER}" }
    fn passive(&self) -> &'static str { "\\mathsf{Pass}" }

    fn lambda(&self, var: &str, body: &str) -> String {
        format!("\\lambda {}.{}", var, body)
    }

    fn counterfactual(&self, antecedent: &str, consequent: &str) -> String {
        format!("({} \\boxright {})", antecedent, consequent)
    }

    fn superlative(&self, comp: &str, domain: &str, subject: &str) -> String {
        format!(
            "\\forall x(({}(x) \\land x \\neq {}) \\supset {}({}, x))",
            domain, subject, comp, subject
        )
    }

    fn categorical_all(&self) -> &'static str { "All" }
    fn categorical_no(&self) -> &'static str { "No" }
    fn categorical_some(&self) -> &'static str { "Some" }
    fn categorical_not(&self) -> &'static str { "not" }

    fn sanitize(&self, s: &str) -> String {
        s.replace('_', r"\_")
            .replace('^', r"\^{}")
            .replace('&', r"\&")
            .replace('%', r"\%")
            .replace('#', r"\#")
            .replace('$', r"\$")
    }
}

pub struct SimpleFOLFormatter;

impl LogicFormatter for SimpleFOLFormatter {
    fn universal(&self) -> String { "∀".to_string() }
    fn existential(&self) -> String { "∃".to_string() }
    fn cardinal(&self, n: u32) -> String { format!("∃={}", n) }
    fn at_least(&self, n: u32) -> String { format!("∃≥{}", n) }
    fn at_most(&self, n: u32) -> String { format!("∃≤{}", n) }

    fn and(&self) -> &'static str { "∧" }
    fn or(&self) -> &'static str { "∨" }
    fn implies(&self) -> &'static str { "→" }
    fn iff(&self) -> &'static str { "↔" }
    fn not(&self) -> &'static str { "¬" }

    fn necessity(&self) -> &'static str { "□" }
    fn possibility(&self) -> &'static str { "◇" }

    fn past(&self) -> &'static str { "Past" }
    fn future(&self) -> &'static str { "Future" }

    fn progressive(&self) -> &'static str { "" }
    fn perfect(&self) -> &'static str { "" }
    fn habitual(&self) -> &'static str { "" }
    fn iterative(&self) -> &'static str { "" }
    fn passive(&self) -> &'static str { "" }

    fn lambda(&self, var: &str, body: &str) -> String {
        format!("λ{}.{}", var, body)
    }

    fn counterfactual(&self, antecedent: &str, consequent: &str) -> String {
        format!("({} □→ {})", antecedent, consequent)
    }

    fn superlative(&self, comp: &str, domain: &str, subject: &str) -> String {
        format!(
            "∀x(({}(x) ∧ x ≠ {}) → {}({}, x))",
            domain, subject, comp, subject
        )
    }

    fn categorical_all(&self) -> &'static str { "∀" }
    fn categorical_no(&self) -> &'static str { "∀¬" }
    fn categorical_some(&self) -> &'static str { "∃" }
    fn categorical_not(&self) -> &'static str { "¬" }

    fn modal(&self, _domain: ModalDomain, _force: f32, body: &str) -> String {
        body.to_string()
    }

    fn aspectual(&self, _op: &AspectOperator, body: &str) -> String {
        body.to_string()
    }

    fn use_simple_events(&self) -> bool {
        true
    }

    fn use_full_names(&self) -> bool {
        true
    }
}

/// Formatter for Kripke lowered output with explicit world arguments.
/// Modals are already lowered to quantifiers; this formatter just renders
/// the result with world arguments appended to predicates.
pub struct KripkeFormatter;

impl LogicFormatter for KripkeFormatter {
    fn universal(&self) -> String { "ForAll ".to_string() }
    fn existential(&self) -> String { "Exists ".to_string() }
    fn cardinal(&self, n: u32) -> String { format!("Exists={} ", n) }
    fn at_least(&self, n: u32) -> String { format!("Exists>={} ", n) }
    fn at_most(&self, n: u32) -> String { format!("Exists<={} ", n) }

    fn and(&self) -> &'static str { " And " }
    fn or(&self) -> &'static str { " Or " }
    fn implies(&self) -> &'static str { " Implies " }
    fn iff(&self) -> &'static str { " Iff " }
    fn not(&self) -> &'static str { "Not " }

    fn necessity(&self) -> &'static str { "Box" }
    fn possibility(&self) -> &'static str { "Diamond" }

    fn past(&self) -> &'static str { "Past" }
    fn future(&self) -> &'static str { "Future" }

    fn progressive(&self) -> &'static str { "Prog" }
    fn perfect(&self) -> &'static str { "Perf" }
    fn habitual(&self) -> &'static str { "HAB" }
    fn iterative(&self) -> &'static str { "ITER" }
    fn passive(&self) -> &'static str { "Pass" }

    fn lambda(&self, var: &str, body: &str) -> String {
        format!("Lambda {}.{}", var, body)
    }

    fn counterfactual(&self, antecedent: &str, consequent: &str) -> String {
        format!("({} Counterfactual {})", antecedent, consequent)
    }

    fn superlative(&self, comp: &str, domain: &str, subject: &str) -> String {
        format!(
            "ForAll x(({}(x) And x != {}) Implies {}({}, x))",
            domain, subject, comp, subject
        )
    }

    fn categorical_all(&self) -> &'static str { "ForAll" }
    fn categorical_no(&self) -> &'static str { "ForAll Not" }
    fn categorical_some(&self) -> &'static str { "Exists" }
    fn categorical_not(&self) -> &'static str { "Not" }

    fn modal(&self, _domain: ModalDomain, _force: f32, body: &str) -> String {
        // Modals already lowered to quantifiers - just pass through
        body.to_string()
    }

    fn use_full_names(&self) -> bool { true }

    fn include_world_arguments(&self) -> bool { true }
}

/// Formatter that produces Rust boolean expressions for runtime assertions.
/// Used by codegen to convert LogicExpr into debug_assert!() compatible code.
pub struct RustFormatter;

impl LogicFormatter for RustFormatter {
    // Operators map to Rust boolean operators
    fn and(&self) -> &'static str { "&&" }
    fn or(&self) -> &'static str { "||" }
    fn not(&self) -> &'static str { "!" }
    fn implies(&self) -> &'static str { "||" } // Handled via binary_op override
    fn iff(&self) -> &'static str { "==" }
    fn identity(&self) -> &'static str { " == " } // Rust equality
    fn wrap_identity(&self) -> bool { true } // Wrap in parens for valid Rust

    // Use full variable names, not abbreviations
    fn use_full_names(&self) -> bool { true }
    fn preserve_case(&self) -> bool { true } // Keep original variable case

    // Quantifiers: runtime can't check universal quantification, emit comments
    fn universal(&self) -> String { "/* ∀ */".to_string() }
    fn existential(&self) -> String { "/* ∃ */".to_string() }
    fn cardinal(&self, n: u32) -> String { format!("/* ∃={} */", n) }
    fn at_least(&self, n: u32) -> String { format!("/* ∃≥{} */", n) }
    fn at_most(&self, n: u32) -> String { format!("/* ∃≤{} */", n) }

    // Modal/Temporal operators are stripped for runtime (not checkable)
    fn necessity(&self) -> &'static str { "" }
    fn possibility(&self) -> &'static str { "" }
    fn past(&self) -> &'static str { "" }
    fn future(&self) -> &'static str { "" }
    fn progressive(&self) -> &'static str { "" }
    fn perfect(&self) -> &'static str { "" }
    fn habitual(&self) -> &'static str { "" }
    fn iterative(&self) -> &'static str { "" }
    fn passive(&self) -> &'static str { "" }
    fn categorical_all(&self) -> &'static str { "" }
    fn categorical_no(&self) -> &'static str { "" }
    fn categorical_some(&self) -> &'static str { "" }
    fn categorical_not(&self) -> &'static str { "" }

    fn lambda(&self, var: &str, body: &str) -> String {
        format!("|{}| {{ {} }}", var, body)
    }

    fn counterfactual(&self, a: &str, c: &str) -> String {
        format!("/* if {} then {} */", a, c)
    }

    fn superlative(&self, _: &str, _: &str, _: &str) -> String {
        "/* superlative */".to_string()
    }

    // Override comparative for Rust: map adjectives to comparison operators
    fn write_comparative<W: Write>(
        &self,
        w: &mut W,
        adjective: &str,
        subject: &str,
        object: &str,
        _difference: Option<&str>,
    ) -> std::fmt::Result {
        let adj_lower = adjective.to_lowercase();
        match adj_lower.as_str() {
            "great" | "big" | "large" | "tall" | "old" | "high" | "more" | "greater" => {
                write!(w, "({} > {})", subject, object)
            }
            "small" | "little" | "short" | "young" | "low" | "less" | "fewer" => {
                write!(w, "({} < {})", subject, object)
            }
            _ => write!(w, "({} > {})", subject, object) // default to greater-than
        }
    }

    // Override unary_op to wrap in parens for valid Rust
    fn unary_op(&self, op: &TokenType, operand: &str) -> String {
        match op {
            TokenType::Not => format!("(!{})", operand),
            _ => format!("/* unknown unary */({})", operand),
        }
    }

    // Override binary_op for implication desugaring: A → B = !A || B
    fn binary_op(&self, op: &TokenType, left: &str, right: &str) -> String {
        match op {
            TokenType::If | TokenType::Then => format!("(!({}) || ({}))", left, right),
            TokenType::And => format!("({} && {})", left, right),
            TokenType::Or => format!("({} || {})", left, right),
            TokenType::Iff => format!("({} == {})", left, right),
            _ => "/* unknown op */".to_string(),
        }
    }

    // Core predicate mapping: semantic interpretation of predicates to Rust operators
    fn write_predicate<W: Write>(
        &self,
        w: &mut W,
        name: &str,
        args: &[Term],
        _registry: &mut SymbolRegistry,
        interner: &Interner,
    ) -> std::fmt::Result {
        // Helper to render a term at given index to a string, preserving original case
        let render = |idx: usize| -> String {
            let mut buf = String::new();
            if let Some(arg) = args.get(idx) {
                let _ = arg.write_to_raw(&mut buf, interner);
            }
            buf
        };

        match name.to_lowercase().as_str() {
            // Comparisons
            "greater" if args.len() == 2 => write!(w, "({} > {})", render(0), render(1)),
            "less" if args.len() == 2 => write!(w, "({} < {})", render(0), render(1)),
            "equal" | "equals" if args.len() == 2 => write!(w, "({} == {})", render(0), render(1)),
            "notequal" | "not_equal" if args.len() == 2 => write!(w, "({} != {})", render(0), render(1)),
            "greaterequal" | "atleast" | "at_least" if args.len() == 2 => write!(w, "({} >= {})", render(0), render(1)),
            "lessequal" | "atmost" | "at_most" if args.len() == 2 => write!(w, "({} <= {})", render(0), render(1)),

            // Unary checks
            "positive" if args.len() == 1 => write!(w, "({} > 0)", render(0)),
            "negative" if args.len() == 1 => write!(w, "({} < 0)", render(0)),
            "zero" if args.len() == 1 => write!(w, "({} == 0)", render(0)),
            "empty" if args.len() == 1 => write!(w, "{}.is_empty()", render(0)),

            // Collection membership
            "in" if args.len() == 2 => write!(w, "{}.contains(&{})", render(1), render(0)),
            "contains" if args.len() == 2 => write!(w, "{}.contains(&{})", render(0), render(1)),

            // Fallback: method call for 1 arg, function call for N args
            _ if args.len() == 1 => write!(w, "{}.is_{}()", render(0), name.to_lowercase()),
            _ => {
                write!(w, "{}(", name.to_lowercase())?;
                for i in 0..args.len() {
                    if i > 0 { write!(w, ", ")?; }
                    write!(w, "{}", render(i))?;
                }
                write!(w, ")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_binary_operators() {
        let f = UnicodeFormatter;
        assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), "(P ∧ Q)");
        assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), "(P ∨ Q)");
        assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), "(P → Q)");
        assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), "(P ↔ Q)");
    }

    #[test]
    fn latex_binary_operators() {
        let f = LatexFormatter;
        assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), r"(P \cdot Q)");
        assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), r"(P \vee Q)");
        assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), r"(P \supset Q)");
        assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), r"(P \equiv Q)");
    }

    #[test]
    fn unicode_quantifiers() {
        let f = UnicodeFormatter;
        assert_eq!(f.quantifier(&QuantifierKind::Universal, "x", "P(x)"), "∀x(P(x))");
        assert_eq!(f.quantifier(&QuantifierKind::Existential, "x", "P(x)"), "∃x(P(x))");
        assert_eq!(f.quantifier(&QuantifierKind::Cardinal(3), "x", "P(x)"), "∃=3.x(P(x))");
    }

    #[test]
    fn latex_quantifiers() {
        let f = LatexFormatter;
        assert_eq!(f.quantifier(&QuantifierKind::Universal, "x", "P(x)"), "\\forall x(P(x))");
        assert_eq!(f.quantifier(&QuantifierKind::Existential, "x", "P(x)"), "\\exists x(P(x))");
    }

    #[test]
    fn latex_sanitization() {
        let f = LatexFormatter;
        assert_eq!(f.sanitize("foo_bar"), r"foo\_bar");
        assert_eq!(f.sanitize("x^2"), r"x\^{}2");
        assert_eq!(f.sanitize("a&b"), r"a\&b");
    }

    #[test]
    fn unicode_no_sanitization() {
        let f = UnicodeFormatter;
        assert_eq!(f.sanitize("foo_bar"), "foo_bar");
    }

    #[test]
    fn unicode_lambda() {
        let f = UnicodeFormatter;
        assert_eq!(f.lambda("x", "P(x)"), "λx.P(x)");
    }

    #[test]
    fn latex_lambda() {
        let f = LatexFormatter;
        assert_eq!(f.lambda("x", "P(x)"), "\\lambda x.P(x)");
    }

    #[test]
    fn unicode_counterfactual() {
        let f = UnicodeFormatter;
        assert_eq!(f.counterfactual("P", "Q"), "(P □→ Q)");
    }

    #[test]
    fn latex_counterfactual() {
        let f = LatexFormatter;
        assert_eq!(f.counterfactual("P", "Q"), r"(P \boxright Q)");
    }

    // RustFormatter tests
    #[test]
    fn rust_binary_operators() {
        let f = RustFormatter;
        assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), "(P && Q)");
        assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), "(P || Q)");
        assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), "(P == Q)");
    }

    #[test]
    fn rust_implication_desugaring() {
        let f = RustFormatter;
        // A → B desugars to !A || B
        assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), "(!(P) || (Q))");
    }

    #[test]
    fn rust_lambda() {
        let f = RustFormatter;
        assert_eq!(f.lambda("x", "x > 0"), "|x| { x > 0 }");
    }

    #[test]
    fn rust_quantifiers_as_comments() {
        let f = RustFormatter;
        assert_eq!(f.universal(), "/* ∀ */");
        assert_eq!(f.existential(), "/* ∃ */");
    }
}
