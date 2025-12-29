use std::fmt::Write;

use crate::ast::{LogicExpr, NounPhrase, Term};
use crate::ast::logic::NumberKind;
use crate::formatter::{LatexFormatter, LogicFormatter, SimpleFOLFormatter, UnicodeFormatter};
use crate::intern::Interner;
use crate::registry::SymbolRegistry;
use crate::token::TokenType;
use crate::{OutputFormat, TranspileContext};

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn write_capitalized<W: Write>(w: &mut W, s: &str) -> std::fmt::Result {
    let mut chars = s.chars();
    match chars.next() {
        None => Ok(()),
        Some(c) => {
            for uc in c.to_uppercase() {
                write!(w, "{}", uc)?;
            }
            write!(w, "{}", chars.as_str())
        }
    }
}

impl<'a> NounPhrase<'a> {
    pub fn to_symbol(&self, registry: &mut SymbolRegistry, interner: &Interner) -> String {
        registry.get_symbol(self.noun, interner)
    }
}

impl<'a> Term<'a> {
    pub fn write_to<W: Write>(
        &self,
        w: &mut W,
        registry: &mut SymbolRegistry,
        interner: &Interner,
    ) -> std::fmt::Result {
        self.write_to_inner(w, registry, interner, false)
    }

    pub fn write_to_full<W: Write>(
        &self,
        w: &mut W,
        registry: &mut SymbolRegistry,
        interner: &Interner,
    ) -> std::fmt::Result {
        self.write_to_inner(w, registry, interner, true)
    }

    /// Write term preserving original case (for code generation)
    pub fn write_to_raw<W: Write>(
        &self,
        w: &mut W,
        interner: &Interner,
    ) -> std::fmt::Result {
        match self {
            Term::Constant(name) | Term::Variable(name) => {
                write!(w, "{}", interner.resolve(*name))
            }
            Term::Function(name, args) => {
                write!(w, "{}(", interner.resolve(*name))?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    arg.write_to_raw(w, interner)?;
                }
                write!(w, ")")
            }
            Term::Group(members) => {
                write!(w, "(")?;
                for (i, m) in members.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    m.write_to_raw(w, interner)?;
                }
                write!(w, ")")
            }
            Term::Possessed { possessor, possessed } => {
                possessor.write_to_raw(w, interner)?;
                write!(w, ".{}", interner.resolve(*possessed))
            }
            Term::Value { kind, .. } => match kind {
                NumberKind::Integer(n) => write!(w, "{}", n),
                NumberKind::Real(f) => write!(w, "{}", f),
                NumberKind::Symbolic(s) => write!(w, "{}", interner.resolve(*s)),
            }
            Term::Sigma(predicate) => write!(w, "σ({})", interner.resolve(*predicate)),
            Term::Intension(predicate) => write!(w, "^{}", interner.resolve(*predicate)),
            Term::Proposition(expr) => write!(w, "[proposition]"),
        }
    }

    fn write_to_inner<W: Write>(
        &self,
        w: &mut W,
        registry: &mut SymbolRegistry,
        interner: &Interner,
        use_full_names: bool,
    ) -> std::fmt::Result {
        match self {
            Term::Constant(name) => {
                if use_full_names {
                    write!(w, "{}", registry.get_symbol_full(*name, interner))
                } else {
                    write!(w, "{}", registry.get_symbol(*name, interner))
                }
            }
            Term::Variable(name) => write!(w, "{}", interner.resolve(*name)),
            Term::Function(name, args) => {
                let fn_name = if use_full_names {
                    registry.get_symbol_full(*name, interner)
                } else {
                    registry.get_symbol(*name, interner)
                };
                write!(w, "{}(", fn_name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    arg.write_to_inner(w, registry, interner, use_full_names)?;
                }
                write!(w, ")")
            }
            Term::Group(members) => {
                for (i, m) in members.iter().enumerate() {
                    if i > 0 {
                        write!(w, " ⊕ ")?;
                    }
                    m.write_to_inner(w, registry, interner, use_full_names)?;
                }
                Ok(())
            }
            Term::Possessed { possessor, possessed } => {
                let poss_name = if use_full_names {
                    registry.get_symbol_full(*possessed, interner)
                } else {
                    registry.get_symbol(*possessed, interner)
                };
                write!(w, "Poss(")?;
                possessor.write_to_inner(w, registry, interner, use_full_names)?;
                write!(w, ", {})", poss_name)
            }
            Term::Sigma(predicate) => {
                let pred_name = if use_full_names {
                    registry.get_symbol_full(*predicate, interner)
                } else {
                    registry.get_symbol(*predicate, interner)
                };
                write!(w, "σ{}", pred_name)
            }
            Term::Intension(predicate) => {
                // Use full word for intensional terms, not abbreviated symbol
                let word = interner.resolve(*predicate);
                let capitalized = word.chars().next()
                    .map(|c| c.to_uppercase().collect::<String>() + &word[1..])
                    .unwrap_or_default();
                write!(w, "^{}", capitalized)
            }
            Term::Proposition(expr) => {
                write!(w, "[")?;
                expr.write_logic(w, registry, interner, &UnicodeFormatter)?;
                write!(w, "]")
            }
            Term::Value { kind, unit, dimension: _ } => {
                use crate::ast::NumberKind;
                match kind {
                    NumberKind::Real(r) => write!(w, "{}", r)?,
                    NumberKind::Integer(i) => write!(w, "{}", i)?,
                    NumberKind::Symbolic(s) => write!(w, "{}", interner.resolve(*s))?,
                }
                if let Some(u) = unit {
                    write!(w, " {}", interner.resolve(*u))?;
                }
                Ok(())
            }
        }
    }

    pub fn transpile(&self, registry: &mut SymbolRegistry, interner: &Interner) -> String {
        let mut buf = String::new();
        let _ = self.write_to(&mut buf, registry, interner);
        buf
    }
}

impl<'a> LogicExpr<'a> {
    pub fn write_logic<W: Write, F: LogicFormatter>(
        &self,
        w: &mut W,
        registry: &mut SymbolRegistry,
        interner: &Interner,
        fmt: &F,
    ) -> std::fmt::Result {
        match self {
            LogicExpr::Predicate { name, args } => {
                let pred_name = if fmt.use_full_names() {
                    registry.get_symbol_full(*name, interner)
                } else {
                    registry.get_symbol(*name, interner)
                };
                fmt.write_predicate(w, &pred_name, args, registry, interner)
            }

            LogicExpr::Identity { left, right } => {
                if fmt.wrap_identity() {
                    write!(w, "(")?;
                }
                if fmt.preserve_case() {
                    left.write_to_raw(w, interner)?;
                } else if fmt.use_full_names() {
                    left.write_to_full(w, registry, interner)?;
                } else {
                    left.write_to(w, registry, interner)?;
                }
                write!(w, "{}", fmt.identity())?;
                if fmt.preserve_case() {
                    right.write_to_raw(w, interner)?;
                } else if fmt.use_full_names() {
                    right.write_to_full(w, registry, interner)?;
                } else {
                    right.write_to(w, registry, interner)?;
                }
                if fmt.wrap_identity() {
                    write!(w, ")")?;
                }
                Ok(())
            }

            LogicExpr::Metaphor { tenor, vehicle } => {
                write!(w, "Metaphor(")?;
                tenor.write_to(w, registry, interner)?;
                write!(w, ", ")?;
                vehicle.write_to(w, registry, interner)?;
                write!(w, ")")
            }

            LogicExpr::Quantifier { kind, variable, body, .. } => {
                let var_str = interner.resolve(*variable);
                let mut body_buf = String::new();
                body.write_logic(&mut body_buf, registry, interner, fmt)?;
                write!(w, "{}", fmt.quantifier(kind, var_str, &body_buf))
            }

            LogicExpr::Categorical(data) => {
                let s = fmt.sanitize(&data.subject.to_symbol(registry, interner));
                let p = fmt.sanitize(&data.predicate.to_symbol(registry, interner));
                match (&data.quantifier, data.copula_negative) {
                    (TokenType::All, false) => write!(w, "{} {} is {}", fmt.categorical_all(), s, p),
                    (TokenType::No, false) => write!(w, "{} {} is {}", fmt.categorical_no(), s, p),
                    (TokenType::Some, false) => write!(w, "{} {} is {}", fmt.categorical_some(), s, p),
                    (TokenType::Some, true) => write!(w, "{} {} is {} {}", fmt.categorical_some(), s, fmt.categorical_not(), p),
                    (TokenType::All, true) => write!(w, "{} {} is {} {}", fmt.categorical_some(), s, fmt.categorical_not(), p),
                    _ => write!(w, "Invalid Syllogism"),
                }
            }

            LogicExpr::Relation(data) => {
                let s = data.subject.to_symbol(registry, interner);
                let v = fmt.sanitize(&registry.get_symbol(data.verb, interner));
                let o = data.object.to_symbol(registry, interner);
                write!(w, "{}({}, {})", v, s, o)
            }

            LogicExpr::Modal { vector, operand } => {
                let mut o = String::new();
                operand.write_logic(&mut o, registry, interner, fmt)?;
                write!(w, "{}", fmt.modal(vector.domain, vector.force, &o))
            }

            LogicExpr::BinaryOp { left, op, right } => {
                let mut l = String::new();
                let mut r = String::new();
                left.write_logic(&mut l, registry, interner, fmt)?;
                right.write_logic(&mut r, registry, interner, fmt)?;
                write!(w, "{}", fmt.binary_op(op, &l, &r))
            }

            LogicExpr::UnaryOp { op, operand } => {
                let mut o = String::new();
                operand.write_logic(&mut o, registry, interner, fmt)?;
                write!(w, "{}", fmt.unary_op(op, &o))
            }

            LogicExpr::Temporal { operator, body } => {
                let mut inner = String::new();
                body.write_logic(&mut inner, registry, interner, fmt)?;
                write!(w, "{}", fmt.temporal(operator, &inner))
            }

            LogicExpr::Aspectual { operator, body } => {
                let mut inner = String::new();
                body.write_logic(&mut inner, registry, interner, fmt)?;
                write!(w, "{}", fmt.aspectual(operator, &inner))
            }

            LogicExpr::Voice { operator, body } => {
                let mut inner = String::new();
                body.write_logic(&mut inner, registry, interner, fmt)?;
                write!(w, "{}", fmt.voice(operator, &inner))
            }

            LogicExpr::Question { wh_variable, body } => {
                let mut body_str = String::new();
                body.write_logic(&mut body_str, registry, interner, fmt)?;
                write!(w, "{}", fmt.lambda(interner.resolve(*wh_variable), &body_str))
            }

            LogicExpr::YesNoQuestion { body } => {
                write!(w, "?")?;
                body.write_logic(w, registry, interner, fmt)
            }

            LogicExpr::Atom(s) => {
                let name = if fmt.preserve_case() {
                    interner.resolve(*s).to_string()
                } else if fmt.use_full_names() {
                    registry.get_symbol_full(*s, interner)
                } else {
                    registry.get_symbol(*s, interner)
                };
                write!(w, "{}", fmt.sanitize(&name))
            }

            LogicExpr::Lambda { variable, body } => {
                let mut b = String::new();
                body.write_logic(&mut b, registry, interner, fmt)?;
                write!(w, "{}", fmt.lambda(interner.resolve(*variable), &b))
            }

            LogicExpr::App { function, argument } => {
                write!(w, "(")?;
                function.write_logic(w, registry, interner, fmt)?;
                write!(w, ")(")?;
                argument.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Intensional { operator, content } => {
                write!(w, "{}[", fmt.sanitize(&registry.get_symbol(*operator, interner)))?;
                content.write_logic(w, registry, interner, fmt)?;
                write!(w, "]")
            }

            LogicExpr::Event { predicate, adverbs } => {
                let mut pred_str = String::new();
                predicate.write_logic(&mut pred_str, registry, interner, fmt)?;
                let adverb_preds: Vec<String> = adverbs
                    .iter()
                    .map(|a| format!("{}(e)", fmt.sanitize(&registry.get_symbol(*a, interner))))
                    .collect();
                write!(w, "{}", fmt.event_quantifier(&pred_str, &adverb_preds))
            }

            LogicExpr::NeoEvent(data) => {
                use crate::ast::{QuantifierKind, ThematicRole};

                if fmt.use_simple_events() {
                    write!(w, "{}", registry.get_symbol_full(data.verb, interner))?;
                    write!(w, "(")?;
                    let mut first = true;
                    for (role, term) in data.roles.iter() {
                        if matches!(role, ThematicRole::Agent | ThematicRole::Patient | ThematicRole::Theme) {
                            if !first {
                                write!(w, ", ")?;
                            }
                            first = false;
                            term.write_to_full(w, registry, interner)?;
                        }
                    }
                    write!(w, ")")
                } else {
                    let e = interner.resolve(data.event_var);
                    let mut body = String::new();
                    write_capitalized(&mut body, interner.resolve(data.verb))?;
                    write!(body, "({})", e)?;
                    for (role, term) in data.roles.iter() {
                        let role_str = match role {
                            ThematicRole::Agent => "Agent",
                            ThematicRole::Patient => "Patient",
                            ThematicRole::Theme => "Theme",
                            ThematicRole::Recipient => "Recipient",
                            ThematicRole::Goal => "Goal",
                            ThematicRole::Source => "Source",
                            ThematicRole::Instrument => "Instrument",
                            ThematicRole::Location => "Location",
                            ThematicRole::Time => "Time",
                            ThematicRole::Manner => "Manner",
                        };
                        write!(body, " {} {}({}, ", fmt.and(), role_str, e)?;
                        term.write_to(&mut body, registry, interner)?;
                        write!(body, ")")?;
                    }
                    for mod_sym in data.modifiers.iter() {
                        write!(body, " {} ", fmt.and())?;
                        write_capitalized(&mut body, interner.resolve(*mod_sym))?;
                        write!(body, "({})", e)?;
                    }
                    write!(w, "{}", fmt.quantifier(&QuantifierKind::Existential, e, &body))
                }
            }

            LogicExpr::Imperative { action } => {
                write!(w, "!")?;
                action.write_logic(w, registry, interner, fmt)
            }

            LogicExpr::SpeechAct { performer, act_type, content } => {
                write!(w, "SpeechAct({}, {}, ", interner.resolve(*act_type), fmt.sanitize(&registry.get_symbol(*performer, interner)))?;
                content.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Counterfactual { antecedent, consequent } => {
                let mut a = String::new();
                let mut c = String::new();
                antecedent.write_logic(&mut a, registry, interner, fmt)?;
                consequent.write_logic(&mut c, registry, interner, fmt)?;
                write!(w, "{}", fmt.counterfactual(&a, &c))
            }

            LogicExpr::Causal { effect, cause } => {
                write!(w, "Cause(")?;
                cause.write_logic(w, registry, interner, fmt)?;
                write!(w, ", ")?;
                effect.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Comparative { adjective, subject, object, difference } => {
                let adj = interner.resolve(*adjective);
                let mut subj_buf = String::new();
                if fmt.preserve_case() {
                    subject.write_to_raw(&mut subj_buf, interner)?;
                } else {
                    subject.write_to(&mut subj_buf, registry, interner)?;
                }
                let mut obj_buf = String::new();
                if fmt.preserve_case() {
                    object.write_to_raw(&mut obj_buf, interner)?;
                } else {
                    object.write_to(&mut obj_buf, registry, interner)?;
                }
                let diff_str = if let Some(diff) = difference {
                    let mut diff_buf = String::new();
                    if fmt.preserve_case() {
                        diff.write_to_raw(&mut diff_buf, interner)?;
                    } else {
                        diff.write_to(&mut diff_buf, registry, interner)?;
                    }
                    Some(diff_buf)
                } else {
                    None
                };
                fmt.write_comparative(w, adj, &subj_buf, &obj_buf, diff_str.as_deref())
            }

            LogicExpr::Superlative { adjective, subject, domain } => {
                let mut s = String::new();
                subject.write_to(&mut s, registry, interner)?;
                let mut d = String::new();
                write_capitalized(&mut d, interner.resolve(*domain))?;
                let comp = format!("{}er", interner.resolve(*adjective));
                write!(w, "{}", fmt.superlative(&comp, &d, &s))
            }

            LogicExpr::Scopal { operator, body } => {
                write!(w, "{}(", interner.resolve(*operator))?;
                body.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::TemporalAnchor { anchor, body } => {
                write!(w, "{}(", interner.resolve(*anchor))?;
                body.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Control { verb, subject, object, infinitive } => {
                write!(w, "{}(", fmt.sanitize(&registry.get_symbol(*verb, interner)))?;
                subject.write_to(w, registry, interner)?;
                if let Some(obj) = object {
                    write!(w, ", ")?;
                    obj.write_to(w, registry, interner)?;
                }
                write!(w, ", ")?;
                infinitive.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Presupposition { assertion, presupposition } => {
                assertion.write_logic(w, registry, interner, fmt)?;
                write!(w, " [Presup: ")?;
                presupposition.write_logic(w, registry, interner, fmt)?;
                write!(w, "]")
            }

            LogicExpr::Focus { kind, focused, scope } => {
                use crate::token::FocusKind;
                let prefix = match kind {
                    FocusKind::Only => "Only",
                    FocusKind::Even => "Even",
                    FocusKind::Just => "Just",
                };
                write!(w, "{}(", prefix)?;
                focused.write_to(w, registry, interner)?;
                write!(w, ", ")?;
                scope.write_logic(w, registry, interner, fmt)?;
                write!(w, ")")
            }

            LogicExpr::Distributive { predicate } => {
                write!(w, "*")?;
                predicate.write_logic(w, registry, interner, fmt)
            }

            LogicExpr::GroupQuantifier { group_var, count, member_var, restriction, body } => {
                let g = interner.resolve(*group_var);
                let x = interner.resolve(*member_var);

                // ∃g(Group(g) ∧ Count(g,n) ∧ ∀x(Member(x,g) → restriction) ∧ body)
                write!(w, "{}{}(Group({}) {} Count({}, {}) {} {}{}(Member({}, {}) {} ",
                    fmt.existential(), g, g,
                    fmt.and(), g, count,
                    fmt.and(), fmt.universal(), x, x, g, fmt.implies())?;

                restriction.write_logic(w, registry, interner, fmt)?;

                write!(w, ") {} ", fmt.and())?;

                body.write_logic(w, registry, interner, fmt)?;

                write!(w, ")")
            }
        }
    }

    pub fn transpile_with<F: LogicFormatter>(
        &self,
        registry: &mut SymbolRegistry,
        interner: &Interner,
        fmt: &F,
    ) -> String {
        let mut buf = String::new();
        let _ = self.write_logic(&mut buf, registry, interner, fmt);
        buf
    }

    pub fn transpile(
        &self,
        registry: &mut SymbolRegistry,
        interner: &Interner,
        format: OutputFormat,
    ) -> String {
        match format {
            OutputFormat::Unicode => self.transpile_with(registry, interner, &UnicodeFormatter),
            OutputFormat::LaTeX => self.transpile_with(registry, interner, &LatexFormatter),
            OutputFormat::SimpleFOL => self.transpile_with(registry, interner, &SimpleFOLFormatter),
        }
    }

    pub fn transpile_ctx<F: LogicFormatter>(
        &self,
        ctx: &mut TranspileContext<'_>,
        fmt: &F,
    ) -> String {
        self.transpile_with(ctx.registry, ctx.interner, fmt)
    }

    pub fn transpile_ctx_unicode(&self, ctx: &mut TranspileContext<'_>) -> String {
        self.transpile_ctx(ctx, &UnicodeFormatter)
    }

    pub fn transpile_ctx_latex(&self, ctx: &mut TranspileContext<'_>) -> String {
        self.transpile_ctx(ctx, &LatexFormatter)
    }
}
