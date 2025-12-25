use crate::ast::{AspectOperator, ModalDomain, QuantifierKind, TemporalOperator, VoiceOperator};
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
}
