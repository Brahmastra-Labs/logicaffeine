mod clause;
mod common;
mod modal;
mod noun;
mod pragmatics;
mod quantifier;
mod question;
mod verb;

#[cfg(test)]
mod tests;

pub use clause::ClauseParsing;
pub use modal::ModalParsing;
pub use noun::NounParsing;
pub use pragmatics::PragmaticsParsing;
pub use quantifier::QuantifierParsing;
pub use question::QuestionParsing;
pub use verb::{LogicVerbParsing, ImperativeVerbParsing};

use crate::analysis::TypeRegistry;
use crate::arena_ctx::AstContext;
use crate::ast::{AspectOperator, LogicExpr, NeoEventData, NumberKind, QuantifierKind, TemporalOperator, Term, ThematicRole, Stmt, Expr, Literal, TypeExpr, BinaryOpKind, MatchArm};
use crate::ast::stmt::ReadSource;
use crate::context::{Case, DiscourseContext, Entity, Gender, Number};
use crate::drs::{Drs, BoxType};
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::{Interner, Symbol, SymbolEq};
use crate::lexer::Lexer;
use crate::lexicon::{self, Aspect, Definiteness, Time, VerbClass};
use crate::token::{BlockType, FocusKind, Token, TokenType};

pub(super) type ParseResult<T> = Result<T, ParseError>;

use std::ops::{Deref, DerefMut};

/// Determines how the parser interprets sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParserMode {
    /// Logicaffeine mode: propositions, NeoEvents, ambiguity allowed.
    #[default]
    Declarative,
    /// LOGOS mode: statements, strict scoping, deterministic.
    Imperative,
}

#[derive(Clone)]
struct ParserCheckpoint {
    pos: usize,
    var_counter: usize,
    bindings_len: usize,
    island: u32,
    time: Option<Time>,
    negative_depth: u32,
}

pub struct ParserGuard<'p, 'a, 'ctx, 'int> {
    parser: &'p mut Parser<'a, 'ctx, 'int>,
    checkpoint: ParserCheckpoint,
    committed: bool,
}

impl<'p, 'a, 'ctx, 'int> ParserGuard<'p, 'a, 'ctx, 'int> {
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl<'p, 'a, 'ctx, 'int> Drop for ParserGuard<'p, 'a, 'ctx, 'int> {
    fn drop(&mut self) {
        if !self.committed {
            self.parser.restore(self.checkpoint.clone());
        }
    }
}

impl<'p, 'a, 'ctx, 'int> Deref for ParserGuard<'p, 'a, 'ctx, 'int> {
    type Target = Parser<'a, 'ctx, 'int>;
    fn deref(&self) -> &Self::Target {
        self.parser
    }
}

impl<'p, 'a, 'ctx, 'int> DerefMut for ParserGuard<'p, 'a, 'ctx, 'int> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.parser
    }
}

#[derive(Clone, Debug)]
pub struct EventTemplate<'a> {
    pub verb: Symbol,
    pub non_agent_roles: Vec<(ThematicRole, Term<'a>)>,
    pub modifiers: Vec<Symbol>,
}

pub struct Parser<'a, 'ctx, 'int> {
    pub(super) tokens: Vec<Token>,
    pub(super) current: usize,
    pub(super) var_counter: usize,
    pub(super) pending_time: Option<Time>,
    pub(super) context: Option<&'ctx mut DiscourseContext>,
    pub(super) donkey_bindings: Vec<(Symbol, Symbol, bool)>,
    pub(super) interner: &'int mut Interner,
    pub(super) ctx: AstContext<'a>,
    pub(super) current_island: u32,
    pub(super) pp_attach_to_noun: bool,
    pub(super) filler_gap: Option<Symbol>,
    pub(super) negative_depth: u32,
    pub(super) discourse_event_var: Option<Symbol>,
    pub(super) last_event_template: Option<EventTemplate<'a>>,
    pub(super) noun_priority_mode: bool,
    pub(super) collective_mode: bool,
    pub(super) pending_cardinal: Option<u32>,
    pub(super) mode: ParserMode,
    pub(super) type_registry: Option<TypeRegistry>,
    pub(super) event_reading_mode: bool,
    pub(super) drs: Drs,
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    pub fn new(
        tokens: Vec<Token>,
        interner: &'int mut Interner,
        ctx: AstContext<'a>,
    ) -> Self {
        Parser {
            tokens,
            current: 0,
            var_counter: 0,
            pending_time: None,
            context: None,
            donkey_bindings: Vec::new(),
            interner,
            ctx,
            current_island: 0,
            pp_attach_to_noun: false,
            filler_gap: None,
            negative_depth: 0,
            discourse_event_var: None,
            last_event_template: None,
            noun_priority_mode: false,
            collective_mode: false,
            pending_cardinal: None,
            mode: ParserMode::Declarative,
            type_registry: None,
            event_reading_mode: false,
            drs: Drs::new(),
        }
    }

    pub fn set_noun_priority_mode(&mut self, mode: bool) {
        self.noun_priority_mode = mode;
    }

    pub fn set_collective_mode(&mut self, mode: bool) {
        self.collective_mode = mode;
    }

    pub fn set_event_reading_mode(&mut self, mode: bool) {
        self.event_reading_mode = mode;
    }

    pub fn with_context(
        tokens: Vec<Token>,
        context: &'ctx mut DiscourseContext,
        interner: &'int mut Interner,
        ctx: AstContext<'a>,
    ) -> Self {
        Parser {
            tokens,
            current: 0,
            var_counter: 0,
            pending_time: None,
            context: Some(context),
            donkey_bindings: Vec::new(),
            interner,
            ctx,
            current_island: 0,
            pp_attach_to_noun: false,
            filler_gap: None,
            negative_depth: 0,
            discourse_event_var: None,
            last_event_template: None,
            noun_priority_mode: false,
            collective_mode: false,
            pending_cardinal: None,
            mode: ParserMode::Declarative,
            type_registry: None,
            event_reading_mode: false,
            drs: Drs::new(),
        }
    }

    /// Create a parser with type registry for two-pass compilation.
    /// The type registry enables disambiguation of "Stack of Integers" (generic)
    /// vs "Owner of House" (possessive).
    pub fn with_types(
        tokens: Vec<Token>,
        context: &'ctx mut DiscourseContext,
        interner: &'int mut Interner,
        ctx: AstContext<'a>,
        types: TypeRegistry,
    ) -> Self {
        Parser {
            tokens,
            current: 0,
            var_counter: 0,
            pending_time: None,
            context: Some(context),
            donkey_bindings: Vec::new(),
            interner,
            ctx,
            current_island: 0,
            pp_attach_to_noun: false,
            filler_gap: None,
            negative_depth: 0,
            discourse_event_var: None,
            last_event_template: None,
            noun_priority_mode: false,
            collective_mode: false,
            pending_cardinal: None,
            mode: ParserMode::Declarative,
            type_registry: Some(types),
            event_reading_mode: false,
            drs: Drs::new(),
        }
    }

    pub fn set_discourse_event_var(&mut self, var: Symbol) {
        self.discourse_event_var = Some(var);
    }

    pub fn mode(&self) -> ParserMode {
        self.mode
    }

    /// Check if a symbol is a known type in the registry.
    /// Used to disambiguate "Stack of Integers" (generic type) vs "Owner of House" (possessive).
    pub fn is_known_type(&self, sym: Symbol) -> bool {
        self.type_registry
            .as_ref()
            .map(|r| r.is_type(sym))
            .unwrap_or(false)
    }

    /// Check if a symbol is a known generic type (takes type parameters).
    /// Used to parse "Stack of Integers" as generic instantiation.
    pub fn is_generic_type(&self, sym: Symbol) -> bool {
        self.type_registry
            .as_ref()
            .map(|r| r.is_generic(sym))
            .unwrap_or(false)
    }

    /// Get the parameter count for a generic type.
    fn get_generic_param_count(&self, sym: Symbol) -> Option<usize> {
        use crate::analysis::TypeDef;
        self.type_registry.as_ref().and_then(|r| {
            match r.get(sym) {
                Some(TypeDef::Generic { param_count }) => Some(*param_count),
                _ => None,
            }
        })
    }

    /// Phase 33: Check if a symbol is a known enum variant and return the enum name.
    fn find_variant(&self, sym: Symbol) -> Option<Symbol> {
        self.type_registry
            .as_ref()
            .and_then(|r| r.find_variant(sym).map(|(enum_name, _)| enum_name))
    }

    /// Consume a type name token (doesn't check entity registration).
    fn consume_type_name(&mut self) -> ParseResult<Symbol> {
        let t = self.advance().clone();
        match t.kind {
            TokenType::Noun(s) | TokenType::Adjective(s) => Ok(s),
            TokenType::ProperName(s) => Ok(s),
            other => Err(ParseError {
                kind: ParseErrorKind::ExpectedContentWord { found: other },
                span: self.current_span(),
            }),
        }
    }

    /// Parse a type expression: Int, Text, List of Int, Result of Int and Text.
    /// Phase 36: Also supports "Type from Module" for qualified imports.
    /// Uses TypeRegistry to distinguish primitives from generics.
    fn parse_type_expression(&mut self) -> ParseResult<TypeExpr<'a>> {
        use noun::NounParsing;

        // Phase 53: Handle "Persistent T" type modifier
        if self.check(&TokenType::Persistent) {
            self.advance(); // consume "Persistent"
            let inner = self.parse_type_expression()?;
            let inner_ref = self.ctx.alloc_type_expr(inner);
            return Ok(TypeExpr::Persistent { inner: inner_ref });
        }

        // Get the base type name (must be a noun or proper name - type names bypass entity check)
        let base = self.consume_type_name()?;

        // Phase 36: Check for "from Module" qualification
        let base_type = if self.check(&TokenType::From) {
            self.advance(); // consume "from"
            let module_name = self.consume_type_name()?;
            let module_str = self.interner.resolve(module_name);
            let base_str = self.interner.resolve(base);
            let qualified = format!("{}::{}", module_str, base_str);
            let qualified_sym = self.interner.intern(&qualified);
            TypeExpr::Named(qualified_sym)
        } else {
            // Phase 38: Get param count from registry OR from built-in std types
            let base_name = self.interner.resolve(base);
            let param_count = self.get_generic_param_count(base)
                .or_else(|| match base_name {
                    // Built-in generic types for Phase 38 std library
                    "Result" => Some(2),    // Result of T and E
                    "Option" => Some(1),    // Option of T
                    "Seq" | "List" | "Vec" => Some(1),  // Seq of T
                    "Set" | "HashSet" => Some(1), // Set of T
                    "Map" | "HashMap" => Some(2), // Map of K and V
                    "Pair" => Some(2),      // Pair of A and B
                    "Triple" => Some(3),    // Triple of A and B and C
                    _ => None,
                });

            // Check if it's a known generic type with parameters
            if let Some(count) = param_count {
                if self.check_of_preposition() || self.check_preposition_is("from") {
                    self.advance(); // consume "of" or "from"

                    let mut params = Vec::new();
                    for i in 0..count {
                        if i > 0 {
                            // Expect separator for params > 1: "and", "to", or ","
                            if self.check(&TokenType::And) || self.check_to_preposition() || self.check(&TokenType::Comma) {
                                self.advance();
                            }
                        }
                        let param = self.parse_type_expression()?;
                        params.push(param);
                    }

                    let params_slice = self.ctx.alloc_type_exprs(params);
                    TypeExpr::Generic { base, params: params_slice }
                } else {
                    // Generic type without parameters - treat as primitive or named
                    let is_primitive = self.type_registry.as_ref().map(|r| r.is_type(base)).unwrap_or(false)
                        || matches!(base_name, "Int" | "Nat" | "Text" | "Bool" | "Boolean" | "Real" | "Unit");
                    if is_primitive {
                        TypeExpr::Primitive(base)
                    } else {
                        TypeExpr::Named(base)
                    }
                }
            } else {
                // Check if it's a known primitive type (Int, Nat, Text, Bool, Real, Unit)
                let is_primitive = self.type_registry.as_ref().map(|r| r.is_type(base)).unwrap_or(false)
                    || matches!(base_name, "Int" | "Nat" | "Text" | "Bool" | "Boolean" | "Real" | "Unit");
                if is_primitive {
                    TypeExpr::Primitive(base)
                } else {
                    // User-defined or unknown type
                    TypeExpr::Named(base)
                }
            }
        };

        // Phase 43C: Check for refinement "where" clause
        if self.check(&TokenType::Where) {
            self.advance(); // consume "where"

            // Parse the predicate expression (supports compound: `x > 0 and x < 100`)
            let predicate_expr = self.parse_condition()?;

            // Extract bound variable from the left side of the expression
            let bound_var = self.extract_bound_var(&predicate_expr)
                .unwrap_or_else(|| self.interner.intern("it"));

            // Convert imperative Expr to logic LogicExpr
            let predicate = self.expr_to_logic_predicate(&predicate_expr, bound_var)
                .ok_or_else(|| ParseError {
                    kind: ParseErrorKind::InvalidRefinementPredicate,
                    span: self.peek().span,
                })?;

            // Allocate the base type
            let base_alloc = self.ctx.alloc_type_expr(base_type);

            return Ok(TypeExpr::Refinement { base: base_alloc, var: bound_var, predicate });
        }

        Ok(base_type)
    }

    /// Extracts the leftmost identifier from an expression as the bound variable.
    fn extract_bound_var(&self, expr: &Expr<'a>) -> Option<Symbol> {
        match expr {
            Expr::Identifier(sym) => Some(*sym),
            Expr::BinaryOp { left, .. } => self.extract_bound_var(left),
            _ => None,
        }
    }

    /// Converts an imperative comparison Expr to a Logic Kernel LogicExpr.
    /// Used for refinement type predicates: `Int where x > 0`
    fn expr_to_logic_predicate(&mut self, expr: &Expr<'a>, bound_var: Symbol) -> Option<&'a LogicExpr<'a>> {
        match expr {
            Expr::BinaryOp { op, left, right } => {
                // Map BinaryOpKind to predicate name
                let pred_name = match op {
                    BinaryOpKind::Gt => "Greater",
                    BinaryOpKind::Lt => "Less",
                    BinaryOpKind::GtEq => "GreaterEqual",
                    BinaryOpKind::LtEq => "LessEqual",
                    BinaryOpKind::Eq => "Equal",
                    BinaryOpKind::NotEq => "NotEqual",
                    BinaryOpKind::And => {
                        // Handle compound `x > 0 and x < 100`
                        let left_logic = self.expr_to_logic_predicate(left, bound_var)?;
                        let right_logic = self.expr_to_logic_predicate(right, bound_var)?;
                        return Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: left_logic,
                            op: TokenType::And,
                            right: right_logic,
                        }));
                    }
                    BinaryOpKind::Or => {
                        let left_logic = self.expr_to_logic_predicate(left, bound_var)?;
                        let right_logic = self.expr_to_logic_predicate(right, bound_var)?;
                        return Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: left_logic,
                            op: TokenType::Or,
                            right: right_logic,
                        }));
                    }
                    _ => return None, // Arithmetic ops not valid as predicates
                };
                let pred_sym = self.interner.intern(pred_name);

                // Convert operands to Terms
                let left_term = self.expr_to_term(left)?;
                let right_term = self.expr_to_term(right)?;

                let args = self.ctx.terms.alloc_slice([left_term, right_term]);
                Some(self.ctx.exprs.alloc(LogicExpr::Predicate { name: pred_sym, args }))
            }
            _ => None,
        }
    }

    /// Converts an imperative Expr to a logic Term.
    fn expr_to_term(&mut self, expr: &Expr<'a>) -> Option<Term<'a>> {
        match expr {
            Expr::Identifier(sym) => Some(Term::Variable(*sym)),
            Expr::Literal(lit) => {
                match lit {
                    Literal::Number(n) => Some(Term::Value {
                        kind: NumberKind::Integer(*n),
                        unit: None,
                        dimension: None,
                    }),
                    Literal::Boolean(b) => {
                        let sym = self.interner.intern(if *b { "true" } else { "false" });
                        Some(Term::Constant(sym))
                    }
                    _ => None, // Text, Nothing not supported in predicates
                }
            }
            _ => None,
        }
    }

    pub fn process_block_headers(&mut self) {
        use crate::token::BlockType;

        while self.current < self.tokens.len() {
            if let TokenType::BlockHeader { block_type } = &self.tokens[self.current].kind {
                self.mode = match block_type {
                    BlockType::Main | BlockType::Function => ParserMode::Imperative,
                    BlockType::Theorem | BlockType::Definition | BlockType::Proof |
                    BlockType::Example | BlockType::Logic | BlockType::Note | BlockType::TypeDef |
                    BlockType::Policy => ParserMode::Declarative,
                };
                self.current += 1;
            } else {
                break;
            }
        }
    }

    pub fn get_event_var(&mut self) -> Symbol {
        self.discourse_event_var.unwrap_or_else(|| self.interner.intern("e"))
    }

    pub fn capture_event_template(&mut self, verb: Symbol, roles: &[(ThematicRole, Term<'a>)], modifiers: &[Symbol]) {
        let non_agent_roles: Vec<_> = roles.iter()
            .filter(|(role, _)| *role != ThematicRole::Agent)
            .cloned()
            .collect();
        self.last_event_template = Some(EventTemplate {
            verb,
            non_agent_roles,
            modifiers: modifiers.to_vec(),
        });
    }

    fn parse_embedded_wh_clause(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // Parse embedded question body: "who runs", "what John ate"
        let var_name = self.interner.intern("x");
        let var_term = Term::Variable(var_name);

        if self.check_verb() {
            // "who runs" pattern
            let verb = self.consume_verb();
            let body = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([var_term]),
            });
            return Ok(body);
        }

        if self.check_content_word() || self.check_article() {
            // "what John ate" pattern
            let subject = self.parse_noun_phrase(true)?;
            if self.check_verb() {
                let verb = self.consume_verb();
                let body = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([
                        Term::Constant(subject.noun),
                        var_term,
                    ]),
                });
                return Ok(body);
            }
        }

        // Fallback: just the wh-variable
        Ok(self.ctx.exprs.alloc(LogicExpr::Atom(var_name)))
    }

    pub fn set_pp_attachment_mode(&mut self, attach_to_noun: bool) {
        self.pp_attach_to_noun = attach_to_noun;
    }

    fn checkpoint(&self) -> ParserCheckpoint {
        ParserCheckpoint {
            pos: self.current,
            var_counter: self.var_counter,
            bindings_len: self.donkey_bindings.len(),
            island: self.current_island,
            time: self.pending_time,
            negative_depth: self.negative_depth,
        }
    }

    fn restore(&mut self, cp: ParserCheckpoint) {
        self.current = cp.pos;
        self.var_counter = cp.var_counter;
        self.donkey_bindings.truncate(cp.bindings_len);
        self.current_island = cp.island;
        self.pending_time = cp.time;
        self.negative_depth = cp.negative_depth;
    }

    fn is_negative_context(&self) -> bool {
        self.negative_depth % 2 == 1
    }

    pub fn guard(&mut self) -> ParserGuard<'_, 'a, 'ctx, 'int> {
        ParserGuard {
            checkpoint: self.checkpoint(),
            parser: self,
            committed: false,
        }
    }

    pub(super) fn try_parse<F, T>(&mut self, op: F) -> Option<T>
    where
        F: FnOnce(&mut Self) -> ParseResult<T>,
    {
        let cp = self.checkpoint();
        match op(self) {
            Ok(res) => Some(res),
            Err(_) => {
                self.restore(cp);
                None
            }
        }
    }

    fn register_entity(&mut self, symbol: &str, noun_class: &str, gender: Gender, number: Number) {
        use crate::context::OwnershipState;
        if let Some(ref mut ctx) = self.context {
            ctx.register(Entity {
                symbol: symbol.to_string(),
                gender,
                number,
                noun_class: noun_class.to_string(),
                ownership: OwnershipState::Owned,
            });
        }
    }

    fn resolve_pronoun(&mut self, gender: Gender, number: Number) -> Option<Symbol> {
        self.context
            .as_ref()
            .and_then(|ctx| ctx.resolve_pronoun(gender, number))
            .map(|e| e.symbol.clone())
            .map(|s| self.interner.intern(&s))
    }

    fn resolve_donkey_pronoun(&mut self, gender: Gender) -> Option<Symbol> {
        for (noun_class, var_name, used) in self.donkey_bindings.iter_mut().rev() {
            let noun_str = self.interner.resolve(*noun_class);
            let noun_gender = Self::infer_noun_gender(noun_str);
            if noun_gender == gender || gender == Gender::Neuter || noun_gender == Gender::Unknown {
                *used = true; // Mark as used by a pronoun (donkey anaphor)
                return Some(*var_name);
            }
        }
        None
    }

    fn infer_noun_gender(noun: &str) -> Gender {
        let lower = noun.to_lowercase();
        if lexicon::is_female_noun(&lower) {
            Gender::Female
        } else if lexicon::is_male_noun(&lower) {
            Gender::Male
        } else {
            Gender::Unknown
        }
    }

    fn is_plural_noun(noun: &str) -> bool {
        let lower = noun.to_lowercase();
        if lexicon::is_irregular_plural(&lower) {
            return true;
        }
        lower.ends_with('s') && !lower.ends_with("ss") && lower.len() > 2
    }

    fn singularize_noun(noun: &str) -> String {
        let lower = noun.to_lowercase();
        if let Some(singular) = lexicon::singularize(&lower) {
            return singular.to_string();
        }
        if lower.ends_with('s') && !lower.ends_with("ss") && lower.len() > 2 {
            let base = &lower[..lower.len() - 1];
            let mut chars: Vec<char> = base.chars().collect();
            if !chars.is_empty() {
                chars[0] = chars[0].to_uppercase().next().unwrap();
            }
            return chars.into_iter().collect();
        }
        let mut chars: Vec<char> = lower.chars().collect();
        if !chars.is_empty() {
            chars[0] = chars[0].to_uppercase().next().unwrap();
        }
        chars.into_iter().collect()
    }

    fn infer_gender(name: &str) -> Gender {
        let lower = name.to_lowercase();
        if lexicon::is_male_name(&lower) {
            Gender::Male
        } else if lexicon::is_female_name(&lower) {
            Gender::Female
        } else {
            Gender::Unknown
        }
    }


    fn next_var_name(&mut self) -> Symbol {
        const VARS: &[&str] = &["x", "y", "z", "w", "v", "u"];
        let idx = self.var_counter;
        self.var_counter += 1;
        if idx < VARS.len() {
            self.interner.intern(VARS[idx])
        } else {
            let name = format!("x{}", idx - VARS.len() + 1);
            self.interner.intern(&name)
        }
    }

    pub fn parse(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let first = self.parse_sentence()?;

        // Only continue to second sentence if there was a period separator
        // AND there are more tokens after the period
        if self.check(&TokenType::Period) {
            self.advance();

            if !self.is_at_end() {
                let second = self.parse_sentence()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: first,
                    op: TokenType::And,
                    right: second,
                }));
            }
        }

        Ok(first)
    }

    pub fn parse_program(&mut self) -> ParseResult<Vec<Stmt<'a>>> {
        let mut statements = Vec::new();
        let mut in_definition_block = false;

        // Check if we started in a Definition block (from process_block_headers)
        if self.mode == ParserMode::Declarative {
            // Check if the previous token was a Definition header
            // For now, assume Definition blocks should be skipped
            // We'll detect them by checking the content pattern
        }

        while !self.is_at_end() {
            // Handle block headers
            if let Some(Token { kind: TokenType::BlockHeader { block_type }, .. }) = self.tokens.get(self.current) {
                match block_type {
                    BlockType::Definition => {
                        in_definition_block = true;
                        self.mode = ParserMode::Declarative;
                        self.advance();
                        continue;
                    }
                    BlockType::Main => {
                        in_definition_block = false;
                        self.mode = ParserMode::Imperative;
                        self.advance();
                        continue;
                    }
                    BlockType::Function => {
                        in_definition_block = false;
                        self.mode = ParserMode::Imperative;
                        self.advance();
                        // Parse function definition
                        let func_def = self.parse_function_def()?;
                        statements.push(func_def);
                        continue;
                    }
                    BlockType::TypeDef => {
                        // Type definitions are handled by DiscoveryPass
                        // Skip content until next block header
                        self.advance();
                        self.skip_type_def_content();
                        continue;
                    }
                    BlockType::Policy => {
                        // Phase 50: Policy definitions are handled by DiscoveryPass
                        // Skip content until next block header
                        in_definition_block = true;  // Reuse flag to skip content
                        self.mode = ParserMode::Declarative;
                        self.advance();
                        continue;
                    }
                    _ => {
                        in_definition_block = false;
                        self.mode = ParserMode::Declarative;
                        self.advance();
                        continue;
                    }
                }
            }

            // Skip Definition block content - handled by DiscoveryPass
            if in_definition_block {
                self.advance();
                continue;
            }

            // Skip indent/dedent/newline tokens at program level
            if self.check(&TokenType::Indent) || self.check(&TokenType::Dedent) || self.check(&TokenType::Newline) {
                self.advance();
                continue;
            }

            // In imperative mode, parse statements
            if self.mode == ParserMode::Imperative {
                let stmt = self.parse_statement()?;
                statements.push(stmt);

                if self.check(&TokenType::Period) {
                    self.advance();
                }
            } else {
                // In declarative mode (Theorem, etc.), skip for now
                self.advance();
            }
        }

        Ok(statements)
    }

    fn parse_statement(&mut self) -> ParseResult<Stmt<'a>> {
        // Phase 32: Function definitions can appear inside Main block
        // Handle both TokenType::To and Preposition("to")
        if self.check(&TokenType::To) || self.check_preposition_is("to") {
            return self.parse_function_def();
        }
        if self.check(&TokenType::Let) {
            return self.parse_let_statement();
        }
        if self.check(&TokenType::Set) {
            return self.parse_set_statement();
        }
        if self.check(&TokenType::Return) {
            return self.parse_return_statement();
        }
        if self.check(&TokenType::If) {
            return self.parse_if_statement();
        }
        if self.check(&TokenType::Assert) {
            return self.parse_assert_statement();
        }
        // Phase 35: Trust statement
        if self.check(&TokenType::Trust) {
            return self.parse_trust_statement();
        }
        // Phase 50: Security Check statement
        if self.check(&TokenType::Check) {
            return self.parse_check_statement();
        }
        // Phase 51: P2P Networking statements
        if self.check(&TokenType::Listen) {
            return self.parse_listen_statement();
        }
        if self.check(&TokenType::NetConnect) {
            return self.parse_connect_statement();
        }
        if self.check(&TokenType::Sleep) {
            return self.parse_sleep_statement();
        }
        // Phase 52: GossipSub sync statement
        if self.check(&TokenType::Sync) {
            return self.parse_sync_statement();
        }
        // Phase 53: Persistent storage mount statement
        if self.check(&TokenType::Mount) {
            return self.parse_mount_statement();
        }
        if self.check(&TokenType::While) {
            return self.parse_while_statement();
        }
        if self.check(&TokenType::Repeat) {
            return self.parse_repeat_statement();
        }
        if self.check(&TokenType::Call) {
            return self.parse_call_statement();
        }
        if self.check(&TokenType::Give) {
            return self.parse_give_statement();
        }
        if self.check(&TokenType::Show) {
            return self.parse_show_statement();
        }
        // Phase 33: Pattern matching on sum types
        if self.check(&TokenType::Inspect) {
            return self.parse_inspect_statement();
        }

        // Phase 43D: Collection operations
        if self.check(&TokenType::Push) {
            return self.parse_push_statement();
        }
        if self.check(&TokenType::Pop) {
            return self.parse_pop_statement();
        }
        // Set operations
        if self.check(&TokenType::Add) {
            return self.parse_add_statement();
        }
        if self.check(&TokenType::Remove) {
            return self.parse_remove_statement();
        }

        // Phase 8.5: Memory zone block
        if self.check(&TokenType::Inside) {
            return self.parse_zone_statement();
        }

        // Phase 9: Structured Concurrency blocks
        if self.check(&TokenType::Attempt) {
            return self.parse_concurrent_block();
        }
        if self.check(&TokenType::Simultaneously) {
            return self.parse_parallel_block();
        }

        // Phase 10: IO statements
        if self.check(&TokenType::Read) {
            return self.parse_read_statement();
        }
        if self.check(&TokenType::Write) {
            return self.parse_write_statement();
        }

        // Phase 46: Agent System statements
        if self.check(&TokenType::Spawn) {
            return self.parse_spawn_statement();
        }
        if self.check(&TokenType::Send) {
            // Phase 54: Disambiguate "Send x into pipe" vs "Send x to agent"
            if self.lookahead_contains_into() {
                return self.parse_send_pipe_statement();
            }
            return self.parse_send_statement();
        }
        if self.check(&TokenType::Await) {
            // Phase 54: Disambiguate "Await the first of:" vs "Await response from agent"
            if self.lookahead_is_first_of() {
                return self.parse_select_statement();
            }
            return self.parse_await_statement();
        }

        // Phase 49: CRDT statements
        if self.check(&TokenType::Merge) {
            return self.parse_merge_statement();
        }
        if self.check(&TokenType::Increase) {
            return self.parse_increase_statement();
        }

        // Phase 54: Go-like Concurrency statements
        if self.check(&TokenType::Launch) {
            return self.parse_launch_statement();
        }
        if self.check(&TokenType::Stop) {
            return self.parse_stop_statement();
        }
        if self.check(&TokenType::Try) {
            return self.parse_try_statement();
        }
        if self.check(&TokenType::Receive) {
            return self.parse_receive_pipe_statement();
        }

        // Expression-statement: function call without "Call" keyword
        // e.g., `greet("Alice").` instead of `Call greet with "Alice".`
        // Check if next token is LParen (indicating a function call)
        if self.tokens.get(self.current + 1)
            .map(|t| matches!(t.kind, TokenType::LParen))
            .unwrap_or(false)
        {
            // Get the function name from current token
            let function = self.peek().lexeme;
            self.advance(); // consume function name

            // Parse the call expression (starts from LParen)
            let expr = self.parse_call_expr(function)?;
            if let Expr::Call { function, args } = expr {
                return Ok(Stmt::Call { function: *function, args: args.clone() });
            }
        }

        Err(ParseError {
            kind: ParseErrorKind::ExpectedStatement,
            span: self.current_span(),
        })
    }

    fn parse_if_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "If"

        // Parse condition expression (simple: identifier equals value)
        let cond = self.parse_condition()?;

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse then block
        let mut then_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            then_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        // Allocate then_block in arena
        let then_block = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(then_stmts.into_iter());

        // Check for Otherwise: block
        let else_block = if self.check(&TokenType::Otherwise) {
            self.advance(); // consume "Otherwise"

            if !self.check(&TokenType::Colon) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume ":"

            if !self.check(&TokenType::Indent) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedStatement,
                    span: self.current_span(),
                });
            }
            self.advance(); // consume Indent

            let mut else_stmts = Vec::new();
            while !self.check(&TokenType::Dedent) && !self.is_at_end() {
                let stmt = self.parse_statement()?;
                else_stmts.push(stmt);
                if self.check(&TokenType::Period) {
                    self.advance();
                }
            }

            if self.check(&TokenType::Dedent) {
                self.advance();
            }

            Some(self.ctx.stmts.expect("imperative arenas not initialized")
                .alloc_slice(else_stmts.into_iter()))
        } else {
            None
        };

        Ok(Stmt::If {
            cond,
            then_block,
            else_block,
        })
    }

    fn parse_while_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "While"

        let cond = self.parse_condition()?;

        // Phase 44: Parse optional (decreasing expr)
        let decreasing = if self.check(&TokenType::LParen) {
            self.advance(); // consume '('

            // Expect "decreasing" keyword
            if !self.check_word("decreasing") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "decreasing".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume "decreasing"

            let variant = self.parse_imperative_expr()?;

            if !self.check(&TokenType::RParen) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume ')'

            Some(variant)
        } else {
            None
        };

        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        let mut body_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            body_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(Stmt::While { cond, body, decreasing })
    }

    fn parse_repeat_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Repeat"

        // Optional "for"
        if self.check(&TokenType::For) {
            self.advance();
        }

        // Parse loop variable (using context-aware identifier parsing)
        let var = self.expect_identifier()?;

        // Determine iteration type: "in" for collection, "from" for range
        let iterable = if self.check(&TokenType::From) || self.check_preposition_is("from") {
            self.advance(); // consume "from"
            let start = self.parse_imperative_expr()?;

            // Expect "to" (can be keyword or preposition)
            if !self.check(&TokenType::To) && !self.check_preposition_is("to") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            let end = self.parse_imperative_expr()?;
            self.ctx.alloc_imperative_expr(Expr::Range { start, end })
        } else if self.check(&TokenType::In) || self.check_preposition_is("in") {
            self.advance(); // consume "in"
            self.parse_imperative_expr()?
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "in or from".to_string() },
                span: self.current_span(),
            });
        };

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance();

        // Parse body statements
        let mut body_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            body_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(Stmt::Repeat { var, iterable, body })
    }

    fn parse_call_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Call"

        // Parse function name (identifier)
        // Function names can be nouns, adjectives, or verbs (e.g., "work", "process")
        // Use the token's lexeme to match function definition casing
        let function = match &self.peek().kind {
            TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            TokenType::Verb { .. } => {
                // Use lexeme (actual text) not lemma to preserve casing
                let s = self.peek().lexeme;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedIdentifier,
                    span: self.current_span(),
                });
            }
        };

        // Expect "with" followed by arguments
        let args = if self.check_preposition_is("with") {
            self.advance(); // consume "with"
            self.parse_call_arguments()?
        } else {
            Vec::new()
        };

        Ok(Stmt::Call { function, args })
    }

    fn parse_call_arguments(&mut self) -> ParseResult<Vec<&'a Expr<'a>>> {
        let mut args = Vec::new();

        // Parse first argument
        let arg = self.parse_imperative_expr()?;
        args.push(arg);

        // Parse additional comma-separated arguments
        while self.check(&TokenType::Comma) {
            self.advance(); // consume ","
            let arg = self.parse_imperative_expr()?;
            args.push(arg);
        }

        Ok(args)
    }

    fn parse_condition(&mut self) -> ParseResult<&'a Expr<'a>> {
        // Grand Challenge: Parse compound conditions with "and" and "or"
        // "or" has lower precedence than "and"
        self.parse_or_condition()
    }

    /// Parse "or" conditions (lower precedence than "and")
    fn parse_or_condition(&mut self) -> ParseResult<&'a Expr<'a>> {
        let mut left = self.parse_and_condition()?;

        while self.check(&TokenType::Or) || self.check_word("or") {
            self.advance();
            let right = self.parse_and_condition()?;
            left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op: BinaryOpKind::Or,
                left,
                right,
            });
        }

        Ok(left)
    }

    /// Parse "and" conditions (higher precedence than "or")
    fn parse_and_condition(&mut self) -> ParseResult<&'a Expr<'a>> {
        let mut left = self.parse_comparison()?;

        while self.check(&TokenType::And) || self.check_word("and") {
            self.advance();
            let right = self.parse_comparison()?;
            left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op: BinaryOpKind::And,
                left,
                right,
            });
        }

        Ok(left)
    }

    /// Grand Challenge: Parse a single comparison expression
    fn parse_comparison(&mut self) -> ParseResult<&'a Expr<'a>> {
        // Handle unary "not" operator: "not a" or "not (x > 5)"
        if self.check(&TokenType::Not) || self.check_word("not") {
            self.advance(); // consume "not"
            let operand = self.parse_comparison()?; // recursive to handle "not not x"
            // Implement as: operand == false (since we don't have UnaryNot)
            return Ok(self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op: BinaryOpKind::Eq,
                left: operand,
                right: self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Boolean(false))),
            }));
        }

        let left = self.parse_imperative_expr()?;

        // Check for comparison operators
        let op = if self.check(&TokenType::Equals) {
            self.advance();
            Some(BinaryOpKind::Eq)
        } else if self.check(&TokenType::Identity) {
            // "is equal to" was tokenized as TokenType::Identity
            self.advance();
            Some(BinaryOpKind::Eq)
        } else if self.check_word("is") {
            // Peek ahead to determine which comparison
            let saved_pos = self.current;
            self.advance(); // consume "is"

            if self.check_word("greater") {
                self.advance(); // consume "greater"
                if self.check_word("than") || self.check_preposition_is("than") {
                    self.advance(); // consume "than"
                    Some(BinaryOpKind::Gt)
                } else {
                    self.current = saved_pos;
                    None
                }
            } else if self.check_word("less") {
                self.advance(); // consume "less"
                if self.check_word("than") || self.check_preposition_is("than") {
                    self.advance(); // consume "than"
                    Some(BinaryOpKind::Lt)
                } else {
                    self.current = saved_pos;
                    None
                }
            } else if self.check_word("at") {
                self.advance(); // consume "at"
                if self.check_word("least") {
                    self.advance(); // consume "least"
                    Some(BinaryOpKind::GtEq)
                } else if self.check_word("most") {
                    self.advance(); // consume "most"
                    Some(BinaryOpKind::LtEq)
                } else {
                    self.current = saved_pos;
                    None
                }
            } else if self.check_word("not") || self.check(&TokenType::Not) {
                // "is not X" → NotEq
                self.advance(); // consume "not"
                Some(BinaryOpKind::NotEq)
            } else if self.check_word("equal") {
                // "is equal to X" → Eq
                self.advance(); // consume "equal"
                if self.check_preposition_is("to") {
                    self.advance(); // consume "to"
                    Some(BinaryOpKind::Eq)
                } else {
                    self.current = saved_pos;
                    None
                }
            } else {
                self.current = saved_pos;
                None
            }
        } else if self.check(&TokenType::Lt) {
            self.advance();
            Some(BinaryOpKind::Lt)
        } else if self.check(&TokenType::Gt) {
            self.advance();
            Some(BinaryOpKind::Gt)
        } else if self.check(&TokenType::LtEq) {
            self.advance();
            Some(BinaryOpKind::LtEq)
        } else if self.check(&TokenType::GtEq) {
            self.advance();
            Some(BinaryOpKind::GtEq)
        } else if self.check(&TokenType::EqEq) {
            self.advance();
            Some(BinaryOpKind::Eq)
        } else if self.check(&TokenType::NotEq) {
            self.advance();
            Some(BinaryOpKind::NotEq)
        } else {
            None
        };

        if let Some(op) = op {
            let right = self.parse_imperative_expr()?;
            Ok(self.ctx.alloc_imperative_expr(Expr::BinaryOp { op, left, right }))
        } else {
            Ok(left)
        }
    }

    fn parse_let_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Let"

        // Check for "mutable" keyword
        let mutable = if self.check_mutable_keyword() {
            self.advance();
            true
        } else {
            false
        };

        // Get identifier
        let var = self.expect_identifier()?;

        // Check for optional type annotation: `: Type`
        let ty = if self.check(&TokenType::Colon) {
            self.advance(); // consume ":"
            let type_expr = self.parse_type_expression()?;
            Some(self.ctx.alloc_type_expr(type_expr))
        } else {
            None
        };

        // Expect "be"
        if !self.check(&TokenType::Be) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "be".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "be"

        // Phase 53: Check for "mounted at [path]" pattern (for Persistent types)
        if self.check_word("mounted") {
            self.advance(); // consume "mounted"
            if !self.check(&TokenType::At) && !self.check_preposition_is("at") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "at".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume "at"
            let path = self.parse_imperative_expr()?;
            return Ok(Stmt::Mount { var, path });
        }

        // Phase 51: Check for "a PeerAgent at [addr]" pattern
        if self.check_article() {
            let saved_pos = self.current;
            self.advance(); // consume article

            // Check if next word is "PeerAgent" (case insensitive)
            if let TokenType::Noun(sym) | TokenType::ProperName(sym) = self.peek().kind {
                let word = self.interner.resolve(sym).to_lowercase();
                if word == "peeragent" {
                    self.advance(); // consume "PeerAgent"

                    // Check for "at" keyword
                    if self.check(&TokenType::At) || self.check_preposition_is("at") {
                        self.advance(); // consume "at"

                        // Parse address expression
                        let address = self.parse_imperative_expr()?;

                        // Bind in ScopeStack if context available
                        if let Some(ctx) = self.context.as_mut() {
                            use crate::context::{Entity, Gender, Number, OwnershipState};
                            let var_name = self.interner.resolve(var).to_string();
                            ctx.register(Entity {
                                symbol: var_name.clone(),
                                gender: Gender::Neuter,
                                number: Number::Singular,
                                noun_class: var_name,
                                ownership: OwnershipState::Owned,
                            });
                        }

                        return Ok(Stmt::LetPeerAgent { var, address });
                    }
                }
            }
            // Not a PeerAgent, backtrack
            self.current = saved_pos;
        }

        // Phase 54: Check for "a Pipe of Type" pattern
        if self.check_article() {
            let saved_pos = self.current;
            self.advance(); // consume article

            if self.check(&TokenType::Pipe) {
                self.advance(); // consume "Pipe"

                // Expect "of"
                if !self.check_word("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                // Parse element type
                let element_type = self.expect_identifier()?;

                // Register variable in scope
                if let Some(ctx) = self.context.as_mut() {
                    use crate::context::{Entity, Gender, Number, OwnershipState};
                    let var_name = self.interner.resolve(var).to_string();
                    ctx.register(Entity {
                        symbol: var_name.clone(),
                        gender: Gender::Neuter,
                        number: Number::Singular,
                        noun_class: "Pipe".to_string(),
                        ownership: OwnershipState::Owned,
                    });
                }

                return Ok(Stmt::CreatePipe { var, element_type, capacity: None });
            }
            // Not a Pipe, backtrack
            self.current = saved_pos;
        }

        // Phase 54: Check for "Launch a task to..." pattern (for task handles)
        if self.check(&TokenType::Launch) {
            self.advance(); // consume "Launch"

            // Expect "a"
            if !self.check_article() {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "a".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            // Expect "task"
            if !self.check(&TokenType::Task) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "task".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            // Expect "to"
            if !self.check(&TokenType::To) && !self.check_word("to") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            // Parse function name
            let function = self.expect_identifier()?;

            // Parse optional arguments: "with arg1, arg2"
            let args = if self.check_word("with") {
                self.advance();
                self.parse_call_arguments()?
            } else {
                vec![]
            };

            return Ok(Stmt::LaunchTaskWithHandle { handle: var, function, args });
        }

        // Parse expression value (simple: just a number for now)
        let value = self.parse_imperative_expr()?;

        // Phase 43B: Type check - verify declared type matches value type
        if let Some(declared_ty) = &ty {
            if let Some(inferred) = self.infer_literal_type(value) {
                if !self.check_type_compatibility(declared_ty, inferred) {
                    let expected = match declared_ty {
                        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                            self.interner.resolve(*sym).to_string()
                        }
                        _ => "unknown".to_string(),
                    };
                    return Err(ParseError {
                        kind: ParseErrorKind::TypeMismatch {
                            expected,
                            found: inferred.to_string(),
                        },
                        span: self.current_span(),
                    });
                }
            }
        }

        // Bind in ScopeStack if context available
        if let Some(ctx) = self.context.as_mut() {
            use crate::context::{Entity, Gender, Number, OwnershipState};
            let var_name = self.interner.resolve(var).to_string();
            ctx.register(Entity {
                symbol: var_name.clone(),
                gender: Gender::Neuter,
                number: Number::Singular,
                noun_class: var_name,
                ownership: OwnershipState::Owned,
            });
        }

        Ok(Stmt::Let { var, ty, value, mutable })
    }

    fn check_mutable_keyword(&self) -> bool {
        if let TokenType::Noun(sym) | TokenType::Adjective(sym) = self.peek().kind {
            let word = self.interner.resolve(sym).to_lowercase();
            word == "mutable" || word == "mut"
        } else {
            false
        }
    }

    /// Phase 43B: Infer the type of a literal expression
    fn infer_literal_type(&self, expr: &Expr<'_>) -> Option<&'static str> {
        match expr {
            Expr::Literal(lit) => match lit {
                crate::ast::Literal::Number(_) => Some("Int"),
                crate::ast::Literal::Float(_) => Some("Real"),
                crate::ast::Literal::Text(_) => Some("Text"),
                crate::ast::Literal::Boolean(_) => Some("Bool"),
                crate::ast::Literal::Nothing => Some("Unit"),
                crate::ast::Literal::Char(_) => Some("Char"),
            },
            _ => None, // Can't infer type for non-literals yet
        }
    }

    /// Phase 43B: Check if declared type matches inferred type
    fn check_type_compatibility(&self, declared: &TypeExpr<'_>, inferred: &str) -> bool {
        match declared {
            TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                let declared_name = self.interner.resolve(*sym);
                // Nat and Byte are compatible with Int literals
                declared_name.eq_ignore_ascii_case(inferred)
                    || (declared_name.eq_ignore_ascii_case("Nat") && inferred == "Int")
                    || (declared_name.eq_ignore_ascii_case("Byte") && inferred == "Int")
            }
            _ => true, // For generics/functions, skip check for now
        }
    }

    fn parse_set_statement(&mut self) -> ParseResult<Stmt<'a>> {
        use crate::ast::Expr;
        self.advance(); // consume "Set"

        // Parse target - can be identifier or field access expression
        let target_expr = self.parse_imperative_expr()?;

        // Expect "to" - can be TokenType::To or Preposition("to")
        let is_to = self.check(&TokenType::To) || matches!(
            &self.peek().kind,
            TokenType::Preposition(sym) if self.interner.resolve(*sym) == "to"
        );
        if !is_to {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse expression value
        let value = self.parse_imperative_expr()?;

        // Phase 31: Handle field access targets
        // Also handle index targets: Set item N of X to Y
        match target_expr {
            Expr::FieldAccess { object, field } => {
                Ok(Stmt::SetField { object, field: *field, value })
            }
            Expr::Identifier(target) => {
                Ok(Stmt::Set { target: *target, value })
            }
            Expr::Index { collection, index } => {
                Ok(Stmt::SetIndex { collection, index, value })
            }
            _ => Err(ParseError {
                kind: ParseErrorKind::ExpectedIdentifier,
                span: self.current_span(),
            })
        }
    }

    fn parse_return_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Return"

        // Check if there's a value or just "Return."
        if self.check(&TokenType::Period) || self.is_at_end() {
            return Ok(Stmt::Return { value: None });
        }

        // Use parse_comparison to support returning comparison results like "n equals 5"
        let value = self.parse_comparison()?;
        Ok(Stmt::Return { value: Some(value) })
    }

    fn parse_assert_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Assert"

        // Optionally consume "that"
        if self.check(&TokenType::That) {
            self.advance();
        }

        // Parse condition using imperative expression parser
        // This allows syntax like "Assert that b is not 0."
        let condition = self.parse_condition()?;

        Ok(Stmt::RuntimeAssert { condition })
    }

    /// Phase 35: Parse Trust statement
    /// Syntax: Trust [that] [proposition] because [justification].
    fn parse_trust_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Trust"

        // Optionally consume "that"
        if self.check(&TokenType::That) {
            self.advance();
        }

        // Save current mode and switch to declarative for proposition parsing
        let saved_mode = self.mode;
        self.mode = ParserMode::Declarative;

        // Parse the proposition using the Logic Kernel
        let proposition = self.parse()?;

        // Restore mode
        self.mode = saved_mode;

        // Expect "because"
        if !self.check(&TokenType::Because) {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    expected: TokenType::Because,
                    found: self.peek().kind.clone(),
                },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "because"

        // Parse justification (string literal)
        let justification = match &self.peek().kind {
            TokenType::StringLiteral(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        expected: TokenType::StringLiteral(self.interner.intern("")),
                        found: self.peek().kind.clone(),
                    },
                    span: self.current_span(),
                });
            }
        };

        Ok(Stmt::Trust { proposition, justification })
    }

    /// Phase 50: Parse Check statement - mandatory security guard
    /// Syntax: Check that [subject] is [predicate].
    /// Syntax: Check that [subject] can [action] the [object].
    fn parse_check_statement(&mut self) -> ParseResult<Stmt<'a>> {
        let start_span = self.current_span();
        self.advance(); // consume "Check"

        // Optionally consume "that"
        if self.check(&TokenType::That) {
            self.advance();
        }

        // Consume optional "the"
        if matches!(self.peek().kind, TokenType::Article(_)) {
            self.advance();
        }

        // Parse subject identifier (e.g., "user")
        let subject = match &self.peek().kind {
            TokenType::Noun(sym) | TokenType::Adjective(sym) | TokenType::ProperName(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                // Try to get an identifier
                let tok = self.peek();
                let s = tok.lexeme;
                self.advance();
                s
            }
        };

        // Determine if this is a predicate check ("is admin") or capability check ("can publish")
        let is_capability;
        let predicate;
        let object;

        if self.check(&TokenType::Is) || self.check(&TokenType::Are) {
            // Predicate check: "user is admin"
            is_capability = false;
            self.advance(); // consume "is" / "are"

            // Parse predicate name (e.g., "admin")
            predicate = match &self.peek().kind {
                TokenType::Noun(sym) | TokenType::Adjective(sym) | TokenType::ProperName(sym) => {
                    let s = *sym;
                    self.advance();
                    s
                }
                _ => {
                    let tok = self.peek();
                    let s = tok.lexeme;
                    self.advance();
                    s
                }
            };
            object = None;
        } else if self.check(&TokenType::Can) {
            // Capability check: "user can publish the document"
            is_capability = true;
            self.advance(); // consume "can"

            // Parse action (e.g., "publish", "edit", "delete")
            predicate = match &self.peek().kind {
                TokenType::Verb { lemma, .. } => {
                    let s = *lemma;
                    self.advance();
                    s
                }
                TokenType::Noun(sym) | TokenType::Adjective(sym) | TokenType::ProperName(sym) => {
                    let s = *sym;
                    self.advance();
                    s
                }
                _ => {
                    let tok = self.peek();
                    let s = tok.lexeme;
                    self.advance();
                    s
                }
            };

            // Consume optional "the"
            if matches!(self.peek().kind, TokenType::Article(_)) {
                self.advance();
            }

            // Parse object (e.g., "document")
            let obj = match &self.peek().kind {
                TokenType::Noun(sym) | TokenType::Adjective(sym) | TokenType::ProperName(sym) => {
                    let s = *sym;
                    self.advance();
                    s
                }
                _ => {
                    let tok = self.peek();
                    let s = tok.lexeme;
                    self.advance();
                    s
                }
            };
            object = Some(obj);
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "is/can".to_string() },
                span: self.current_span(),
            });
        }

        // Build source text for error message
        let source_text = if is_capability {
            let obj_name = self.interner.resolve(object.unwrap());
            let pred_name = self.interner.resolve(predicate);
            let subj_name = self.interner.resolve(subject);
            format!("{} can {} the {}", subj_name, pred_name, obj_name)
        } else {
            let pred_name = self.interner.resolve(predicate);
            let subj_name = self.interner.resolve(subject);
            format!("{} is {}", subj_name, pred_name)
        };

        Ok(Stmt::Check {
            subject,
            predicate,
            is_capability,
            object,
            source_text,
            span: start_span,
        })
    }

    /// Phase 51: Parse Listen statement - bind to network address
    /// Syntax: Listen on [address].
    fn parse_listen_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Listen"

        // Expect "on" preposition
        if !self.check_preposition_is("on") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "on".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "on"

        // Parse address expression (string literal or variable)
        let address = self.parse_imperative_expr()?;

        Ok(Stmt::Listen { address })
    }

    /// Phase 51: Parse Connect statement - dial remote peer
    /// Syntax: Connect to [address].
    fn parse_connect_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Connect"

        // Expect "to" (can be TokenType::To or preposition)
        if !self.check(&TokenType::To) && !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse address expression
        let address = self.parse_imperative_expr()?;

        Ok(Stmt::ConnectTo { address })
    }

    /// Phase 51: Parse Sleep statement - pause execution
    /// Syntax: Sleep [milliseconds].
    fn parse_sleep_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Sleep"

        // Parse milliseconds expression (number or variable)
        let milliseconds = self.parse_imperative_expr()?;

        Ok(Stmt::Sleep { milliseconds })
    }

    /// Phase 52: Parse Sync statement - automatic CRDT replication
    /// Syntax: Sync [var] on [topic].
    fn parse_sync_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Sync"

        // Parse variable name (must be an identifier)
        let var = match &self.tokens[self.current].kind {
            TokenType::ProperName(sym) | TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "variable name".to_string() },
                    span: self.current_span(),
                });
            }
        };

        // Expect "on" preposition
        if !self.check_preposition_is("on") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "on".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "on"

        // Parse topic expression (string literal or variable)
        let topic = self.parse_imperative_expr()?;

        Ok(Stmt::Sync { var, topic })
    }

    /// Phase 53: Parse Mount statement
    /// Syntax: Mount [var] at [path].
    /// Example: Mount counter at "data/counter.journal".
    fn parse_mount_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Mount"

        // Parse variable name (must be an identifier)
        let var = match &self.tokens[self.current].kind {
            TokenType::ProperName(sym) | TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "variable name".to_string() },
                    span: self.current_span(),
                });
            }
        };

        // Expect "at" keyword (TokenType::At in imperative mode)
        if !self.check(&TokenType::At) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "at".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "at"

        // Parse path expression (string literal or variable)
        let path = self.parse_imperative_expr()?;

        Ok(Stmt::Mount { var, path })
    }

    // =========================================================================
    // Phase 54: Go-like Concurrency Parser Methods
    // =========================================================================

    /// Helper: Check if lookahead contains "into" (for Send...into pipe disambiguation)
    fn lookahead_contains_into(&self) -> bool {
        for i in self.current..std::cmp::min(self.current + 5, self.tokens.len()) {
            if matches!(self.tokens[i].kind, TokenType::Into) {
                return true;
            }
        }
        false
    }

    /// Helper: Check if lookahead is "the first of" (for Await select disambiguation)
    fn lookahead_is_first_of(&self) -> bool {
        // Check for "Await the first of:"
        self.current + 3 < self.tokens.len()
            && matches!(self.tokens.get(self.current + 1), Some(t) if matches!(t.kind, TokenType::Article(_)))
            && self.tokens.get(self.current + 2)
                .map(|t| self.interner.resolve(t.lexeme).to_lowercase() == "first")
                .unwrap_or(false)
    }

    /// Phase 54: Parse Launch statement - spawn a task
    /// Syntax: Launch a task to verb(args).
    fn parse_launch_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Launch"

        // Expect "a"
        if !self.check_article() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "a".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect "task"
        if !self.check(&TokenType::Task) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "task".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect "to"
        if !self.check(&TokenType::To) && !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Parse function name
        let function = match &self.tokens[self.current].kind {
            TokenType::ProperName(sym) | TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "function name".to_string() },
                    span: self.current_span(),
                });
            }
        };

        // Optional arguments in parentheses or with "with" keyword
        let args = if self.check(&TokenType::LParen) {
            self.parse_call_arguments()?
        } else if self.check_word("with") {
            self.advance(); // consume "with"
            let mut args = Vec::new();
            let arg = self.parse_imperative_expr()?;
            args.push(arg);
            // Handle additional args separated by "and"
            while self.check(&TokenType::And) {
                self.advance();
                let arg = self.parse_imperative_expr()?;
                args.push(arg);
            }
            args
        } else {
            Vec::new()
        };

        Ok(Stmt::LaunchTask { function, args })
    }

    /// Phase 54: Parse Send into pipe statement
    /// Syntax: Send value into pipe.
    fn parse_send_pipe_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Send"

        // Parse value expression
        let value = self.parse_imperative_expr()?;

        // Expect "into"
        if !self.check(&TokenType::Into) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "into".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Parse pipe expression
        let pipe = self.parse_imperative_expr()?;

        Ok(Stmt::SendPipe { value, pipe })
    }

    /// Phase 54: Parse Receive from pipe statement
    /// Syntax: Receive x from pipe.
    fn parse_receive_pipe_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Receive"

        // Get variable name - use expect_identifier which handles various token types
        let var = self.expect_identifier()?;

        // Expect "from"
        if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Parse pipe expression
        let pipe = self.parse_imperative_expr()?;

        Ok(Stmt::ReceivePipe { var, pipe })
    }

    /// Phase 54: Parse Try statement (non-blocking send/receive)
    /// Syntax: Try to send x into pipe. OR Try to receive x from pipe.
    fn parse_try_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Try"

        // Expect "to"
        if !self.check(&TokenType::To) && !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Check if send or receive
        if self.check(&TokenType::Send) {
            self.advance(); // consume "Send"
            let value = self.parse_imperative_expr()?;

            if !self.check(&TokenType::Into) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "into".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            let pipe = self.parse_imperative_expr()?;
            Ok(Stmt::TrySendPipe { value, pipe, result: None })
        } else if self.check(&TokenType::Receive) {
            self.advance(); // consume "Receive"

            let var = self.expect_identifier()?;

            if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            let pipe = self.parse_imperative_expr()?;
            Ok(Stmt::TryReceivePipe { var, pipe })
        } else {
            Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "send or receive".to_string() },
                span: self.current_span(),
            })
        }
    }

    /// Phase 54: Parse Stop statement
    /// Syntax: Stop handle.
    fn parse_stop_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Stop"

        let handle = self.parse_imperative_expr()?;

        Ok(Stmt::StopTask { handle })
    }

    /// Phase 54: Parse Select statement
    /// Syntax:
    /// Await the first of:
    ///     Receive x from pipe:
    ///         ...
    ///     After N seconds:
    ///         ...
    fn parse_select_statement(&mut self) -> ParseResult<Stmt<'a>> {
        use crate::ast::stmt::SelectBranch;

        self.advance(); // consume "Await"

        // Expect "the"
        if !self.check_article() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "the".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect "first"
        if !self.check_word("first") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "first".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect "of"
        if !self.check_preposition_is("of") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance();

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance();

        // Parse branches
        let mut branches = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let branch = self.parse_select_branch()?;
            branches.push(branch);
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        Ok(Stmt::Select { branches })
    }

    /// Phase 54: Parse a single select branch
    fn parse_select_branch(&mut self) -> ParseResult<crate::ast::stmt::SelectBranch<'a>> {
        use crate::ast::stmt::SelectBranch;

        if self.check(&TokenType::Receive) {
            self.advance(); // consume "Receive"

            let var = match &self.tokens[self.current].kind {
                TokenType::ProperName(sym) | TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                    let s = *sym;
                    self.advance();
                    s
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "variable name".to_string() },
                        span: self.current_span(),
                    });
                }
            };

            if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            let pipe = self.parse_imperative_expr()?;

            // Expect colon
            if !self.check(&TokenType::Colon) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            // Parse body
            let body = self.parse_indented_block()?;

            Ok(SelectBranch::Receive { var, pipe, body })
        } else if self.check_word("after") {
            self.advance(); // consume "After"

            let milliseconds = self.parse_imperative_expr()?;

            // Skip "seconds" or "milliseconds" if present
            if self.check_word("seconds") || self.check_word("milliseconds") {
                self.advance();
            }

            // Expect colon
            if !self.check(&TokenType::Colon) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance();

            // Parse body
            let body = self.parse_indented_block()?;

            Ok(SelectBranch::Timeout { milliseconds, body })
        } else {
            Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "Receive or After".to_string() },
                span: self.current_span(),
            })
        }
    }

    /// Phase 54: Parse an indented block of statements
    fn parse_indented_block(&mut self) -> ParseResult<crate::ast::stmt::Block<'a>> {
        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance();

        let mut stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let block = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(stmts.into_iter());

        Ok(block)
    }

    fn parse_give_statement(&mut self) -> ParseResult<Stmt<'a>> {
        use crate::context::OwnershipState;

        self.advance(); // consume "Give"

        // Parse the object being given: "x" or "the data"
        let object = self.parse_imperative_expr()?;

        // Expect "to" preposition
        if !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse the recipient: "processor" or "the console"
        let recipient = self.parse_imperative_expr()?;

        // CRITICAL: Mark the object as Moved in the ownership tracker
        if let Expr::Identifier(sym) = *object {
            if let Some(ctx) = self.context.as_mut() {
                let name = self.interner.resolve(sym);
                ctx.set_ownership(name, OwnershipState::Moved);
            }
        }

        Ok(Stmt::Give { object, recipient })
    }

    fn parse_show_statement(&mut self) -> ParseResult<Stmt<'a>> {
        use crate::context::OwnershipState;

        self.advance(); // consume "Show"

        // Parse the object being shown - use parse_condition to support
        // comparisons (x is less than y) and boolean operators (a and b)
        let object = self.parse_condition()?;

        // Optional "to" preposition - if not present, default to "show" function
        let recipient = if self.check_preposition_is("to") {
            self.advance(); // consume "to"

            // Phase 10: "Show x to console." or "Show x to the console."
            // is idiomatic for printing to stdout - use default show function
            if self.check_article() {
                self.advance(); // skip "the"
            }
            if self.check(&TokenType::Console) {
                self.advance(); // consume "console"
                let show_sym = self.interner.intern("show");
                self.ctx.alloc_imperative_expr(Expr::Identifier(show_sym))
            } else {
                // Parse the recipient: custom function
                self.parse_imperative_expr()?
            }
        } else {
            // Default recipient: the runtime "show" function
            let show_sym = self.interner.intern("show");
            self.ctx.alloc_imperative_expr(Expr::Identifier(show_sym))
        };

        // Mark the object as Borrowed (NOT Moved - still accessible)
        if let Expr::Identifier(sym) = *object {
            if let Some(ctx) = self.context.as_mut() {
                let name = self.interner.resolve(sym);
                ctx.set_ownership(name, OwnershipState::Borrowed);
            }
        }

        Ok(Stmt::Show { object, recipient })
    }

    /// Phase 43D: Parse Push statement for collection operations
    /// Syntax: Push x to items.
    fn parse_push_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Push"

        // Parse the value being pushed
        let value = self.parse_imperative_expr()?;

        // Expect "to" preposition
        if !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse the collection
        let collection = self.parse_imperative_expr()?;

        Ok(Stmt::Push { value, collection })
    }

    /// Phase 43D: Parse Pop statement for collection operations
    /// Syntax: Pop from items. OR Pop from items into y.
    fn parse_pop_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Pop"

        // Expect "from" - can be keyword token or preposition
        if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "from"

        // Parse the collection
        let collection = self.parse_imperative_expr()?;

        // Check for optional "into" binding (can be Into keyword or preposition)
        let into = if self.check(&TokenType::Into) || self.check_preposition_is("into") {
            self.advance(); // consume "into"

            // Parse variable name
            if let TokenType::Noun(sym) | TokenType::ProperName(sym) = &self.peek().kind {
                let sym = *sym;
                self.advance();
                Some(sym)
            } else if let Some(token) = self.tokens.get(self.current) {
                // Also handle identifier-like tokens
                let sym = token.lexeme;
                self.advance();
                Some(sym)
            } else {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedIdentifier,
                    span: self.current_span(),
                });
            }
        } else {
            None
        };

        Ok(Stmt::Pop { collection, into })
    }

    /// Parse Add statement for Set insertion
    /// Syntax: Add x to set.
    fn parse_add_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Add"

        // Parse the value to add
        let value = self.parse_imperative_expr()?;

        // Expect "to" preposition
        if !self.check_preposition_is("to") && !self.check(&TokenType::To) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse the collection expression
        let collection = self.parse_imperative_expr()?;

        Ok(Stmt::Add { value, collection })
    }

    /// Parse Remove statement for Set deletion
    /// Syntax: Remove x from set.
    fn parse_remove_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Remove"

        // Parse the value to remove
        let value = self.parse_imperative_expr()?;

        // Expect "from" preposition
        if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "from"

        // Parse the collection expression
        let collection = self.parse_imperative_expr()?;

        Ok(Stmt::Remove { value, collection })
    }

    /// Phase 10: Parse Read statement for console/file input
    /// Syntax: Read <var> from the console.
    ///         Read <var> from file <path>.
    fn parse_read_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Read"

        // Get the variable name
        let var = self.expect_identifier()?;

        // Expect "from" preposition
        if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "from"

        // Skip optional article "the"
        if self.check_article() {
            self.advance();
        }

        // Determine source: console or file
        let source = if self.check(&TokenType::Console) {
            self.advance(); // consume "console"
            ReadSource::Console
        } else if self.check(&TokenType::File) {
            self.advance(); // consume "file"
            let path = self.parse_imperative_expr()?;
            ReadSource::File(path)
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "console or file".to_string() },
                span: self.current_span(),
            });
        };

        Ok(Stmt::ReadFrom { var, source })
    }

    /// Phase 10: Parse Write statement for file output
    /// Syntax: Write <content> to file <path>.
    fn parse_write_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Write"

        // Parse the content expression
        let content = self.parse_imperative_expr()?;

        // Expect "to" preposition
        if !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Expect "file" keyword
        if !self.check(&TokenType::File) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "file".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "file"

        // Parse the path expression
        let path = self.parse_imperative_expr()?;

        Ok(Stmt::WriteFile { content, path })
    }

    /// Phase 8.5: Parse Zone statement for memory arena blocks
    /// Syntax variants:
    ///   - Inside a new zone called "Scratch":
    ///   - Inside a zone called "Buffer" of size 1 MB:
    ///   - Inside a zone called "Data" mapped from "file.bin":
    fn parse_zone_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Inside"

        // Optional article "a"
        if self.check_article() {
            self.advance();
        }

        // Optional "new"
        if self.check(&TokenType::New) {
            self.advance();
        }

        // Expect "zone"
        if !self.check(&TokenType::Zone) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "zone".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "zone"

        // Expect "called"
        if !self.check(&TokenType::Called) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "called".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "called"

        // Parse zone name (can be string literal or identifier)
        let name = match &self.peek().kind {
            TokenType::StringLiteral(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            TokenType::ProperName(sym) | TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                // Try to use the lexeme directly as an identifier
                let token = self.peek().clone();
                self.advance();
                token.lexeme
            }
        };

        let mut capacity = None;
        let mut source_file = None;

        // Check for "mapped from" (file-backed zone)
        if self.check(&TokenType::Mapped) {
            self.advance(); // consume "mapped"

            // Expect "from"
            if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume "from"

            // Parse file path (must be string literal)
            if let TokenType::StringLiteral(path) = &self.peek().kind {
                source_file = Some(*path);
                self.advance();
            } else {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "file path string".to_string() },
                    span: self.current_span(),
                });
            }
        }
        // Check for "of size N Unit" (sized heap zone)
        else if self.check_of_preposition() {
            self.advance(); // consume "of"

            // Expect "size"
            if !self.check(&TokenType::Size) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "size".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume "size"

            // Parse size number
            let size_value = match &self.peek().kind {
                TokenType::Number(sym) => {
                    let num_str = self.interner.resolve(*sym);
                    let val = num_str.replace('_', "").parse::<usize>().unwrap_or(0);
                    self.advance();
                    val
                }
                TokenType::Cardinal(n) => {
                    let val = *n as usize;
                    self.advance();
                    val
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedNumber,
                        span: self.current_span(),
                    });
                }
            };

            // Parse unit (KB, MB, GB, or B)
            let unit_multiplier = self.parse_size_unit()?;
            capacity = Some(size_value * unit_multiplier);
        }

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse body statements
        let mut body_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            body_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(Stmt::Zone { name, capacity, source_file, body })
    }

    /// Parse size unit (B, KB, MB, GB) and return multiplier
    fn parse_size_unit(&mut self) -> ParseResult<usize> {
        let token = self.peek().clone();
        let unit_str = self.interner.resolve(token.lexeme).to_uppercase();
        self.advance();

        match unit_str.as_str() {
            "B" | "BYTES" | "BYTE" => Ok(1),
            "KB" | "KILOBYTE" | "KILOBYTES" => Ok(1024),
            "MB" | "MEGABYTE" | "MEGABYTES" => Ok(1024 * 1024),
            "GB" | "GIGABYTE" | "GIGABYTES" => Ok(1024 * 1024 * 1024),
            _ => Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword {
                    keyword: "size unit (B, KB, MB, GB)".to_string(),
                },
                span: token.span,
            }),
        }
    }

    /// Phase 9: Parse concurrent execution block (async, I/O-bound)
    ///
    /// Syntax:
    /// ```logos
    /// Attempt all of the following:
    ///     Call fetch_user with id.
    ///     Call fetch_orders with id.
    /// ```
    fn parse_concurrent_block(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Attempt"

        // Expect "all"
        if !self.check(&TokenType::All) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "all".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "all"

        // Expect "of" (preposition)
        if !self.check_of_preposition() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "of"

        // Expect "the"
        if !self.check_article() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "the".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "the"

        // Expect "following"
        if !self.check(&TokenType::Following) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "following".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "following"

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse body statements
        let mut task_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            task_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let tasks = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(task_stmts.into_iter());

        Ok(Stmt::Concurrent { tasks })
    }

    /// Phase 9: Parse parallel execution block (CPU-bound)
    ///
    /// Syntax:
    /// ```logos
    /// Simultaneously:
    ///     Call compute_hash with data1.
    ///     Call compute_hash with data2.
    /// ```
    fn parse_parallel_block(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Simultaneously"

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse body statements
        let mut task_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            task_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let tasks = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(task_stmts.into_iter());

        Ok(Stmt::Parallel { tasks })
    }

    /// Phase 33: Parse Inspect statement for pattern matching
    /// Syntax: Inspect target:
    ///             If it is a Variant [(bindings)]:
    ///                 body...
    ///             Otherwise:
    ///                 body...
    fn parse_inspect_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Inspect"

        // Parse target expression
        let target = self.parse_imperative_expr()?;

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        let mut arms = Vec::new();
        let mut has_otherwise = false;

        // Parse match arms until dedent
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            if self.check(&TokenType::Otherwise) {
                // Parse "Otherwise:" default arm
                self.advance(); // consume "Otherwise"

                if !self.check(&TokenType::Colon) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume ":"

                // Handle both inline (Otherwise: stmt.) and block body
                let body_stmts = if self.check(&TokenType::Indent) {
                    self.advance(); // consume Indent
                    let mut stmts = Vec::new();
                    while !self.check(&TokenType::Dedent) && !self.is_at_end() {
                        let stmt = self.parse_statement()?;
                        stmts.push(stmt);
                        if self.check(&TokenType::Period) {
                            self.advance();
                        }
                    }
                    if self.check(&TokenType::Dedent) {
                        self.advance();
                    }
                    stmts
                } else {
                    // Inline body: "Otherwise: Show x."
                    let stmt = self.parse_statement()?;
                    if self.check(&TokenType::Period) {
                        self.advance();
                    }
                    vec![stmt]
                };

                let body = self.ctx.stmts.expect("imperative arenas not initialized")
                    .alloc_slice(body_stmts.into_iter());

                arms.push(MatchArm { enum_name: None, variant: None, bindings: vec![], body });
                has_otherwise = true;
                break;
            }

            if self.check(&TokenType::If) {
                // Parse "If it is a VariantName [(bindings)]:"
                let arm = self.parse_match_arm()?;
                arms.push(arm);
            } else if self.check(&TokenType::When) || self.check_word("When") {
                // Parse "When Variant [(bindings)]:" (concise syntax)
                let arm = self.parse_when_arm()?;
                arms.push(arm);
            } else if self.check(&TokenType::Newline) {
                // Skip newlines between arms
                self.advance();
            } else {
                // Skip unexpected tokens
                self.advance();
            }
        }

        // Consume final dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        Ok(Stmt::Inspect { target, arms, has_otherwise })
    }

    /// Parse a single match arm: "If it is a Variant [(field: binding)]:"
    fn parse_match_arm(&mut self) -> ParseResult<MatchArm<'a>> {
        self.advance(); // consume "If"

        // Expect "it"
        if !self.check_word("it") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "it".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "it"

        // Expect "is"
        if !self.check(&TokenType::Is) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "is".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "is"

        // Consume article "a" or "an"
        if self.check_article() {
            self.advance();
        }

        // Get variant name
        let variant = self.expect_identifier()?;

        // Look up the enum name for this variant
        let enum_name = self.find_variant(variant);

        // Optional: "(field)" or "(field: binding)" or "(f1, f2: b2)"
        let bindings = if self.check(&TokenType::LParen) {
            self.parse_pattern_bindings()?
        } else {
            vec![]
        };

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Expect indent
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse body statements
        let mut body_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            body_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(MatchArm { enum_name, variant: Some(variant), bindings, body })
    }

    /// Parse a concise match arm: "When Variant [(bindings)]:" or "When Variant: stmt."
    fn parse_when_arm(&mut self) -> ParseResult<MatchArm<'a>> {
        self.advance(); // consume "When"

        // Get variant name
        let variant = self.expect_identifier()?;

        // Look up the enum name and variant definition for this variant
        let (enum_name, variant_fields) = self.type_registry
            .as_ref()
            .and_then(|r| r.find_variant(variant).map(|(enum_name, vdef)| {
                let fields: Vec<_> = vdef.fields.iter().map(|f| f.name).collect();
                (Some(enum_name), fields)
            }))
            .unwrap_or((None, vec![]));

        // Optional: "(binding)" or "(b1, b2)" - positional bindings
        let bindings = if self.check(&TokenType::LParen) {
            let raw_bindings = self.parse_when_bindings()?;
            // Map positional bindings to actual field names
            raw_bindings.into_iter().enumerate().map(|(i, binding)| {
                let field = variant_fields.get(i).copied().unwrap_or(binding);
                (field, binding)
            }).collect()
        } else {
            vec![]
        };

        // Expect colon
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ":"

        // Handle both inline body (When Variant: stmt.) and block body
        let body_stmts = if self.check(&TokenType::Indent) {
            self.advance(); // consume Indent
            let mut stmts = Vec::new();
            while !self.check(&TokenType::Dedent) && !self.is_at_end() {
                let stmt = self.parse_statement()?;
                stmts.push(stmt);
                if self.check(&TokenType::Period) {
                    self.advance();
                }
            }
            if self.check(&TokenType::Dedent) {
                self.advance();
            }
            stmts
        } else {
            // Inline body: "When Red: Show x."
            let stmt = self.parse_statement()?;
            if self.check(&TokenType::Period) {
                self.advance();
            }
            vec![stmt]
        };

        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(MatchArm { enum_name, variant: Some(variant), bindings, body })
    }

    /// Parse concise When bindings: "(r)" or "(w, h)" - just binding variable names
    fn parse_when_bindings(&mut self) -> ParseResult<Vec<Symbol>> {
        self.advance(); // consume '('
        let mut bindings = Vec::new();

        loop {
            let binding = self.expect_identifier()?;
            bindings.push(binding);

            if !self.check(&TokenType::Comma) {
                break;
            }
            self.advance(); // consume ','
        }

        if !self.check(&TokenType::RParen) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ')'

        Ok(bindings)
    }

    /// Parse pattern bindings: "(field)" or "(field: binding)" or "(f1, f2: b2)"
    fn parse_pattern_bindings(&mut self) -> ParseResult<Vec<(Symbol, Symbol)>> {
        self.advance(); // consume '('
        let mut bindings = Vec::new();

        loop {
            let field = self.expect_identifier()?;
            let binding = if self.check(&TokenType::Colon) {
                self.advance(); // consume ":"
                self.expect_identifier()?
            } else {
                field // field name = binding name
            };
            bindings.push((field, binding));

            if !self.check(&TokenType::Comma) {
                break;
            }
            self.advance(); // consume ','
        }

        if !self.check(&TokenType::RParen) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ')'

        Ok(bindings)
    }

    /// Parse constructor fields: "with field1 value1 [and field2 value2]..."
    /// Example: "with radius 10" or "with x 10 and y 20"
    /// Used for both variant constructors and struct initialization
    fn parse_constructor_fields(&mut self) -> ParseResult<Vec<(Symbol, &'a Expr<'a>)>> {
        use crate::ast::Expr;
        let mut fields = Vec::new();

        // Consume "with"
        self.advance();

        loop {
            // Parse field name
            let field_name = self.expect_identifier()?;

            // Parse field value expression
            let value = self.parse_imperative_expr()?;

            fields.push((field_name, value));

            // Check for "and" to continue
            if self.check(&TokenType::And) {
                self.advance(); // consume "and"
                continue;
            }
            break;
        }

        Ok(fields)
    }

    /// Alias for variant constructors (backwards compat)
    fn parse_variant_constructor_fields(&mut self) -> ParseResult<Vec<(Symbol, &'a Expr<'a>)>> {
        self.parse_constructor_fields()
    }

    /// Alias for struct initialization
    fn parse_struct_init_fields(&mut self) -> ParseResult<Vec<(Symbol, &'a Expr<'a>)>> {
        self.parse_constructor_fields()
    }

    /// Phase 34: Parse generic type arguments for constructor instantiation
    /// Parses "of Int" or "of Int and Text" after a generic type name
    /// Returns empty Vec for non-generic types
    fn parse_generic_type_args(&mut self, type_name: Symbol) -> ParseResult<Vec<Symbol>> {
        // Only parse type args if the type is a known generic
        if !self.is_generic_type(type_name) {
            return Ok(vec![]);
        }

        // Expect "of" preposition
        if !self.check_preposition_is("of") {
            return Ok(vec![]);  // Generic type without arguments - will use defaults
        }
        self.advance(); // consume "of"

        let mut type_args = Vec::new();
        loop {
            // Parse type argument (e.g., "Int", "Text", "User")
            let type_arg = self.expect_identifier()?;
            type_args.push(type_arg);

            // Check for "and" or "to" to continue (for multi-param generics like "Map of Text to Int")
            if self.check(&TokenType::And) || self.check_to_preposition() {
                self.advance(); // consume separator
                continue;
            }
            break;
        }

        Ok(type_args)
    }

    /// Skip type definition content until next block header
    /// Used for TypeDef blocks (## A Point has:, ## A Color is one of:)
    /// The actual parsing is done by DiscoveryPass
    fn skip_type_def_content(&mut self) {
        while !self.is_at_end() {
            // Stop at next block header
            if matches!(
                self.tokens.get(self.current),
                Some(Token { kind: TokenType::BlockHeader { .. }, .. })
            ) {
                break;
            }
            self.advance();
        }
    }

    /// Phase 32: Parse function definition after `## To` header
    /// Phase 32/38: Parse function definition
    /// Syntax: [To] [native] name (a: Type) [and (b: Type)] [-> ReturnType]
    ///         body statements... (only if not native)
    fn parse_function_def(&mut self) -> ParseResult<Stmt<'a>> {
        // Consume "To" if present (when called from parse_statement)
        if self.check(&TokenType::To) || self.check_preposition_is("to") {
            self.advance();
        }

        // Phase 38: Check for native modifier
        let is_native = if self.check(&TokenType::Native) {
            self.advance(); // consume "native"
            true
        } else {
            false
        };

        // Parse function name (first identifier after ## To [native])
        let name = self.expect_identifier()?;

        // Parse parameters: (name: Type) groups separated by "and", or comma-separated in one group
        let mut params = Vec::new();
        while self.check(&TokenType::LParen) {
            self.advance(); // consume (

            // Parse parameters in this group (possibly comma-separated)
            loop {
                let param_name = self.expect_identifier()?;

                // Expect colon
                if !self.check(&TokenType::Colon) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume :

                // Phase 38: Parse full type expression instead of simple identifier
                let param_type_expr = self.parse_type_expression()?;
                let param_type = self.ctx.alloc_type_expr(param_type_expr);

                params.push((param_name, param_type));

                // Check for comma (more params in this group) or ) (end of group)
                if self.check(&TokenType::Comma) {
                    self.advance(); // consume ,
                    continue;
                }
                break;
            }

            // Expect )
            if !self.check(&TokenType::RParen) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                    span: self.current_span(),
                });
            }
            self.advance(); // consume )

            // Check for "and", preposition, or "from" between parameter groups
            // Allows: "## To withdraw (amount: Int) from (balance: Int)"
            if self.check_word("and") || self.check_preposition() || self.check(&TokenType::From) {
                self.advance();
            }
        }

        // Phase 38: Parse optional return type -> Type
        let return_type = if self.check(&TokenType::Arrow) {
            self.advance(); // consume ->
            let ret_type_expr = self.parse_type_expression()?;
            Some(self.ctx.alloc_type_expr(ret_type_expr))
        } else {
            None
        };

        // Phase 38: Native functions have no body
        if is_native {
            // Consume trailing period or newline if present
            if self.check(&TokenType::Period) {
                self.advance();
            }
            if self.check(&TokenType::Newline) {
                self.advance();
            }

            // Return with empty body
            let empty_body = self.ctx.stmts.expect("imperative arenas not initialized")
                .alloc_slice(std::iter::empty());

            return Ok(Stmt::FunctionDef {
                name,
                params,
                body: empty_body,
                return_type,
                is_native: true,
            });
        }

        // Non-native: expect colon after parameter list / return type
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ":".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume :

        // Expect indent for function body
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance(); // consume Indent

        // Parse body statements
        let mut body_stmts = Vec::new();
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            // Skip newlines between statements
            if self.check(&TokenType::Newline) {
                self.advance();
                continue;
            }
            // Stop if we hit another block header
            if matches!(self.peek().kind, TokenType::BlockHeader { .. }) {
                break;
            }
            let stmt = self.parse_statement()?;
            body_stmts.push(stmt);
            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        // Consume dedent if present
        if self.check(&TokenType::Dedent) {
            self.advance();
        }

        // Allocate body in arena
        let body = self.ctx.stmts.expect("imperative arenas not initialized")
            .alloc_slice(body_stmts.into_iter());

        Ok(Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            is_native: false,
        })
    }

    /// Parse a primary expression (literal, identifier, index, slice, list, etc.)
    fn parse_primary_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        use crate::ast::{Expr, Literal};

        let token = self.peek().clone();
        match &token.kind {
            // Phase 31: Constructor expression "new TypeName" or "a new TypeName"
            // Phase 33: Extended for variant constructors "new Circle with radius 10"
            // Phase 34: Extended for generic instantiation "new Box of Int"
            TokenType::New => {
                self.advance(); // consume "new"
                let base_type_name = self.expect_identifier()?;

                // Phase 36: Check for "from Module" qualification
                let type_name = if self.check(&TokenType::From) {
                    self.advance(); // consume "from"
                    let module_name = self.expect_identifier()?;
                    let module_str = self.interner.resolve(module_name);
                    let base_str = self.interner.resolve(base_type_name);
                    let qualified = format!("{}::{}", module_str, base_str);
                    self.interner.intern(&qualified)
                } else {
                    base_type_name
                };

                // Phase 33: Check if this is a variant constructor
                if let Some(enum_name) = self.find_variant(type_name) {
                    // Parse optional "with field value" pairs
                    let fields = if self.check_word("with") {
                        self.parse_variant_constructor_fields()?
                    } else {
                        vec![]
                    };
                    let base = self.ctx.alloc_imperative_expr(Expr::NewVariant {
                        enum_name,
                        variant: type_name,
                        fields,
                    });
                    return self.parse_field_access_chain(base);
                }

                // Phase 34: Parse generic type arguments "of Int" or "of Int and Text"
                let type_args = self.parse_generic_type_args(type_name)?;

                // Parse optional "with field value" pairs for struct initialization
                let init_fields = if self.check_word("with") {
                    self.parse_struct_init_fields()?
                } else {
                    vec![]
                };

                let base = self.ctx.alloc_imperative_expr(Expr::New { type_name, type_args, init_fields });
                return self.parse_field_access_chain(base);
            }

            // Phase 31: Handle "a new TypeName" pattern OR single-letter identifier
            // Phase 33: Extended for variant constructors "a new Circle with radius 10"
            // Phase 34: Extended for generic instantiation "a new Box of Int"
            TokenType::Article(_) => {
                // Phase 48: Check if followed by Manifest or Chunk token
                // Pattern: "the manifest of Zone" or "the chunk at N in Zone"
                if let Some(next) = self.tokens.get(self.current + 1) {
                    if matches!(next.kind, TokenType::Manifest) {
                        self.advance(); // consume "the"
                        // Delegate to Manifest handling
                        return self.parse_primary_expr();
                    }
                    if matches!(next.kind, TokenType::Chunk) {
                        self.advance(); // consume "the"
                        // Delegate to Chunk handling
                        return self.parse_primary_expr();
                    }
                }
                // Check if followed by New token
                if let Some(next) = self.tokens.get(self.current + 1) {
                    if matches!(next.kind, TokenType::New) {
                        self.advance(); // consume article "a"/"an"
                        self.advance(); // consume "new"
                        let base_type_name = self.expect_identifier()?;

                        // Phase 36: Check for "from Module" qualification
                        let type_name = if self.check(&TokenType::From) {
                            self.advance(); // consume "from"
                            let module_name = self.expect_identifier()?;
                            let module_str = self.interner.resolve(module_name);
                            let base_str = self.interner.resolve(base_type_name);
                            let qualified = format!("{}::{}", module_str, base_str);
                            self.interner.intern(&qualified)
                        } else {
                            base_type_name
                        };

                        // Phase 33: Check if this is a variant constructor
                        if let Some(enum_name) = self.find_variant(type_name) {
                            // Parse optional "with field value" pairs
                            let fields = if self.check_word("with") {
                                self.parse_variant_constructor_fields()?
                            } else {
                                vec![]
                            };
                            let base = self.ctx.alloc_imperative_expr(Expr::NewVariant {
                                enum_name,
                                variant: type_name,
                                fields,
                            });
                            return self.parse_field_access_chain(base);
                        }

                        // Phase 34: Parse generic type arguments "of Int" or "of Int and Text"
                        let type_args = self.parse_generic_type_args(type_name)?;

                        // Parse optional "with field value" pairs for struct initialization
                        let init_fields = if self.check_word("with") {
                            self.parse_struct_init_fields()?
                        } else {
                            vec![]
                        };

                        let base = self.ctx.alloc_imperative_expr(Expr::New { type_name, type_args, init_fields });
                        return self.parse_field_access_chain(base);
                    }
                }
                // Phase 32: Treat as identifier (single-letter var like "a", "b")
                let sym = token.lexeme;
                self.advance();
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                return self.parse_field_access_chain(base);
            }

            // Index access: "item N of collection" or "item i of collection"
            TokenType::Item => {
                self.advance(); // consume "item"

                // Grand Challenge: Parse index as expression (number, identifier, or parenthesized)
                let index = if let TokenType::Number(sym) = &self.peek().kind {
                    // Literal number - check for zero index at compile time
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    let index_val = num_str.parse::<i64>().unwrap_or(0);

                    // Index 0 Guard: LOGOS uses 1-based indexing
                    if index_val == 0 {
                        return Err(ParseError {
                            kind: ParseErrorKind::ZeroIndex,
                            span: self.current_span(),
                        });
                    }

                    self.ctx.alloc_imperative_expr(
                        Expr::Literal(crate::ast::Literal::Number(index_val))
                    )
                } else if self.check(&TokenType::LParen) {
                    // Parenthesized expression like (mid + 1)
                    self.advance(); // consume '('
                    let inner = self.parse_imperative_expr()?;
                    if !self.check(&TokenType::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume ')'
                    inner
                } else if let TokenType::StringLiteral(sym) = self.peek().kind {
                    // Phase 57B: String literal key for Map access like item "iron" of prices
                    let sym = sym;
                    self.advance();
                    self.ctx.alloc_imperative_expr(Expr::Literal(crate::ast::Literal::Text(sym)))
                } else if !self.check_preposition_is("of") {
                    // Variable identifier like i, j, idx (any token that's not "of")
                    let sym = self.peek().lexeme;
                    self.advance();
                    self.ctx.alloc_imperative_expr(Expr::Identifier(sym))
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedExpression,
                        span: self.current_span(),
                    });
                };

                // Expect "of"
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                // Parse collection as primary expression (identifier or field chain)
                // Using primary_expr instead of imperative_expr prevents consuming operators
                let collection = self.parse_primary_expr()?;

                Ok(self.ctx.alloc_imperative_expr(Expr::Index {
                    collection,
                    index,
                }))
            }

            // Slice access: "items N through M of collection"
            // OR variable named "items" - disambiguate by checking if next token starts an expression
            TokenType::Items => {
                // Peek ahead to determine if this is slice syntax or variable usage
                // Slice syntax: "items" followed by number or paren (clear indicators of index)
                // Variable: "items" followed by something else (operator, dot, etc.)
                let is_slice_syntax = if let Some(next) = self.tokens.get(self.current + 1) {
                    matches!(next.kind, TokenType::Number(_) | TokenType::LParen)
                } else {
                    false
                };

                if !is_slice_syntax {
                    // Treat "items" as a variable identifier
                    let sym = token.lexeme;
                    self.advance();
                    let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                    return self.parse_field_access_chain(base);
                }

                self.advance(); // consume "items"

                // Grand Challenge: Parse start index as expression (number, identifier, or parenthesized)
                let start = if let TokenType::Number(sym) = &self.peek().kind {
                    // Literal number - check for zero index at compile time
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    let start_val = num_str.parse::<i64>().unwrap_or(0);

                    // Index 0 Guard for start
                    if start_val == 0 {
                        return Err(ParseError {
                            kind: ParseErrorKind::ZeroIndex,
                            span: self.current_span(),
                        });
                    }

                    self.ctx.alloc_imperative_expr(
                        Expr::Literal(crate::ast::Literal::Number(start_val))
                    )
                } else if self.check(&TokenType::LParen) {
                    // Parenthesized expression like (mid + 1)
                    self.advance(); // consume '('
                    let inner = self.parse_imperative_expr()?;
                    if !self.check(&TokenType::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume ')'
                    inner
                } else if !self.check_preposition_is("through") {
                    // Variable identifier like mid, idx
                    let sym = self.peek().lexeme;
                    self.advance();
                    self.ctx.alloc_imperative_expr(Expr::Identifier(sym))
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedExpression,
                        span: self.current_span(),
                    });
                };

                // Expect "through"
                if !self.check_preposition_is("through") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "through".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "through"

                // Grand Challenge: Parse end index as expression (number, identifier, or parenthesized)
                let end = if let TokenType::Number(sym) = &self.peek().kind {
                    // Literal number - check for zero index at compile time
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    let end_val = num_str.parse::<i64>().unwrap_or(0);

                    // Index 0 Guard for end
                    if end_val == 0 {
                        return Err(ParseError {
                            kind: ParseErrorKind::ZeroIndex,
                            span: self.current_span(),
                        });
                    }

                    self.ctx.alloc_imperative_expr(
                        Expr::Literal(crate::ast::Literal::Number(end_val))
                    )
                } else if self.check(&TokenType::LParen) {
                    // Parenthesized expression like (mid + 1)
                    self.advance(); // consume '('
                    let inner = self.parse_imperative_expr()?;
                    if !self.check(&TokenType::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume ')'
                    inner
                } else if !self.check_preposition_is("of") {
                    // Variable identifier like n, length
                    let sym = self.peek().lexeme;
                    self.advance();
                    self.ctx.alloc_imperative_expr(Expr::Identifier(sym))
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedExpression,
                        span: self.current_span(),
                    });
                };

                // "of collection" is now optional - collection can be inferred from context
                // (e.g., "items 1 through mid" when items is the local variable)
                let collection = if self.check_preposition_is("of") {
                    self.advance(); // consume "of"
                    self.parse_imperative_expr()?
                } else {
                    // The variable is the collection itself (already consumed as "items")
                    // Re-intern "items" to use as the collection identifier
                    let items_sym = self.interner.intern("items");
                    self.ctx.alloc_imperative_expr(Expr::Identifier(items_sym))
                };

                Ok(self.ctx.alloc_imperative_expr(Expr::Slice {
                    collection,
                    start,
                    end,
                }))
            }

            // List literal: [1, 2, 3]
            TokenType::LBracket => {
                self.advance(); // consume "["

                let mut items = Vec::new();
                if !self.check(&TokenType::RBracket) {
                    loop {
                        items.push(self.parse_imperative_expr()?);
                        if !self.check(&TokenType::Comma) {
                            break;
                        }
                        self.advance(); // consume ","
                    }
                }

                if !self.check(&TokenType::RBracket) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "]".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "]"

                // Check for typed empty list: [] of Int
                if items.is_empty() && self.check_word("of") {
                    self.advance(); // consume "of"
                    let type_name = self.expect_identifier()?;
                    // Generate: Seq::<Type>::default()
                    let seq_sym = self.interner.intern("Seq");
                    return Ok(self.ctx.alloc_imperative_expr(Expr::New {
                        type_name: seq_sym,
                        type_args: vec![type_name],
                        init_fields: vec![],
                    }));
                }

                Ok(self.ctx.alloc_imperative_expr(Expr::List(items)))
            }

            TokenType::Number(sym) => {
                self.advance();
                let num_str = self.interner.resolve(*sym);
                // Check if it's a float (contains decimal point)
                if num_str.contains('.') {
                    let num = num_str.parse::<f64>().unwrap_or(0.0);
                    Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Float(num))))
                } else {
                    let num = num_str.parse::<i64>().unwrap_or(0);
                    Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Number(num))))
                }
            }

            // Phase 33: String literals
            TokenType::StringLiteral(sym) => {
                self.advance();
                Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Text(*sym))))
            }

            // Character literals
            TokenType::CharLiteral(sym) => {
                let char_str = self.interner.resolve(*sym);
                let ch = char_str.chars().next().unwrap_or('\0');
                self.advance();
                Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Char(ch))))
            }

            // Handle 'nothing' literal
            TokenType::Nothing => {
                self.advance();
                Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Nothing)))
            }

            // Phase 43D: Length expression: "length of items" or "length(items)"
            TokenType::Length => {
                let func_name = self.peek().lexeme;

                // Check for function call syntax: length(x)
                if self.tokens.get(self.current + 1)
                    .map(|t| matches!(t.kind, TokenType::LParen))
                    .unwrap_or(false)
                {
                    self.advance(); // consume "length"
                    return self.parse_call_expr(func_name);
                }

                self.advance(); // consume "length"

                // Expect "of" for natural syntax
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                let collection = self.parse_imperative_expr()?;
                Ok(self.ctx.alloc_imperative_expr(Expr::Length { collection }))
            }

            // Phase 43D: Copy expression: "copy of slice" or "copy(slice)"
            TokenType::Copy => {
                let func_name = self.peek().lexeme;

                // Check for function call syntax: copy(x)
                if self.tokens.get(self.current + 1)
                    .map(|t| matches!(t.kind, TokenType::LParen))
                    .unwrap_or(false)
                {
                    self.advance(); // consume "copy"
                    return self.parse_call_expr(func_name);
                }

                self.advance(); // consume "copy"

                // Expect "of" for natural syntax
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                let expr = self.parse_imperative_expr()?;
                Ok(self.ctx.alloc_imperative_expr(Expr::Copy { expr }))
            }

            // Phase 48: Manifest expression: "manifest of Zone"
            TokenType::Manifest => {
                self.advance(); // consume "manifest"

                // Expect "of"
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                let zone = self.parse_imperative_expr()?;
                Ok(self.ctx.alloc_imperative_expr(Expr::ManifestOf { zone }))
            }

            // Phase 48: Chunk expression: "chunk at N in Zone"
            TokenType::Chunk => {
                self.advance(); // consume "chunk"

                // Expect "at"
                if !self.check(&TokenType::At) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "at".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "at"

                let index = self.parse_imperative_expr()?;

                // Expect "in"
                if !self.check_preposition_is("in") && !self.check(&TokenType::In) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "in".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "in"

                let zone = self.parse_imperative_expr()?;
                Ok(self.ctx.alloc_imperative_expr(Expr::ChunkAt { index, zone }))
            }

            // Handle verbs in expression context:
            // - "empty" is a literal Nothing
            // - Other verbs can be function names (e.g., read, write)
            TokenType::Verb { lemma, .. } => {
                let word = self.interner.resolve(*lemma).to_lowercase();
                if word == "empty" {
                    self.advance();
                    return Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Nothing)));
                }
                // Phase 38: Allow verbs to be used as function calls
                let sym = token.lexeme;
                self.advance();
                if self.check(&TokenType::LParen) {
                    return self.parse_call_expr(sym);
                }
                // Treat as identifier reference
                self.verify_identifier_access(sym)?;
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                self.parse_field_access_chain(base)
            }

            // Phase 38: Adverbs as identifiers (e.g., "now" for time functions)
            TokenType::TemporalAdverb(_) | TokenType::ScopalAdverb(_) | TokenType::Adverb(_) => {
                let sym = token.lexeme;
                self.advance();
                if self.check(&TokenType::LParen) {
                    return self.parse_call_expr(sym);
                }
                // Treat as identifier reference (e.g., "Let t be now.")
                self.verify_identifier_access(sym)?;
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                self.parse_field_access_chain(base)
            }

            // Phase 10: IO keywords as function calls (e.g., "read", "write", "file")
            // Phase 57: Add/Remove keywords as function calls
            TokenType::Read | TokenType::Write | TokenType::File | TokenType::Console |
            TokenType::Add | TokenType::Remove => {
                let sym = token.lexeme;
                self.advance();
                if self.check(&TokenType::LParen) {
                    return self.parse_call_expr(sym);
                }
                // Treat as identifier reference
                self.verify_identifier_access(sym)?;
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                self.parse_field_access_chain(base)
            }

            // Unified identifier handling - all identifier-like tokens get verified
            // First check for boolean/special literals before treating as variable
            TokenType::Noun(sym) | TokenType::ProperName(sym) | TokenType::Adjective(sym) => {
                let sym = *sym;
                let word = self.interner.resolve(sym);

                // Check for boolean literals
                if word == "true" {
                    self.advance();
                    return Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Boolean(true))));
                }
                if word == "false" {
                    self.advance();
                    return Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Boolean(false))));
                }

                // Check for 'empty' - treat as unit value for collections
                if word == "empty" {
                    self.advance();
                    return Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Nothing)));
                }

                // Don't verify as variable - might be a function call or enum variant
                self.advance();

                // Phase 32: Check for function call: identifier(args)
                if self.check(&TokenType::LParen) {
                    return self.parse_call_expr(sym);
                }

                // Phase 33: Check if this is a bare enum variant (e.g., "North" for Direction)
                if let Some(enum_name) = self.find_variant(sym) {
                    let base = self.ctx.alloc_imperative_expr(Expr::NewVariant {
                        enum_name,
                        variant: sym,
                        fields: vec![],
                    });
                    return self.parse_field_access_chain(base);
                }

                // Centralized verification for undefined/moved checks (only for variables)
                self.verify_identifier_access(sym)?;
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                // Phase 31: Check for field access via possessive
                self.parse_field_access_chain(base)
            }

            // Pronouns can be variable names in code context ("i", "it")
            TokenType::Pronoun { .. } => {
                let sym = token.lexeme;
                self.advance();
                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                // Phase 31: Check for field access via possessive
                self.parse_field_access_chain(base)
            }

            // Phase 49: CRDT keywords can be function names (Merge, Increase)
            TokenType::Merge | TokenType::Increase => {
                let sym = token.lexeme;
                self.advance();

                // Check for function call: Merge(args)
                if self.check(&TokenType::LParen) {
                    return self.parse_call_expr(sym);
                }

                let base = self.ctx.alloc_imperative_expr(Expr::Identifier(sym));
                self.parse_field_access_chain(base)
            }

            // Handle ambiguous tokens that might be identifiers
            TokenType::Ambiguous { primary, alternatives } => {
                let sym = match &**primary {
                    TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::ProperName(s) => Some(*s),
                    _ => alternatives.iter().find_map(|t| match t {
                        TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::ProperName(s) => Some(*s),
                        _ => None
                    })
                };

                if let Some(s) = sym {
                    self.verify_identifier_access(s)?;
                    self.advance();
                    let base = self.ctx.alloc_imperative_expr(Expr::Identifier(s));
                    // Phase 31: Check for field access via possessive
                    self.parse_field_access_chain(base)
                } else {
                    Err(ParseError {
                        kind: ParseErrorKind::ExpectedExpression,
                        span: self.current_span(),
                    })
                }
            }

            // Parenthesized expression: (expr) or Tuple literal: (expr, expr, ...)
            TokenType::LParen => {
                self.advance(); // consume '('
                let first = self.parse_imperative_expr()?;

                // Check if this is a tuple (has comma) or just grouping
                if self.check(&TokenType::Comma) {
                    // It's a tuple - parse remaining elements
                    let mut items = vec![first];
                    while self.check(&TokenType::Comma) {
                        self.advance(); // consume ","
                        items.push(self.parse_imperative_expr()?);
                    }

                    if !self.check(&TokenType::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume ')'

                    let base = self.ctx.alloc_imperative_expr(Expr::Tuple(items));
                    self.parse_field_access_chain(base)
                } else {
                    // Just a parenthesized expression
                    if !self.check(&TokenType::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume ')'
                    Ok(first)
                }
            }

            _ => {
                Err(ParseError {
                    kind: ParseErrorKind::ExpectedExpression,
                    span: self.current_span(),
                })
            }
        }
    }

    /// Parse a complete imperative expression including binary operators.
    /// Uses precedence climbing for correct associativity and precedence.
    fn parse_imperative_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        self.parse_additive_expr()
    }

    /// Parse additive expressions (+, -, combined with, union, intersection, contains) - left-to-right associative
    fn parse_additive_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        let mut left = self.parse_multiplicative_expr()?;

        loop {
            match &self.peek().kind {
                TokenType::Plus => {
                    self.advance();
                    let right = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left,
                        right,
                    });
                }
                TokenType::Minus => {
                    self.advance();
                    let right = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                        op: BinaryOpKind::Subtract,
                        left,
                        right,
                    });
                }
                // Phase 53: "combined with" for string concatenation
                TokenType::Combined => {
                    self.advance(); // consume "combined"
                    // Expect "with" (preposition)
                    if !self.check_preposition_is("with") {
                        return Err(ParseError {
                            kind: ParseErrorKind::ExpectedKeyword { keyword: "with".to_string() },
                            span: self.current_span(),
                        });
                    }
                    self.advance(); // consume "with"
                    let right = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                        op: BinaryOpKind::Concat,
                        left,
                        right,
                    });
                }
                // Set operations: union, intersection
                TokenType::Union => {
                    self.advance(); // consume "union"
                    let right = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::Union {
                        left,
                        right,
                    });
                }
                TokenType::Intersection => {
                    self.advance(); // consume "intersection"
                    let right = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::Intersection {
                        left,
                        right,
                    });
                }
                // Set membership: "set contains value"
                TokenType::Contains => {
                    self.advance(); // consume "contains"
                    let value = self.parse_multiplicative_expr()?;
                    left = self.ctx.alloc_imperative_expr(Expr::Contains {
                        collection: left,
                        value,
                    });
                }
                _ => break,
            }
        }

        Ok(left)
    }

    /// Parse unary expressions (currently just unary minus)
    fn parse_unary_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        use crate::ast::{Expr, Literal};

        if self.check(&TokenType::Minus) {
            self.advance(); // consume '-'
            let operand = self.parse_unary_expr()?; // recursive for --5
            // Implement as 0 - operand (no UnaryOp variant in Expr)
            return Ok(self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Number(0))),
                right: operand,
            }));
        }
        self.parse_primary_expr()
    }

    /// Parse multiplicative expressions (*, /, %) - left-to-right associative
    fn parse_multiplicative_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        let mut left = self.parse_unary_expr()?;

        loop {
            let op = match &self.peek().kind {
                TokenType::Star => {
                    self.advance();
                    BinaryOpKind::Multiply
                }
                TokenType::Slash => {
                    self.advance();
                    BinaryOpKind::Divide
                }
                TokenType::Percent => {
                    self.advance();
                    BinaryOpKind::Modulo
                }
                _ => break,
            };
            let right = self.parse_unary_expr()?;
            left = self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op,
                left,
                right,
            });
        }

        Ok(left)
    }

    /// Try to parse a binary operator (+, -, *, /)
    fn try_parse_binary_op(&mut self) -> Option<BinaryOpKind> {
        match &self.peek().kind {
            TokenType::Plus => {
                self.advance();
                Some(BinaryOpKind::Add)
            }
            TokenType::Minus => {
                self.advance();
                Some(BinaryOpKind::Subtract)
            }
            TokenType::Star => {
                self.advance();
                Some(BinaryOpKind::Multiply)
            }
            TokenType::Slash => {
                self.advance();
                Some(BinaryOpKind::Divide)
            }
            _ => None,
        }
    }

    /// Phase 32: Parse function call expression: f(x, y, ...)
    fn parse_call_expr(&mut self, function: Symbol) -> ParseResult<&'a Expr<'a>> {
        use crate::ast::Expr;

        self.advance(); // consume '('

        let mut args = Vec::new();
        if !self.check(&TokenType::RParen) {
            loop {
                args.push(self.parse_imperative_expr()?);
                if !self.check(&TokenType::Comma) {
                    break;
                }
                self.advance(); // consume ','
            }
        }

        if !self.check(&TokenType::RParen) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: ")".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume ')'

        Ok(self.ctx.alloc_imperative_expr(Expr::Call { function, args }))
    }

    /// Phase 31: Parse field access chain via possessive ('s) and bracket indexing
    /// Handles patterns like: p's x, p's x's y, items[1], items[i]'s field
    fn parse_field_access_chain(&mut self, base: &'a Expr<'a>) -> ParseResult<&'a Expr<'a>> {
        use crate::ast::Expr;

        let mut result = base;

        // Keep parsing field accesses and bracket indexing
        loop {
            if self.check(&TokenType::Possessive) {
                // Field access: p's x
                self.advance(); // consume "'s"
                let field = self.expect_identifier()?;
                result = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: result,
                    field,
                });
            } else if self.check(&TokenType::LBracket) {
                // Bracket indexing: items[1], items[i]
                self.advance(); // consume "["
                let index = self.parse_imperative_expr()?;

                if !self.check(&TokenType::RBracket) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "]".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "]"

                result = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: result,
                    index,
                });
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Centralized verification for identifier access in imperative mode.
    /// Checks for use-after-move errors on known variables.
    fn verify_identifier_access(&self, sym: Symbol) -> ParseResult<()> {
        if self.mode != ParserMode::Imperative {
            return Ok(());
        }

        use crate::context::OwnershipState;
        let name = self.interner.resolve(sym);

        // Check for Use-After-Move on variables we're tracking
        let ownership = self.context.as_ref()
            .and_then(|ctx| ctx.get_ownership(name));

        if ownership == Some(OwnershipState::Moved) {
            return Err(ParseError {
                kind: ParseErrorKind::UseAfterMove { name: name.to_string() },
                span: self.current_span(),
            });
        }

        Ok(())
    }

    fn expect_identifier(&mut self) -> ParseResult<Symbol> {
        let token = self.peek().clone();
        match &token.kind {
            // Standard identifiers
            TokenType::Noun(sym) | TokenType::ProperName(sym) | TokenType::Adjective(sym) => {
                self.advance();
                Ok(*sym)
            }
            // Verbs can be variable names in code context ("empty", "run", etc.)
            // Use raw lexeme to preserve original casing
            TokenType::Verb { .. } => {
                let sym = token.lexeme;
                self.advance();
                Ok(sym)
            }
            // Phase 32: Articles can be single-letter identifiers (a, an)
            TokenType::Article(_) => {
                let sym = token.lexeme;
                self.advance();
                Ok(sym)
            }
            // Overloaded tokens that are valid identifiers in code context
            TokenType::Pronoun { .. } |  // "i", "it"
            TokenType::Items |           // "items"
            TokenType::Item |            // "item"
            TokenType::Nothing |         // "nothing"
            // Phase 38: Adverbs can be function names (now, sleep, etc.)
            TokenType::TemporalAdverb(_) |
            TokenType::ScopalAdverb(_) |
            TokenType::Adverb(_) |
            // Phase 10: IO keywords can be function names (read, write, file, console)
            TokenType::Read |
            TokenType::Write |
            TokenType::File |
            TokenType::Console |
            // Phase 49: CRDT keywords can be function names (Merge, Increase)
            TokenType::Merge |
            TokenType::Increase |
            // Phase 54: "first", "second", etc. can be variable names
            // Phase 57: "add", "remove" can be function names
            TokenType::Add |
            TokenType::Remove |
            TokenType::First => {
                // Use the raw lexeme (interned string) as the symbol
                let sym = token.lexeme;
                self.advance();
                Ok(sym)
            }
            TokenType::Ambiguous { primary, .. } => {
                // For ambiguous tokens, extract symbol from primary
                let sym = match &**primary {
                    TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::ProperName(s) => *s,
                    TokenType::Verb { lemma, .. } => *lemma,
                    _ => token.lexeme,
                };
                self.advance();
                Ok(sym)
            }
            _ => Err(ParseError {
                kind: ParseErrorKind::ExpectedIdentifier,
                span: self.current_span(),
            }),
        }
    }

    fn consume_content_word_for_relative(&mut self) -> ParseResult<Symbol> {
        let t = self.advance().clone();
        match t.kind {
            TokenType::Noun(s) | TokenType::Adjective(s) => Ok(s),
            TokenType::ProperName(s) => Ok(s),
            TokenType::Verb { lemma, .. } => Ok(lemma),
            other => Err(ParseError {
                kind: ParseErrorKind::ExpectedContentWord { found: other },
                span: self.current_span(),
            }),
        }
    }

    fn check_modal(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Must
                | TokenType::Shall
                | TokenType::Should
                | TokenType::Can
                | TokenType::May
                | TokenType::Cannot
                | TokenType::Could
                | TokenType::Would
        )
    }

    fn check_pronoun(&self) -> bool {
        match &self.peek().kind {
            TokenType::Pronoun { case, .. } => {
                // In noun_priority_mode, possessive pronouns start NPs, not standalone objects
                if self.noun_priority_mode && matches!(case, Case::Possessive) {
                    return false;
                }
                true
            }
            TokenType::Ambiguous { primary, alternatives } => {
                // In noun_priority_mode, if there's a possessive alternative, prefer noun path
                if self.noun_priority_mode {
                    let has_possessive = matches!(**primary, TokenType::Pronoun { case: Case::Possessive, .. })
                        || alternatives.iter().any(|t| matches!(t, TokenType::Pronoun { case: Case::Possessive, .. }));
                    if has_possessive {
                        return false;
                    }
                }
                matches!(**primary, TokenType::Pronoun { .. })
                    || alternatives.iter().any(|t| matches!(t, TokenType::Pronoun { .. }))
            }
            _ => false,
        }
    }

    fn parse_atom(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // Handle Focus particles: "Only John loves Mary", "Even John ran"
        if self.check_focus() {
            return self.parse_focus();
        }

        // Handle mass noun measure: "Much water flows", "Little time remains"
        if self.check_measure() {
            return self.parse_measure();
        }

        if self.check_quantifier() {
            self.advance();
            return self.parse_quantified();
        }

        if self.check_npi_quantifier() {
            return self.parse_npi_quantified();
        }

        if self.check_temporal_npi() {
            return self.parse_temporal_npi();
        }

        if self.match_token(&[TokenType::LParen]) {
            let expr = self.parse_sentence()?;
            self.consume(TokenType::RParen)?;
            return Ok(expr);
        }

        // Handle pronoun as subject
        if self.check_pronoun() {
            let token = self.advance().clone();
            let (gender, number) = match &token.kind {
                TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                TokenType::Ambiguous { primary, alternatives } => {
                    if let TokenType::Pronoun { gender, number, .. } = **primary {
                        (gender, number)
                    } else {
                        alternatives.iter().find_map(|t| {
                            if let TokenType::Pronoun { gender, number, .. } = t {
                                Some((*gender, *number))
                            } else {
                                None
                            }
                        }).unwrap_or((Gender::Unknown, Number::Singular))
                    }
                }
                _ => (Gender::Unknown, Number::Singular),
            };

            let token_text = self.interner.resolve(token.lexeme);

            // Handle deictic pronouns that don't need discourse resolution
            let resolved = if token_text.eq_ignore_ascii_case("i") {
                self.interner.intern("Speaker")
            } else if token_text.eq_ignore_ascii_case("you") {
                self.interner.intern("Addressee")
            } else {
                // Try discourse resolution for anaphoric pronouns
                let unknown = self.interner.intern("?");
                self.resolve_pronoun(gender, number).unwrap_or(unknown)
            };

            // Check for performative: "I promise that..." or "I promise to..."
            if self.check_performative() {
                if let TokenType::Performative(act) = self.advance().kind.clone() {
                    // Check for infinitive complement: "I promise to come"
                    if self.check(&TokenType::To) {
                        self.advance(); // consume "to"

                        if self.check_verb() {
                            let infinitive_verb = self.consume_verb();

                            let content = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: infinitive_verb,
                                args: self.ctx.terms.alloc_slice([Term::Constant(resolved)]),
                            });

                            return Ok(self.ctx.exprs.alloc(LogicExpr::SpeechAct {
                                performer: resolved,
                                act_type: act,
                                content,
                            }));
                        }
                    }

                    // Skip "that" if present
                    if self.check(&TokenType::That) {
                        self.advance();
                    }
                    let content = self.parse_sentence()?;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::SpeechAct {
                        performer: resolved,
                        act_type: act,
                        content,
                    }));
                }
            }

            // Continue parsing verb phrase with resolved subject
            return self.parse_predicate_with_subject(resolved);
        }

        let subject = self.parse_noun_phrase(true)?;

        // Handle plural subjects: "John and Mary verb"
        if self.check(&TokenType::And) {
            match self.try_parse_plural_subject(&subject) {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {} // Not a plural subject, continue
                Err(e) => return Err(e), // Semantic error (e.g., respectively mismatch)
            }
        }

        // Handle scopal adverbs: "John almost died"
        if self.check_scopal_adverb() {
            return self.parse_scopal_adverb(&subject);
        }

        // Handle topicalization: "The cake, John ate." - first NP is object, not subject
        if self.check(&TokenType::Comma) {
            let saved_pos = self.current;
            self.advance(); // consume comma

            // Check if followed by pronoun subject (e.g., "The book, he read.")
            if self.check_pronoun() {
                let topic_attempt = self.try_parse(|p| {
                    let token = p.peek().clone();
                    let pronoun_features = match &token.kind {
                        TokenType::Pronoun { gender, number, .. } => Some((*gender, *number)),
                        TokenType::Ambiguous { primary, alternatives } => {
                            if let TokenType::Pronoun { gender, number, .. } = **primary {
                                Some((gender, number))
                            } else {
                                alternatives.iter().find_map(|t| {
                                    if let TokenType::Pronoun { gender, number, .. } = t {
                                        Some((*gender, *number))
                                    } else {
                                        None
                                    }
                                })
                            }
                        }
                        _ => None,
                    };

                    if let Some((gender, number)) = pronoun_features {
                        p.advance(); // consume pronoun
                        let unknown = p.interner.intern("?");
                        let resolved = p.resolve_pronoun(gender, number).unwrap_or(unknown);

                        if p.check_verb() {
                            let verb = p.consume_verb();
                            let predicate = p.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: verb,
                                args: p.ctx.terms.alloc_slice([
                                    Term::Constant(resolved),
                                    Term::Constant(subject.noun),
                                ]),
                            });
                            p.wrap_with_definiteness_full(&subject, predicate)
                        } else {
                            Err(ParseError {
                                kind: ParseErrorKind::ExpectedVerb { found: p.peek().kind.clone() },
                                span: p.current_span(),
                            })
                        }
                    } else {
                        Err(ParseError {
                            kind: ParseErrorKind::ExpectedContentWord { found: token.kind },
                            span: p.current_span(),
                        })
                    }
                });

                if let Some(result) = topic_attempt {
                    return Ok(result);
                }
            }

            // Check if followed by another NP and then a verb (topicalization pattern)
            if self.check_content_word() {
                let topic_attempt = self.try_parse(|p| {
                    let real_subject = p.parse_noun_phrase(true)?;
                    if p.check_verb() {
                        let verb = p.consume_verb();
                        let predicate = p.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: p.ctx.terms.alloc_slice([
                                Term::Constant(real_subject.noun),
                                Term::Constant(subject.noun),
                            ]),
                        });
                        p.wrap_with_definiteness_full(&subject, predicate)
                    } else {
                        Err(ParseError {
                            kind: ParseErrorKind::ExpectedVerb { found: p.peek().kind.clone() },
                            span: p.current_span(),
                        })
                    }
                });

                if let Some(result) = topic_attempt {
                    return Ok(result);
                }
            }

            // Restore position if topicalization didn't match
            self.current = saved_pos;
        }

        // Handle relative clause after subject: "The cat that the dog chased ran."
        let mut relative_clause: Option<(Symbol, &'a LogicExpr<'a>)> = None;
        if self.check(&TokenType::That) || self.check(&TokenType::Who) {
            self.advance();
            let var_name = self.next_var_name();
            let rel_pred = self.parse_relative_clause(var_name)?;
            relative_clause = Some((var_name, rel_pred));
        } else if matches!(self.peek().kind, TokenType::Article(_)) && self.is_contact_clause_pattern() {
            // Contact clause (reduced relative): "The cat the dog chased ran."
            // NP + NP + Verb pattern indicates embedded relative without explicit "that"
            let var_name = self.next_var_name();
            let rel_pred = self.parse_relative_clause(var_name)?;
            relative_clause = Some((var_name, rel_pred));
        }

        // Handle main verb after relative clause: "The cat that the dog chased ran."
        if let Some((var_name, rel_clause)) = relative_clause {
            if self.check_verb() {
                let (verb, verb_time, _, _) = self.consume_verb_with_metadata();
                let var_term = Term::Variable(var_name);

                let event_var = self.get_event_var();
                let mut modifiers = vec![];
                if verb_time == Time::Past {
                    modifiers.push(self.interner.intern("Past"));
                }
                let main_pred = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(vec![
                        (ThematicRole::Agent, var_term),
                    ]),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                })));

                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: rel_clause,
                });

                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: inner,
                    op: TokenType::And,
                    right: main_pred,
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
            }

            // No main verb - just the relative clause: "The cat that runs" as a complete NP
            // Build: ∃x(Cat(x) ∧ Runs(x) ∧ ∀y(Cat(y) → y=x))
            if self.is_at_end() || self.check(&TokenType::Period) || self.check(&TokenType::Comma) {
                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: rel_clause,
                });

                // Add uniqueness for definite description
                let uniqueness_body = if subject.definiteness == Some(Definiteness::Definite) {
                    let y_var = self.next_var_name();
                    let type_pred_y = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: subject.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(y_var)]),
                    });
                    let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                        left: self.ctx.terms.alloc(Term::Variable(y_var)),
                        right: self.ctx.terms.alloc(Term::Variable(var_name)),
                    });
                    let uniqueness_cond = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: type_pred_y,
                        op: TokenType::If,
                        right: identity,
                    });
                    let uniqueness = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Universal,
                        variable: y_var,
                        body: uniqueness_cond,
                        island_id: self.current_island,
                    });
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: body,
                        op: TokenType::And,
                        right: uniqueness,
                    })
                } else {
                    body
                };

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var_name,
                    body: uniqueness_body,
                    island_id: self.current_island,
                }));
            }

            // Re-store for copula handling below
            relative_clause = Some((var_name, rel_clause));
        }

        // Identity check: "Clark is equal to Superman"
        if self.check(&TokenType::Identity) {
            self.advance();
            let right = self.consume_content_word()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Identity {
                left: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                right: self.ctx.terms.alloc(Term::Constant(right)),
            }));
        }

        if self.check_modal() {
            if let Some((var_name, rel_clause)) = relative_clause {
                let modal_pred = self.parse_aspect_chain_with_term(Term::Variable(var_name))?;

                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: rel_clause,
                });

                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: inner,
                    op: TokenType::And,
                    right: modal_pred,
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
            }

            let modal_pred = self.parse_aspect_chain(subject.noun)?;
            return self.wrap_with_definiteness_full(&subject, modal_pred);
        }

        if self.check(&TokenType::Is) || self.check(&TokenType::Are)
            || self.check(&TokenType::Was) || self.check(&TokenType::Were)
        {
            let copula_time = if self.check(&TokenType::Was) || self.check(&TokenType::Were) {
                Time::Past
            } else {
                Time::Present
            };
            self.advance();

            // Check for Number token (measure phrase) before comparative or adjective
            // "John is 2 inches taller than Mary" or "The rope is 5 meters long"
            if self.check_number() {
                let measure = self.parse_measure_phrase()?;

                // Check if followed by comparative: "2 inches taller than"
                if self.check_comparative() {
                    return self.parse_comparative(&subject, copula_time, Some(measure));
                }

                // Check for dimensional adjective: "5 meters long"
                if self.check_content_word() {
                    let adj = self.consume_content_word()?;
                    let result = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([
                            Term::Constant(subject.noun),
                            *measure,
                        ]),
                    });
                    return self.wrap_with_definiteness_full(&subject, result);
                }

                // Bare measure phrase: "The temperature is 98.6 degrees."
                // Output: Identity(subject, measure)
                if self.check(&TokenType::Period) || self.is_at_end() {
                    // In imperative mode, reject "x is 5" - suggest "x equals 5"
                    if self.mode == ParserMode::Imperative {
                        let variable = self.interner.resolve(subject.noun).to_string();
                        let value = if let Term::Value { kind, .. } = measure {
                            format!("{:?}", kind)
                        } else {
                            "value".to_string()
                        };
                        return Err(ParseError {
                            kind: ParseErrorKind::IsValueEquality { variable, value },
                            span: self.current_span(),
                        });
                    }
                    let result = self.ctx.exprs.alloc(LogicExpr::Identity {
                        left: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                        right: measure,
                    });
                    return self.wrap_with_definiteness_full(&subject, result);
                }
            }

            // Check for comparative: "is taller than"
            if self.check_comparative() {
                return self.parse_comparative(&subject, copula_time, None);
            }

            // Check for existential "is": "God is." - bare copula followed by period/EOF
            if self.check(&TokenType::Period) || self.is_at_end() {
                let var = self.next_var_name();
                let body = self.ctx.exprs.alloc(LogicExpr::Identity {
                    left: self.ctx.terms.alloc(Term::Variable(var)),
                    right: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                });
                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }));
            }

            // Check for superlative: "is the tallest man"
            if self.check(&TokenType::Article(Definiteness::Definite)) {
                let saved_pos = self.current;
                self.advance();
                if self.check_superlative() {
                    return self.parse_superlative(&subject);
                }
                self.current = saved_pos;
            }

            // Check for predicate NP: "Juliet is the sun" or "John is a man"
            if self.check_article() {
                let predicate_np = self.parse_noun_phrase(true)?;
                let predicate_noun = predicate_np.noun;

                // Phase 41: Event adjective reading
                // "beautiful dancer" in event mode → ∃e(Dance(e) ∧ Agent(e, x) ∧ Beautiful(e))
                if self.event_reading_mode {
                    let noun_str = self.interner.resolve(predicate_noun);
                    if let Some(base_verb) = lexicon::lookup_agentive_noun(noun_str) {
                        // Check if any adjective can modify events
                        let event_adj = predicate_np.adjectives.iter().find(|adj| {
                            lexicon::is_event_modifier_adjective(self.interner.resolve(**adj))
                        });

                        if let Some(&adj_sym) = event_adj {
                            // Build event reading: ∃e(Verb(e) ∧ Agent(e, subject) ∧ Adj(e))
                            let verb_sym = self.interner.intern(base_verb);
                            let event_var = self.get_event_var();

                            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: verb_sym,
                                args: self.ctx.terms.alloc_slice([Term::Variable(event_var)]),
                            });

                            let agent_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: self.interner.intern("Agent"),
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(event_var),
                                    Term::Constant(subject.noun),
                                ]),
                            });

                            let adj_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: adj_sym,
                                args: self.ctx.terms.alloc_slice([Term::Variable(event_var)]),
                            });

                            // Conjoin: Verb(e) ∧ Agent(e, x)
                            let verb_agent = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: verb_pred,
                                op: TokenType::And,
                                right: agent_pred,
                            });

                            // Conjoin: (Verb(e) ∧ Agent(e, x)) ∧ Adj(e)
                            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: verb_agent,
                                op: TokenType::And,
                                right: adj_pred,
                            });

                            // Wrap in existential: ∃e(...)
                            let event_reading = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                                kind: QuantifierKind::Existential,
                                variable: event_var,
                                body,
                                island_id: self.current_island,
                            });

                            return self.wrap_with_definiteness(subject.definiteness, subject.noun, event_reading);
                        }
                    }
                }

                let subject_sort = lexicon::lookup_sort(self.interner.resolve(subject.noun));
                let predicate_sort = lexicon::lookup_sort(self.interner.resolve(predicate_noun));

                if let (Some(s_sort), Some(p_sort)) = (subject_sort, predicate_sort) {
                    if !s_sort.is_compatible_with(p_sort) && !p_sort.is_compatible_with(s_sort) {
                        let metaphor = self.ctx.exprs.alloc(LogicExpr::Metaphor {
                            tenor: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                            vehicle: self.ctx.terms.alloc(Term::Constant(predicate_noun)),
                        });
                        return self.wrap_with_definiteness(subject.definiteness, subject.noun, metaphor);
                    }
                }

                // Default: intersective reading for adjectives
                // Build Adj1(x) ∧ Adj2(x) ∧ ... ∧ Noun(x)
                let mut predicates: Vec<&'a LogicExpr<'a>> = Vec::new();

                // Add adjective predicates
                for &adj_sym in predicate_np.adjectives {
                    let adj_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj_sym,
                        args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
                    });
                    predicates.push(adj_pred);
                }

                // Add noun predicate
                let noun_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate_noun,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
                });
                predicates.push(noun_pred);

                // Conjoin all predicates
                let result = if predicates.len() == 1 {
                    predicates[0]
                } else {
                    let mut combined = predicates[0];
                    for pred in &predicates[1..] {
                        combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: combined,
                            op: TokenType::And,
                            right: *pred,
                        });
                    }
                    combined
                };

                return self.wrap_with_definiteness(subject.definiteness, subject.noun, result);
            }

            // After copula, prefer Adjective over simple-aspect Verb for ambiguous tokens
            // "is open" (Adj: state) is standard; "is open" (Verb: habitual) is ungrammatical here
            let prefer_adjective = if let TokenType::Ambiguous { primary, alternatives } = &self.peek().kind {
                let is_simple_verb = if let TokenType::Verb { aspect, .. } = **primary {
                    aspect == Aspect::Simple
                } else {
                    false
                };
                let has_adj_alt = alternatives.iter().any(|t| matches!(t, TokenType::Adjective(_)));
                is_simple_verb && has_adj_alt
            } else {
                false
            };

            if !prefer_adjective && self.check_verb() {
                let (verb, _verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();

                // Stative verbs cannot be progressive
                if verb_class.is_stative() && verb_aspect == Aspect::Progressive {
                    return Err(ParseError {
                        kind: ParseErrorKind::StativeProgressiveConflict,
                        span: self.current_span(),
                    });
                }

                // Collect any prepositional phrases before "by" (for ditransitives)
                // "given to Mary by John" → goal = Mary, then agent = John
                let mut goal_args: Vec<Term<'a>> = Vec::new();
                while self.check_to_preposition() {
                    self.advance(); // consume "to"
                    let goal = self.parse_noun_phrase(true)?;
                    goal_args.push(self.noun_phrase_to_term(&goal));
                }

                // Check for passive: "was loved by John" or "was given to Mary by John"
                if self.check_by_preposition() {
                    self.advance(); // consume "by"
                    let agent = self.parse_noun_phrase(true)?;

                    // Build args: agent, theme (subject), then any goals
                    let mut args = vec![
                        self.noun_phrase_to_term(&agent),
                        self.noun_phrase_to_term(&subject),
                    ];
                    args.extend(goal_args);

                    let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice(args),
                    });

                    let with_time = if copula_time == Time::Past {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: predicate,
                        })
                    } else {
                        predicate
                    };

                    return self.wrap_with_definiteness(subject.definiteness, subject.noun, with_time);
                }

                // Agentless passive: "The book was read" → ∃x.Read(x, Book)
                if copula_time == Time::Past && verb_aspect == Aspect::Simple {
                    // Could be agentless passive - treat as existential
                    let var_name = self.next_var_name();
                    let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([
                            Term::Variable(var_name),
                            Term::Constant(subject.noun),
                        ]),
                    });

                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: subject.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                    });

                    let temporal = self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: predicate,
                    });

                    let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: type_pred,
                        op: TokenType::And,
                        right: temporal,
                    });

                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Existential,
                        variable: var_name,
                        body,
                        island_id: self.current_island,
                    }));
                }

                let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
                });

                let with_aspect = if verb_aspect == Aspect::Progressive {
                    // Semelfactive + Progressive → Iterative
                    let operator = if verb_class == VerbClass::Semelfactive {
                        AspectOperator::Iterative
                    } else {
                        AspectOperator::Progressive
                    };
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator,
                        body: predicate,
                    })
                } else {
                    predicate
                };

                let with_time = if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: with_aspect,
                    })
                } else {
                    with_aspect
                };

                return self.wrap_with_definiteness(subject.definiteness, subject.noun, with_time);
            }

            // Handle relative clause with copula: "The book that John read is good."
            if let Some((var_name, rel_clause)) = relative_clause {
                let var_term = Term::Variable(var_name);
                let pred_word = self.consume_content_word()?;

                let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: pred_word,
                    args: self.ctx.terms.alloc_slice([var_term]),
                });

                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: rel_clause,
                });

                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: inner,
                    op: TokenType::And,
                    right: main_pred,
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
            }

            // Handle "The king is bald" - NP copula ADJ/NOUN
            // Also handles bare noun predicates like "Time is money"
            let predicate_name = self.consume_content_word()?;

            // Check for sort violation (metaphor detection)
            let subject_sort = lexicon::lookup_sort(self.interner.resolve(subject.noun));
            let predicate_str = self.interner.resolve(predicate_name);

            // Check ontology's predicate sort requirements (for adjectives like "happy")
            if let Some(s_sort) = subject_sort {
                if !crate::ontology::check_sort_compatibility(predicate_str, s_sort) {
                    let metaphor = self.ctx.exprs.alloc(LogicExpr::Metaphor {
                        tenor: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                        vehicle: self.ctx.terms.alloc(Term::Constant(predicate_name)),
                    });
                    return self.wrap_with_definiteness(subject.definiteness, subject.noun, metaphor);
                }
            }

            // Check copular NP predicate sort compatibility (for "Time is money")
            let predicate_sort = lexicon::lookup_sort(predicate_str);
            if let (Some(s_sort), Some(p_sort)) = (subject_sort, predicate_sort) {
                if s_sort != p_sort && !s_sort.is_compatible_with(p_sort) && !p_sort.is_compatible_with(s_sort) {
                    let metaphor = self.ctx.exprs.alloc(LogicExpr::Metaphor {
                        tenor: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                        vehicle: self.ctx.terms.alloc(Term::Constant(predicate_name)),
                    });
                    return self.wrap_with_definiteness(subject.definiteness, subject.noun, metaphor);
                }
            }

            let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate_name,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
            });
            return self.wrap_with_definiteness(subject.definiteness, subject.noun, predicate);
        }

        // Handle auxiliary: set pending_time, handle negation
        if self.check_auxiliary() {
            let aux_time = if let TokenType::Auxiliary(time) = self.advance().kind {
                time
            } else {
                Time::None
            };
            self.pending_time = Some(aux_time);

            // Handle negation: "John did not see dogs"
            if self.match_token(&[TokenType::Not]) {
                self.negative_depth += 1;

                // Skip "ever" if present: "John did not ever run"
                if self.check(&TokenType::Ever) {
                    self.advance();
                }

                if self.check_verb() {
                    let verb = self.consume_verb();
                    let subject_term = self.noun_phrase_to_term(&subject);

                    // Check for NPI object first: "John did not see anything"
                    if self.check_npi_object() {
                        let npi_token = self.advance().kind.clone();
                        let obj_var = self.next_var_name();

                        let restriction_name = match npi_token {
                            TokenType::Anything => "Thing",
                            TokenType::Anyone => "Person",
                            _ => "Thing",
                        };

                        let restriction_sym = self.interner.intern(restriction_name);
                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: restriction_sym,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([subject_term.clone(), Term::Variable(obj_var)]),
                        });

                        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: verb_pred,
                        });

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Existential,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    // Check for quantifier object: "John did not see any dogs"
                    if self.check_quantifier() {
                        let quantifier_token = self.advance().kind.clone();
                        let object_np = self.parse_noun_phrase(false)?;
                        let obj_var = self.next_var_name();

                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: object_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([subject_term.clone(), Term::Variable(obj_var)]),
                        });

                        let (kind, body) = match quantifier_token {
                            TokenType::Any => {
                                if self.is_negative_context() {
                                    (
                                        QuantifierKind::Existential,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::And,
                                            right: verb_pred,
                                        }),
                                    )
                                } else {
                                    (
                                        QuantifierKind::Universal,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::If,
                                            right: verb_pred,
                                        }),
                                    )
                                }
                            }
                            TokenType::Some => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                            TokenType::All => (
                                QuantifierKind::Universal,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::If,
                                    right: verb_pred,
                                }),
                            ),
                            _ => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                        };

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    let mut roles: Vec<(ThematicRole, Term<'a>)> = vec![(ThematicRole::Agent, subject_term)];

                    // Add temporal modifier from pending_time
                    let effective_time = self.pending_time.take().unwrap_or(Time::None);
                    let mut modifiers: Vec<Symbol> = vec![];
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    // Check for object
                    if self.check_content_word() || self.check_article() {
                        let object = self.parse_noun_phrase(false)?;
                        let object_term = self.noun_phrase_to_term(&object);
                        roles.push((ThematicRole::Theme, object_term));
                    }

                    let event_var = self.get_event_var();
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                    })));

                    self.negative_depth -= 1;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: neo_event,
                    }));
                }

                self.negative_depth -= 1;
            }
            // Non-negated auxiliary: pending_time is set, fall through to normal verb handling
        }

        // Check for presupposition triggers: "stopped", "started", "regrets", "knows"
        // Factive verbs like "know" only trigger presupposition with clausal complements
        // "John knows that..." → presupposition, "John knows Mary" → regular verb
        // Only trigger presupposition if followed by a gerund (e.g., "stopped smoking")
        // "John stopped." alone should parse as intransitive verb, not presupposition
        if self.check_presup_trigger() && !self.is_followed_by_np_object() && self.is_followed_by_gerund() {
            let presup_kind = match self.advance().kind {
                TokenType::PresupTrigger(kind) => kind,
                TokenType::Verb { lemma, .. } => {
                    let s = self.interner.resolve(lemma).to_lowercase();
                    crate::lexicon::lookup_presup_trigger(&s)
                        .expect("Lexicon mismatch: Verb flagged as trigger but lookup failed")
                }
                _ => panic!("Expected presupposition trigger"),
            };
            return self.parse_presupposition(&subject, presup_kind);
        }

        // Handle bare plurals: "Birds fly." → Gen x. Bird(x) → Fly(x)
        let noun_str = self.interner.resolve(subject.noun);
        let is_bare_plural = subject.definiteness.is_none()
            && subject.possessor.is_none()
            && Self::is_plural_noun(noun_str)
            && self.check_verb();

        if is_bare_plural {
            let var_name = self.next_var_name();
            let (verb, verb_time, verb_aspect, _) = self.consume_verb_with_metadata();

            let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: subject.noun,
                args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
            });

            let mut args = vec![Term::Variable(var_name)];
            if self.check_content_word() {
                let object = self.parse_noun_phrase(false)?;
                args.push(self.noun_phrase_to_term(&object));
            }

            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice(args),
            });

            let effective_time = self.pending_time.take().unwrap_or(verb_time);
            let with_time = match effective_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: verb_pred,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: verb_pred,
                }),
                _ => verb_pred,
            };

            let with_aspect = if verb_aspect == Aspect::Progressive {
                self.ctx.exprs.alloc(LogicExpr::Aspectual {
                    operator: AspectOperator::Progressive,
                    body: with_time,
                })
            } else {
                with_time
            };

            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: type_pred,
                op: TokenType::If,
                right: with_aspect,
            });

            return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Generic,
                variable: var_name,
                body,
                island_id: self.current_island,
            }));
        }

        // Handle do-support: "John does not exist" or "John does run"
        if self.check(&TokenType::Does) || self.check(&TokenType::Do) {
            self.advance(); // consume does/do
            let is_negated = self.match_token(&[TokenType::Not]);

            if self.check_verb() {
                let verb = self.consume_verb();
                let verb_lemma = self.interner.resolve(verb).to_lowercase();

                // Check for embedded wh-clause with negation: "I don't know who"
                if self.check_wh_word() {
                    let wh_token = self.advance().kind.clone();
                    let is_who = matches!(wh_token, TokenType::Who);
                    let is_what = matches!(wh_token, TokenType::What);

                    let is_sluicing = self.is_at_end() ||
                        self.check(&TokenType::Period) ||
                        self.check(&TokenType::Comma);

                    if is_sluicing {
                        if let Some(template) = self.last_event_template.clone() {
                            let wh_var = self.next_var_name();
                            let subject_term = self.noun_phrase_to_term(&subject);

                            let roles: Vec<_> = if is_who {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            } else if is_what {
                                vec![
                                    (ThematicRole::Agent, subject_term.clone()),
                                    (ThematicRole::Theme, Term::Variable(wh_var)),
                                ]
                            } else {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            };

                            let event_var = self.get_event_var();
                            let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var,
                                verb: template.verb,
                                roles: self.ctx.roles.alloc_slice(roles),
                                modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                            })));

                            let question = self.ctx.exprs.alloc(LogicExpr::Question {
                                wh_variable: wh_var,
                                body: reconstructed,
                            });

                            let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var: self.get_event_var(),
                                verb,
                                roles: self.ctx.roles.alloc_slice(vec![
                                    (ThematicRole::Agent, subject_term),
                                    (ThematicRole::Theme, Term::Proposition(question)),
                                ]),
                                modifiers: self.ctx.syms.alloc_slice(vec![]),
                            })));

                            let result = if is_negated {
                                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                    op: TokenType::Not,
                                    operand: know_event,
                                })
                            } else {
                                know_event
                            };

                            return self.wrap_with_definiteness_full(&subject, result);
                        }
                    }
                }

                // Special handling for "exist" with negation
                if verb_lemma == "exist" && is_negated {
                    // "The King of France does not exist" -> ¬∃x(KingOfFrance(x))
                    let var_name = self.next_var_name();
                    let restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: subject.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                    });
                    let exists = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Existential,
                        variable: var_name,
                        body: restriction,
                        island_id: self.current_island,
                    });
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: exists,
                    }));
                }

                // Regular do-support: "John does run" or "John does not run"
                let subject_term = self.noun_phrase_to_term(&subject);
                let roles: Vec<(ThematicRole, Term<'a>)> = vec![(ThematicRole::Agent, subject_term)];
                let modifiers: Vec<Symbol> = vec![];
                let event_var = self.get_event_var();

                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                })));

                if is_negated {
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: neo_event,
                    }));
                }
                return Ok(neo_event);
            }
        }

        // Garden path detection: "The horse raced past the barn fell."
        // If we have a definite NP + past verb + more content + another verb,
        // try reduced relative interpretation
        // Skip if pending_time is set (auxiliary like "will" was just consumed)
        // Skip if verb is has/have/had (perfect aspect, not reduced relative)
        let is_perfect_aux = if self.check_verb() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            word == "has" || word == "have" || word == "had"
        } else {
            false
        };
        if subject.definiteness == Some(Definiteness::Definite) && self.check_verb() && self.pending_time.is_none() && !is_perfect_aux {
            let saved_pos = self.current;

            // Try parsing as reduced relative: first verb is modifier, look for main verb after
            if let Some(garden_path_result) = self.try_parse(|p| {
                let (modifier_verb, _modifier_time, _, _) = p.consume_verb_with_metadata();

                // Collect any PP modifiers on the reduced relative
                let mut pp_mods: Vec<&'a LogicExpr<'a>> = Vec::new();
                while p.check_preposition() {
                    let prep = if let TokenType::Preposition(prep) = p.advance().kind {
                        prep
                    } else {
                        break;
                    };
                    if p.check_article() || p.check_content_word() {
                        let pp_obj = p.parse_noun_phrase(false)?;
                        let pp_pred = p.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep,
                            args: p.ctx.terms.alloc_slice([Term::Variable(p.interner.intern("x")), Term::Constant(pp_obj.noun)]),
                        });
                        pp_mods.push(pp_pred);
                    }
                }

                // Now check if there's ANOTHER verb (the real main verb)
                if !p.check_verb() {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedVerb { found: p.peek().kind.clone() },
                        span: p.current_span(),
                    });
                }

                let (main_verb, main_time, _, _) = p.consume_verb_with_metadata();

                // Build: ∃x((Horse(x) ∧ ∀y(Horse(y) → y=x)) ∧ Raced(x) ∧ Past(x, Barn) ∧ Fell(x))
                let var = p.interner.intern("x");

                // Type predicate
                let type_pred = p.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: p.ctx.terms.alloc_slice([Term::Variable(var)]),
                });

                // Modifier verb predicate (reduced relative)
                let mod_pred = p.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: modifier_verb,
                    args: p.ctx.terms.alloc_slice([Term::Variable(var)]),
                });

                // Main verb predicate
                let main_pred = p.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: main_verb,
                    args: p.ctx.terms.alloc_slice([Term::Variable(var)]),
                });

                // Combine type + modifier
                let mut body = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: mod_pred,
                });

                // Add PP modifiers
                for pp in pp_mods {
                    body = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: body,
                        op: TokenType::And,
                        right: pp,
                    });
                }

                // Add main predicate
                body = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: main_pred,
                });

                // Wrap with temporal if needed
                let with_time = match main_time {
                    Time::Past => p.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body,
                    }),
                    Time::Future => p.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Future,
                        body,
                    }),
                    _ => body,
                };

                // Wrap in existential quantifier for definite
                Ok(p.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body: with_time,
                    island_id: p.current_island,
                }))
            }) {
                return Ok(garden_path_result);
            }

            // Restore position if garden path didn't work
            self.current = saved_pos;
        }

        if self.check_modal() {
            return self.parse_aspect_chain(subject.noun);
        }

        // Handle "has/have/had" perfect aspect: "John has run"
        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "has" || word == "have" || word == "had" {
                // Lookahead to distinguish perfect aspect ("has eaten") from possession ("has 3 children")
                let is_perfect_aspect = if self.current + 1 < self.tokens.len() {
                    let next_token = &self.tokens[self.current + 1].kind;
                    matches!(
                        next_token,
                        TokenType::Verb { .. } | TokenType::Not
                    ) && !matches!(next_token, TokenType::Number(_))
                } else {
                    false
                };
                if is_perfect_aspect {
                    return self.parse_aspect_chain(subject.noun);
                }
                // Otherwise fall through to verb parsing below
            }
        }

        // Handle TokenType::Had for past perfect: "John had run"
        if self.check(&TokenType::Had) {
            return self.parse_aspect_chain(subject.noun);
        }

        // Handle "never" temporal negation: "John never runs"
        if self.check(&TokenType::Never) {
            self.advance();
            let verb = self.consume_verb();
            let subject_term = self.noun_phrase_to_term(&subject);
            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([subject_term]),
            });
            let result = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: verb_pred,
            });
            return self.wrap_with_definiteness_full(&subject, result);
        }

        if self.check_verb() {
            let (mut verb, verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();

            // Check for verb sort violation (metaphor detection)
            let subject_sort = lexicon::lookup_sort(self.interner.resolve(subject.noun));
            let verb_str = self.interner.resolve(verb);
            if let Some(s_sort) = subject_sort {
                if !crate::ontology::check_sort_compatibility(verb_str, s_sort) {
                    let metaphor = self.ctx.exprs.alloc(LogicExpr::Metaphor {
                        tenor: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                        vehicle: self.ctx.terms.alloc(Term::Constant(verb)),
                    });
                    return self.wrap_with_definiteness(subject.definiteness, subject.noun, metaphor);
                }
            }

            // Check for control verb + infinitive
            if self.is_control_verb(verb) {
                return self.parse_control_structure(&subject, verb, verb_time);
            }

            // If we have a relative clause, use variable binding
            if let Some((var_name, rel_clause)) = relative_clause {
                let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let effective_time = self.pending_time.take().unwrap_or(verb_time);
                let with_time = match effective_time {
                    Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: main_pred,
                    }),
                    Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Future,
                        body: main_pred,
                    }),
                    _ => main_pred,
                };

                // Build: ∃x(Type(x) ∧ RelClause(x) ∧ MainPred(x))
                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: subject.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

                let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: rel_clause,
                });

                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: inner,
                    op: TokenType::And,
                    right: with_time,
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
            }

            let subject_term = self.noun_phrase_to_term(&subject);
            let mut args = vec![subject_term.clone()];

            let unknown = self.interner.intern("?");

            // Check for embedded wh-clause: "I know who/what"
            if self.check_wh_word() {
                let wh_token = self.advance().kind.clone();

                // Determine wh-type for slot matching
                let is_who = matches!(wh_token, TokenType::Who);
                let is_what = matches!(wh_token, TokenType::What);

                // Check for sluicing: wh-word followed by terminator
                let is_sluicing = self.is_at_end() ||
                    self.check(&TokenType::Period) ||
                    self.check(&TokenType::Comma);

                if is_sluicing {
                    // Reconstruct from template
                    if let Some(template) = self.last_event_template.clone() {
                        let wh_var = self.next_var_name();

                        // Build roles with wh-variable in appropriate slot
                        let roles: Vec<_> = if is_who {
                            // "who" replaces Agent
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        } else if is_what {
                            // "what" replaces Theme - use Agent from context, Theme is variable
                            vec![
                                (ThematicRole::Agent, subject_term.clone()),
                                (ThematicRole::Theme, Term::Variable(wh_var)),
                            ]
                        } else {
                            // Default: wh-variable as Agent
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        };

                        let event_var = self.get_event_var();
                        let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb: template.verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                        })));

                        let question = self.ctx.exprs.alloc(LogicExpr::Question {
                            wh_variable: wh_var,
                            body: reconstructed,
                        });

                        // Build: Know(subject, question)
                        let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var: self.get_event_var(),
                            verb,
                            roles: self.ctx.roles.alloc_slice(vec![
                                (ThematicRole::Agent, subject_term),
                                (ThematicRole::Theme, Term::Proposition(question)),
                            ]),
                            modifiers: self.ctx.syms.alloc_slice(vec![]),
                        })));

                        return self.wrap_with_definiteness_full(&subject, know_event);
                    }
                }

                // Non-sluicing embedded question: "I know who runs"
                let embedded = self.parse_embedded_wh_clause()?;
                let question = self.ctx.exprs.alloc(LogicExpr::Question {
                    wh_variable: self.interner.intern("x"),
                    body: embedded,
                });

                // Build: Know(subject, question)
                let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: self.get_event_var(),
                    verb,
                    roles: self.ctx.roles.alloc_slice(vec![
                        (ThematicRole::Agent, subject_term),
                        (ThematicRole::Theme, Term::Proposition(question)),
                    ]),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                })));

                return self.wrap_with_definiteness_full(&subject, know_event);
            }

            let mut object_term: Option<Term<'a>> = None;
            let mut second_object_term: Option<Term<'a>> = None;
            let mut object_superlative: Option<(Symbol, Symbol)> = None; // (adjective, noun)
            if self.check(&TokenType::Reflexive) {
                self.advance();
                let term = self.noun_phrase_to_term(&subject);
                object_term = Some(term.clone());
                args.push(term);

                // Check for distanced phrasal verb particle: "gave himself up"
                if let TokenType::Particle(particle_sym) = self.peek().kind {
                    let verb_str = self.interner.resolve(verb).to_lowercase();
                    let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                    if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                        self.advance();
                        verb = self.interner.intern(phrasal_lemma);
                    }
                }
            } else if self.check_pronoun() {
                let token = self.advance().clone();
                if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    let resolved = self.resolve_pronoun(gender, number)
                        .unwrap_or(unknown);
                    let term = Term::Constant(resolved);
                    object_term = Some(term.clone());
                    args.push(term);

                    // Check for distanced phrasal verb particle: "gave it up"
                    if let TokenType::Particle(particle_sym) = self.peek().kind {
                        let verb_str = self.interner.resolve(verb).to_lowercase();
                        let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                        if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                            self.advance();
                            verb = self.interner.intern(phrasal_lemma);
                        }
                    }
                }
            } else if self.check_quantifier() || self.check_article() {
                // Quantified object: "John loves every woman" or "John saw a dog"
                let obj_quantifier = if self.check_quantifier() {
                    Some(self.advance().kind.clone())
                } else {
                    let art = self.advance().kind.clone();
                    if let TokenType::Article(def) = art {
                        if def == Definiteness::Indefinite {
                            Some(TokenType::Some)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let object_np = self.parse_noun_phrase(false)?;

                // Capture superlative info for constraint generation
                if let Some(adj) = object_np.superlative {
                    object_superlative = Some((adj, object_np.noun));
                }

                // Check for distanced phrasal verb particle: "gave the book up"
                if let TokenType::Particle(particle_sym) = self.peek().kind {
                    let verb_str = self.interner.resolve(verb).to_lowercase();
                    let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                    if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                        self.advance(); // consume the particle
                        verb = self.interner.intern(phrasal_lemma);
                    }
                }

                if let Some(obj_q) = obj_quantifier {
                    // Check for opaque verb with indefinite object (de dicto reading)
                    // For verbs like "seek", "want", "believe" with indefinite objects,
                    // use Term::Intension to represent the intensional (concept) reading
                    let verb_str = self.interner.resolve(verb).to_lowercase();
                    let is_opaque = lexicon::lookup_verb_db(&verb_str)
                        .map(|meta| meta.features.contains(&lexicon::Feature::Opaque))
                        .unwrap_or(false);

                    if is_opaque && matches!(obj_q, TokenType::Some) {
                        // De dicto reading: use Term::Intension for the theme
                        let intension_term = Term::Intension(object_np.noun);

                        // Register intensional entity for anaphora resolution
                        let noun_str = self.interner.resolve(object_np.noun).to_string();
                        let first_char = noun_str.chars().next().unwrap_or('X');
                        if first_char.is_alphabetic() {
                            let symbol = format!("^{}", first_char.to_uppercase());
                            self.register_entity(&symbol, &noun_str, Gender::Neuter, Number::Singular);
                        }

                        let event_var = self.get_event_var();
                        let mut modifiers = self.collect_adverbs();
                        let effective_time = self.pending_time.take().unwrap_or(verb_time);
                        match effective_time {
                            Time::Past => modifiers.push(self.interner.intern("Past")),
                            Time::Future => modifiers.push(self.interner.intern("Future")),
                            _ => {}
                        }

                        let subject_term_for_event = self.noun_phrase_to_term(&subject);
                        let roles = vec![
                            (ThematicRole::Agent, subject_term_for_event),
                            (ThematicRole::Theme, intension_term),
                        ];

                        let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(modifiers),
                        })));

                        return self.wrap_with_definiteness_full(&subject, neo_event);
                    }

                    let obj_var = self.next_var_name();
                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object_np.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                    });

                    let obj_restriction = if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                        self.advance();
                        let rel_clause = self.parse_relative_clause(obj_var)?;
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: type_pred,
                            op: TokenType::And,
                            right: rel_clause,
                        })
                    } else {
                        type_pred
                    };

                    let event_var = self.get_event_var();
                    let mut modifiers = self.collect_adverbs();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    let subject_term_for_event = self.noun_phrase_to_term(&subject);
                    let roles = vec![
                        (ThematicRole::Agent, subject_term_for_event),
                        (ThematicRole::Theme, Term::Variable(obj_var)),
                    ];

                    // Capture template with object type for ellipsis reconstruction
                    // Use the object noun type instead of variable for reconstruction
                    let template_roles = vec![
                        (ThematicRole::Agent, subject_term_for_event),
                        (ThematicRole::Theme, Term::Constant(object_np.noun)),
                    ];
                    self.capture_event_template(verb, &template_roles, &modifiers);

                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                    })));

                    let obj_kind = match obj_q {
                        TokenType::All => QuantifierKind::Universal,
                        TokenType::Some => QuantifierKind::Existential,
                        TokenType::No => QuantifierKind::Universal,
                        TokenType::Most => QuantifierKind::Most,
                        TokenType::Few => QuantifierKind::Few,
                        TokenType::Many => QuantifierKind::Many,
                        TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                        TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                        TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                        _ => QuantifierKind::Existential,
                    };

                    let obj_body = match obj_q {
                        TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::If,
                            right: neo_event,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: neo_event,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: obj_restriction,
                                op: TokenType::If,
                                right: neg,
                            })
                        }
                        _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: neo_event,
                        }),
                    };

                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: obj_kind,
                        variable: obj_var,
                        body: obj_body,
                        island_id: self.current_island,
                    }));
                } else {
                    let term = self.noun_phrase_to_term(&object_np);
                    object_term = Some(term.clone());
                    args.push(term);
                }
            } else if self.check_focus() {
                let focus_kind = if let TokenType::Focus(k) = self.advance().kind {
                    k
                } else {
                    FocusKind::Only
                };

                let event_var = self.get_event_var();
                let mut modifiers = self.collect_adverbs();
                let effective_time = self.pending_time.take().unwrap_or(verb_time);
                match effective_time {
                    Time::Past => modifiers.push(self.interner.intern("Past")),
                    Time::Future => modifiers.push(self.interner.intern("Future")),
                    _ => {}
                }

                let subject_term_for_event = self.noun_phrase_to_term(&subject);

                if self.check_preposition() {
                    let prep_token = self.advance().clone();
                    let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                        sym
                    } else {
                        self.interner.intern("to")
                    };
                    let pp_obj = self.parse_noun_phrase(false)?;
                    let pp_obj_term = Term::Constant(pp_obj.noun);

                    let roles = vec![(ThematicRole::Agent, subject_term_for_event)];
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                    })));

                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var), pp_obj_term]),
                    });

                    let with_pp = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: neo_event,
                        op: TokenType::And,
                        right: pp_pred,
                    });

                    let focused_ref = self.ctx.terms.alloc(pp_obj_term);
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                        kind: focus_kind,
                        focused: focused_ref,
                        scope: with_pp,
                    }));
                }

                let focused_np = self.parse_noun_phrase(false)?;
                let focused_term = self.noun_phrase_to_term(&focused_np);
                args.push(focused_term.clone());

                let roles = vec![
                    (ThematicRole::Agent, subject_term_for_event),
                    (ThematicRole::Theme, focused_term.clone()),
                ];

                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                })));

                let focused_ref = self.ctx.terms.alloc(focused_term);
                return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                    kind: focus_kind,
                    focused: focused_ref,
                    scope: neo_event,
                }));
            } else if self.check_number() {
                // Handle "has 3 children" or "has cardinality aleph_0"
                let measure = self.parse_measure_phrase()?;

                // If there's a noun after the measure (for "3 children" where children wasn't a unit)
                if self.check_content_word() {
                    let noun_sym = self.consume_content_word()?;
                    // Build: Has(Subject, 3, Children) where 3 is the count
                    let count_term = *measure;
                    object_term = Some(count_term.clone());
                    args.push(count_term);
                    second_object_term = Some(Term::Constant(noun_sym));
                    args.push(Term::Constant(noun_sym));
                } else {
                    // Just the measure: "has cardinality 5"
                    object_term = Some(*measure);
                    args.push(*measure);
                }
            } else if self.check_content_word() || self.check_article() {
                let object = self.parse_noun_phrase(false)?;
                if let Some(adj) = object.superlative {
                    object_superlative = Some((adj, object.noun));
                }

                // Collect all objects for potential "respectively" handling
                let mut all_objects: Vec<Symbol> = vec![object.noun];

                // Check for coordinated objects: "Tom and Jerry and Bob"
                while self.check(&TokenType::And) {
                    let saved = self.current;
                    self.advance(); // consume "and"
                    if self.check_content_word() || self.check_article() {
                        let next_obj = match self.parse_noun_phrase(false) {
                            Ok(np) => np,
                            Err(_) => {
                                self.current = saved;
                                break;
                            }
                        };
                        all_objects.push(next_obj.noun);
                    } else {
                        self.current = saved;
                        break;
                    }
                }

                // Check for "respectively" with single subject
                if self.check(&TokenType::Respectively) {
                    let respectively_span = self.peek().span;
                    // Single subject with multiple objects + respectively = error
                    if all_objects.len() > 1 {
                        return Err(ParseError {
                            kind: ParseErrorKind::RespectivelyLengthMismatch {
                                subject_count: 1,
                                object_count: all_objects.len(),
                            },
                            span: respectively_span,
                        });
                    }
                    // Single subject, single object + respectively is valid (trivially pairwise)
                    self.advance(); // consume "respectively"
                }

                // Use the first object (or only object) for normal processing
                let term = self.noun_phrase_to_term(&object);
                object_term = Some(term.clone());
                args.push(term.clone());

                // For multiple objects without "respectively", use group semantics
                if all_objects.len() > 1 {
                    let obj_members: Vec<Term<'a>> = all_objects.iter()
                        .map(|o| Term::Constant(*o))
                        .collect();
                    let obj_group = Term::Group(self.ctx.terms.alloc_slice(obj_members));
                    // Replace the single object with the group
                    args.pop();
                    args.push(obj_group);
                }

                // Check for distanced phrasal verb particle: "gave the book up"
                if let TokenType::Particle(particle_sym) = self.peek().kind {
                    let verb_str = self.interner.resolve(verb).to_lowercase();
                    let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                    if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                        self.advance(); // consume the particle
                        verb = self.interner.intern(phrasal_lemma);
                    }
                }

                // Check for "has cardinality aleph_0" pattern: noun followed by number
                if self.check_number() {
                    let measure = self.parse_measure_phrase()?;
                    second_object_term = Some(*measure);
                    args.push(*measure);
                }
                // Check for ditransitive: "John gave Mary a book"
                else {
                    let verb_str = self.interner.resolve(verb);
                    if Lexer::is_ditransitive_verb(verb_str) && (self.check_content_word() || self.check_article()) {
                        let second_np = self.parse_noun_phrase(false)?;
                        let second_term = self.noun_phrase_to_term(&second_np);
                        second_object_term = Some(second_term.clone());
                        args.push(second_term);
                    }
                }
            }

            let mut pp_predicates: Vec<&'a LogicExpr<'a>> = Vec::new();
            while self.check_preposition() || self.check_to() {
                let prep_token = self.advance().clone();
                let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                    sym
                } else if matches!(prep_token.kind, TokenType::To) {
                    self.interner.intern("To")
                } else {
                    continue;
                };

                let pp_obj_term = if self.check(&TokenType::Reflexive) {
                    self.advance();
                    self.noun_phrase_to_term(&subject)
                } else if self.check_pronoun() {
                    let token = self.advance().clone();
                    if let TokenType::Pronoun { gender, number, .. } = token.kind {
                        let resolved = self.resolve_pronoun(gender, number)
                            .unwrap_or(unknown);
                        Term::Constant(resolved)
                    } else {
                        continue;
                    }
                } else if self.check_content_word() || self.check_article() {
                    let prep_obj = self.parse_noun_phrase(false)?;
                    self.noun_phrase_to_term(&prep_obj)
                } else {
                    continue;
                };

                if self.pp_attach_to_noun {
                    if let Some(ref obj) = object_term {
                        // NP-attachment: PP modifies the object noun
                        let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep_name,
                            args: self.ctx.terms.alloc_slice([obj.clone(), pp_obj_term]),
                        });
                        pp_predicates.push(pp_pred);
                    } else {
                        args.push(pp_obj_term);
                    }
                } else {
                    // VP-attachment: PP modifies the event (instrument/manner)
                    let event_sym = self.get_event_var();
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_sym), pp_obj_term]),
                    });
                    pp_predicates.push(pp_pred);
                }
            }

            // Check for trailing relative clause on object NP: "the girl with the telescope that laughed"
            if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                self.advance();
                let rel_var = self.next_var_name();
                let rel_pred = self.parse_relative_clause(rel_var)?;
                pp_predicates.push(rel_pred);
            }

            // Collect any trailing adverbs FIRST (before building NeoEvent)
            let mut modifiers = self.collect_adverbs();

            // Add temporal modifier as part of event semantics
            let effective_time = self.pending_time.take().unwrap_or(verb_time);
            match effective_time {
                Time::Past => modifiers.push(self.interner.intern("Past")),
                Time::Future => modifiers.push(self.interner.intern("Future")),
                _ => {}
            }

            // Add aspect modifier if applicable
            if verb_aspect == Aspect::Progressive {
                modifiers.push(self.interner.intern("Progressive"));
            } else if verb_aspect == Aspect::Perfect {
                modifiers.push(self.interner.intern("Perfect"));
            }

            // Build thematic roles for Neo-Davidsonian event semantics
            let mut roles: Vec<(ThematicRole, Term<'a>)> = Vec::new();
            roles.push((ThematicRole::Agent, subject_term));
            if let Some(second_obj) = second_object_term {
                // Ditransitive: first object is Recipient, second is Theme
                if let Some(first_obj) = object_term {
                    roles.push((ThematicRole::Recipient, first_obj));
                }
                roles.push((ThematicRole::Theme, second_obj));
            } else if let Some(obj) = object_term {
                // Normal transitive: object is Theme
                roles.push((ThematicRole::Theme, obj));
            }

            // Create event variable
            let event_var = self.get_event_var();

            // Capture template for ellipsis reconstruction before consuming roles
            self.capture_event_template(verb, &roles, &modifiers);

            // Create NeoEvent structure with all modifiers including time/aspect
            let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self.ctx.roles.alloc_slice(roles),
                modifiers: self.ctx.syms.alloc_slice(modifiers),
            })));

            // Combine with PP predicates if any
            let with_pps = if pp_predicates.is_empty() {
                neo_event
            } else {
                let mut combined = neo_event;
                for pp in pp_predicates {
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: pp,
                    });
                }
                combined
            };

            // Apply aspectual operators based on verb class
            let with_aspect = if verb_aspect == Aspect::Progressive {
                // Semelfactive + Progressive → Iterative
                if verb_class == crate::lexicon::VerbClass::Semelfactive {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Iterative,
                        body: with_pps,
                    })
                } else {
                    // Other verbs + Progressive → Progressive
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Progressive,
                        body: with_pps,
                    })
                }
            } else if verb_aspect == Aspect::Perfect {
                self.ctx.exprs.alloc(LogicExpr::Aspectual {
                    operator: AspectOperator::Perfect,
                    body: with_pps,
                })
            } else if effective_time == Time::Present && verb_aspect == Aspect::Simple {
                // Non-state verbs in simple present get Habitual reading
                if !verb_class.is_stative() {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Habitual,
                        body: with_pps,
                    })
                } else {
                    // State verbs in present: direct predication
                    with_pps
                }
            } else {
                with_pps
            };

            let with_adverbs = with_aspect;

            // Check for temporal anchor adverb at end of sentence
            let with_temporal = if self.check_temporal_adverb() {
                let anchor = if let TokenType::TemporalAdverb(adv) = self.advance().kind.clone() {
                    adv
                } else {
                    panic!("Expected temporal adverb");
                };
                self.ctx.exprs.alloc(LogicExpr::TemporalAnchor {
                    anchor,
                    body: with_adverbs,
                })
            } else {
                with_adverbs
            };

            let wrapped = self.wrap_with_definiteness_full(&subject, with_temporal)?;

            // Add superlative constraint for object NP if applicable
            if let Some((adj, noun)) = object_superlative {
                let superlative_expr = self.ctx.exprs.alloc(LogicExpr::Superlative {
                    adjective: adj,
                    subject: self.ctx.terms.alloc(Term::Constant(noun)),
                    domain: noun,
                });
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: wrapped,
                    op: TokenType::And,
                    right: superlative_expr,
                }));
            }

            return Ok(wrapped);
        }

        Ok(self.ctx.exprs.alloc(LogicExpr::Atom(subject.noun)))
    }

    fn check_preposition(&self) -> bool {
        matches!(self.peek().kind, TokenType::Preposition(_))
    }

    fn check_by_preposition(&self) -> bool {
        if let TokenType::Preposition(p) = self.peek().kind {
            p.is(self.interner, "by")
        } else {
            false
        }
    }

    fn check_preposition_is(&self, word: &str) -> bool {
        if let TokenType::Preposition(p) = self.peek().kind {
            p.is(self.interner, word)
        } else {
            false
        }
    }

    /// Check if current token is a word (noun/adj/verb lexeme) matching the given string
    fn check_word(&self, word: &str) -> bool {
        let token = self.peek();
        let lexeme = self.interner.resolve(token.lexeme);
        lexeme.eq_ignore_ascii_case(word)
    }

    fn check_to_preposition(&self) -> bool {
        if let TokenType::Preposition(p) = self.peek().kind {
            p.is(self.interner, "to")
        } else {
            false
        }
    }

    fn check_content_word(&self) -> bool {
        match &self.peek().kind {
            TokenType::Noun(_)
            | TokenType::Adjective(_)
            | TokenType::NonIntersectiveAdjective(_)
            | TokenType::Verb { .. }
            | TokenType::ProperName(_)
            | TokenType::Article(_) => true,
            TokenType::Ambiguous { primary, alternatives } => {
                Self::is_content_word_type(primary)
                    || alternatives.iter().any(Self::is_content_word_type)
            }
            _ => false,
        }
    }

    fn is_content_word_type(t: &TokenType) -> bool {
        matches!(
            t,
            TokenType::Noun(_)
                | TokenType::Adjective(_)
                | TokenType::NonIntersectiveAdjective(_)
                | TokenType::Verb { .. }
                | TokenType::ProperName(_)
                | TokenType::Article(_)
        )
    }

    fn check_verb(&self) -> bool {
        match &self.peek().kind {
            TokenType::Verb { .. } => true,
            TokenType::Ambiguous { primary, alternatives } => {
                if self.noun_priority_mode {
                    return false;
                }
                matches!(**primary, TokenType::Verb { .. })
                    || alternatives.iter().any(|t| matches!(t, TokenType::Verb { .. }))
            }
            _ => false,
        }
    }

    fn check_adverb(&self) -> bool {
        matches!(self.peek().kind, TokenType::Adverb(_))
    }

    fn check_performative(&self) -> bool {
        matches!(self.peek().kind, TokenType::Performative(_))
    }

    fn collect_adverbs(&mut self) -> Vec<Symbol> {
        let mut adverbs = Vec::new();
        while self.check_adverb() {
            if let TokenType::Adverb(adv) = self.advance().kind.clone() {
                adverbs.push(adv);
            }
            // Skip "and" between adverbs
            if self.check(&TokenType::And) {
                self.advance();
            }
        }
        adverbs
    }

    fn check_auxiliary(&self) -> bool {
        matches!(self.peek().kind, TokenType::Auxiliary(_))
    }

    fn check_to(&self) -> bool {
        matches!(self.peek().kind, TokenType::To)
    }


    fn consume_verb(&mut self) -> Symbol {
        let t = self.advance().clone();
        match t.kind {
            TokenType::Verb { lemma, .. } => lemma,
            TokenType::Ambiguous { primary, .. } => match *primary {
                TokenType::Verb { lemma, .. } => lemma,
                _ => panic!("Expected verb in Ambiguous primary, got {:?}", primary),
            },
            _ => panic!("Expected verb, got {:?}", t.kind),
        }
    }

    fn consume_verb_with_metadata(&mut self) -> (Symbol, Time, Aspect, VerbClass) {
        let t = self.advance().clone();
        match t.kind {
            TokenType::Verb { lemma, time, aspect, class } => (lemma, time, aspect, class),
            TokenType::Ambiguous { primary, .. } => match *primary {
                TokenType::Verb { lemma, time, aspect, class } => (lemma, time, aspect, class),
                _ => panic!("Expected verb in Ambiguous primary, got {:?}", primary),
            },
            _ => panic!("Expected verb, got {:?}", t.kind),
        }
    }

    fn match_token(&mut self, types: &[TokenType]) -> bool {
        for t in types {
            if self.check(t) {
                self.advance();
                return true;
            }
        }
        false
    }

    fn check_quantifier(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::All
                | TokenType::No
                | TokenType::Some
                | TokenType::Any
                | TokenType::Most
                | TokenType::Few
                | TokenType::Many
                | TokenType::Cardinal(_)
                | TokenType::AtLeast(_)
                | TokenType::AtMost(_)
        )
    }

    fn check_npi_quantifier(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Nobody | TokenType::Nothing | TokenType::NoOne
        )
    }

    fn check_npi_object(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Anything | TokenType::Anyone
        )
    }

    fn check_temporal_npi(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Ever | TokenType::Never
        )
    }

    fn parse_npi_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let npi_token = self.advance().kind.clone();
        let var_name = self.next_var_name();

        let (restriction_name, is_person) = match npi_token {
            TokenType::Nobody | TokenType::NoOne => ("Person", true),
            TokenType::Nothing => ("Thing", false),
            _ => ("Thing", false),
        };

        let restriction_sym = self.interner.intern(restriction_name);
        let subject_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: restriction_sym,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
        });

        self.negative_depth += 1;

        let verb = self.consume_verb();

        if self.check_npi_object() {
            let obj_npi_token = self.advance().kind.clone();
            let obj_var = self.next_var_name();

            let obj_restriction_name = match obj_npi_token {
                TokenType::Anything => "Thing",
                TokenType::Anyone => "Person",
                _ => "Thing",
            };

            let obj_restriction_sym = self.interner.intern(obj_restriction_name);
            let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: obj_restriction_sym,
                args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
            });

            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Variable(var_name), Term::Variable(obj_var)]),
            });

            let verb_and_obj = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: obj_restriction,
                op: TokenType::And,
                right: verb_pred,
            });

            let inner_existential = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: crate::ast::QuantifierKind::Existential,
                variable: obj_var,
                body: verb_and_obj,
                island_id: self.current_island,
            });

            self.negative_depth -= 1;

            let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: inner_existential,
            });

            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: subject_pred,
                op: TokenType::If,
                right: negated,
            });

            return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: crate::ast::QuantifierKind::Universal,
                variable: var_name,
                body,
                island_id: self.current_island,
            }));
        }

        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
        });

        self.negative_depth -= 1;

        let negated_verb = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
            op: TokenType::Not,
            operand: verb_pred,
        });

        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: subject_pred,
            op: TokenType::If,
            right: negated_verb,
        });

        Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind: crate::ast::QuantifierKind::Universal,
            variable: var_name,
            body,
            island_id: self.current_island,
        }))
    }

    fn parse_temporal_npi(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let npi_token = self.advance().kind.clone();
        let is_never = matches!(npi_token, TokenType::Never);

        let subject = self.parse_noun_phrase(true)?;

        if is_never {
            self.negative_depth += 1;
        }

        let verb = self.consume_verb();
        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
        });

        if is_never {
            self.negative_depth -= 1;
            Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: verb_pred,
            }))
        } else {
            Ok(verb_pred)
        }
    }

    fn check(&self, kind: &TokenType) -> bool {
        if self.is_at_end() {
            return false;
        }
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn check_any(&self, kinds: &[TokenType]) -> bool {
        if self.is_at_end() {
            return false;
        }
        let current = std::mem::discriminant(&self.peek().kind);
        kinds.iter().any(|k| std::mem::discriminant(k) == current)
    }

    fn check_article(&self) -> bool {
        matches!(self.peek().kind, TokenType::Article(_))
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn is_at_end(&self) -> bool {
        self.peek().kind == TokenType::EOF
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    /// Phase 35: Check if the next token (after current) is a string literal.
    /// Used to distinguish causal `because` from Trust's `because "reason"`.
    fn peek_next_is_string_literal(&self) -> bool {
        self.tokens.get(self.current + 1)
            .map(|t| matches!(t.kind, TokenType::StringLiteral(_)))
            .unwrap_or(false)
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn current_span(&self) -> crate::token::Span {
        self.peek().span
    }

    fn consume(&mut self, kind: TokenType) -> ParseResult<&Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    expected: kind,
                    found: self.peek().kind.clone(),
                },
                span: self.current_span(),
            })
        }
    }

    fn consume_content_word(&mut self) -> ParseResult<Symbol> {
        let t = self.advance().clone();
        match t.kind {
            TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::NonIntersectiveAdjective(s) => Ok(s),
            // Phase 35: Allow single-letter articles (a, an) to be used as variable names
            TokenType::Article(_) => Ok(t.lexeme),
            // Phase 35: Allow numeric literals as content words (e.g., "equal to 42")
            TokenType::Number(s) => Ok(s),
            TokenType::ProperName(s) => {
                let s_str = self.interner.resolve(s);

                // In imperative mode, reject unknown or moved entities
                if self.mode == ParserMode::Imperative {
                    use crate::context::OwnershipState;

                    let is_known = self.context.as_ref()
                        .map(|ctx| ctx.has_entity_by_noun_class(s_str))
                        .unwrap_or(false);

                    if !is_known {
                        return Err(ParseError {
                            kind: ParseErrorKind::UndefinedVariable { name: s_str.to_string() },
                            span: self.current_span(),
                        });
                    }

                    // Check for use-after-move
                    let ownership = self.context.as_ref()
                        .and_then(|ctx| ctx.get_ownership(s_str));

                    if ownership == Some(OwnershipState::Moved) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UseAfterMove { name: s_str.to_string() },
                            span: self.current_span(),
                        });
                    }
                }

                let gender = Self::infer_gender(s_str);
                let symbol_str = s_str.chars().next().unwrap().to_string();
                let noun_class = s_str.to_string();
                self.register_entity(&symbol_str, &noun_class, gender, Number::Singular);
                Ok(s)
            }
            TokenType::Verb { lemma, .. } => Ok(lemma),
            TokenType::Ambiguous { primary, .. } => {
                match *primary {
                    TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::NonIntersectiveAdjective(s) => Ok(s),
                    TokenType::Verb { lemma, .. } => Ok(lemma),
                    TokenType::ProperName(s) => Ok(s),
                    _ => Err(ParseError {
                        kind: ParseErrorKind::ExpectedContentWord { found: *primary },
                        span: self.current_span(),
                    }),
                }
            }
            other => Err(ParseError {
                kind: ParseErrorKind::ExpectedContentWord { found: other },
                span: self.current_span(),
            }),
        }
    }

    fn consume_copula(&mut self) -> ParseResult<()> {
        if self.match_token(&[TokenType::Is, TokenType::Are, TokenType::Was, TokenType::Were]) {
            Ok(())
        } else {
            Err(ParseError {
                kind: ParseErrorKind::ExpectedCopula,
                span: self.current_span(),
            })
        }
    }

    fn check_comparative(&self) -> bool {
        matches!(self.peek().kind, TokenType::Comparative(_))
    }

    fn is_contact_clause_pattern(&self) -> bool {
        // Detect "The cat [the dog chased] ran" pattern
        // Also handles nested: "The rat [the cat [the dog chased] ate] died"
        let mut pos = self.current;

        // Skip the article we're at
        if pos < self.tokens.len() && matches!(self.tokens[pos].kind, TokenType::Article(_)) {
            pos += 1;
        } else {
            return false;
        }

        // Skip adjectives
        while pos < self.tokens.len() && matches!(self.tokens[pos].kind, TokenType::Adjective(_)) {
            pos += 1;
        }

        // Must have noun/proper name (embedded subject)
        if pos < self.tokens.len() && matches!(self.tokens[pos].kind, TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Adjective(_)) {
            pos += 1;
        } else {
            return false;
        }

        // Must have verb OR another article (nested contact clause) after
        pos < self.tokens.len() && matches!(self.tokens[pos].kind, TokenType::Verb { .. } | TokenType::Article(_))
    }

    fn check_superlative(&self) -> bool {
        matches!(self.peek().kind, TokenType::Superlative(_))
    }

    fn check_scopal_adverb(&self) -> bool {
        matches!(self.peek().kind, TokenType::ScopalAdverb(_))
    }

    fn check_temporal_adverb(&self) -> bool {
        matches!(self.peek().kind, TokenType::TemporalAdverb(_))
    }

    fn check_non_intersective_adjective(&self) -> bool {
        matches!(self.peek().kind, TokenType::NonIntersectiveAdjective(_))
    }

    fn check_focus(&self) -> bool {
        matches!(self.peek().kind, TokenType::Focus(_))
    }

    fn check_measure(&self) -> bool {
        matches!(self.peek().kind, TokenType::Measure(_))
    }

    fn check_presup_trigger(&self) -> bool {
        match &self.peek().kind {
            TokenType::PresupTrigger(_) => true,
            TokenType::Verb { lemma, .. } => {
                let s = self.interner.resolve(*lemma).to_lowercase();
                crate::lexicon::lookup_presup_trigger(&s).is_some()
            }
            _ => false,
        }
    }

    fn is_followed_by_np_object(&self) -> bool {
        if self.current + 1 >= self.tokens.len() {
            return false;
        }
        let next = &self.tokens[self.current + 1].kind;
        matches!(next,
            TokenType::ProperName(_) |
            TokenType::Article(_) |
            TokenType::Noun(_) |
            TokenType::Pronoun { .. } |
            TokenType::Reflexive |
            TokenType::Who |
            TokenType::What |
            TokenType::Where |
            TokenType::When |
            TokenType::Why
        )
    }

    fn is_followed_by_gerund(&self) -> bool {
        if self.current + 1 >= self.tokens.len() {
            return false;
        }
        matches!(self.tokens[self.current + 1].kind, TokenType::Verb { .. })
    }

    // =========================================================================
    // Phase 46: Agent System Parsing
    // =========================================================================

    /// Parse spawn statement: "Spawn a Worker called 'w1'."
    fn parse_spawn_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Spawn"

        // Expect article (a/an)
        if !self.check_article() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "a/an".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume article

        // Get agent type name (Noun or ProperName)
        let agent_type = match &self.tokens[self.current].kind {
            TokenType::Noun(sym) | TokenType::ProperName(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "agent type".to_string() },
                    span: self.current_span(),
                });
            }
        };

        // Expect "called"
        if !self.check(&TokenType::Called) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "called".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "called"

        // Get agent name (string literal)
        let name = if let TokenType::StringLiteral(sym) = &self.tokens[self.current].kind {
            let s = *sym;
            self.advance();
            s
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "agent name".to_string() },
                span: self.current_span(),
            });
        };

        Ok(Stmt::Spawn { agent_type, name })
    }

    /// Parse send statement: "Send Ping to 'agent'."
    fn parse_send_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Send"

        // Parse message expression
        let message = self.parse_imperative_expr()?;

        // Expect "to"
        if !self.check_preposition_is("to") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "to"

        // Parse destination expression
        let destination = self.parse_imperative_expr()?;

        Ok(Stmt::SendMessage { message, destination })
    }

    /// Parse await statement: "Await response from 'agent' into result."
    fn parse_await_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Await"

        // Skip optional "response" word
        if self.check_word("response") {
            self.advance();
        }

        // Expect "from" (can be keyword or preposition)
        if !self.check(&TokenType::From) && !self.check_preposition_is("from") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "from".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "from"

        // Parse source expression
        let source = self.parse_imperative_expr()?;

        // Expect "into"
        if !self.check_word("into") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "into".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "into"

        // Get variable name (Noun, ProperName, or Adjective - can be any content word)
        let into = match &self.tokens[self.current].kind {
            TokenType::Noun(sym) | TokenType::ProperName(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
                self.advance();
                s
            }
            // Also accept lexemes from other token types if they look like identifiers
            _ if self.check_content_word() => {
                let sym = self.tokens[self.current].lexeme;
                self.advance();
                sym
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword { keyword: "variable name".to_string() },
                    span: self.current_span(),
                });
            }
        };

        Ok(Stmt::AwaitMessage { source, into })
    }

    // =========================================================================
    // Phase 49: CRDT Statement Parsing
    // =========================================================================

    /// Parse merge statement: "Merge remote into local." or "Merge remote's field into local's field."
    fn parse_merge_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Merge"

        // Parse source expression
        let source = self.parse_imperative_expr()?;

        // Expect "into"
        if !self.check_word("into") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "into".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "into"

        // Parse target expression
        let target = self.parse_imperative_expr()?;

        Ok(Stmt::MergeCrdt { source, target })
    }

    /// Parse increase statement: "Increase local's points by 10."
    fn parse_increase_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Increase"

        // Parse object with field access (e.g., "local's points")
        let expr = self.parse_imperative_expr()?;

        // Must be a field access
        let (object, field) = if let Expr::FieldAccess { object, field } = expr {
            (object, field)
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "field access (e.g., 'x's count')".to_string() },
                span: self.current_span(),
            });
        };

        // Expect "by"
        if !self.check_preposition_is("by") {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "by".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "by"

        // Parse amount
        let amount = self.parse_imperative_expr()?;

        Ok(Stmt::IncreaseCrdt { object, field: *field, amount })
    }

}

