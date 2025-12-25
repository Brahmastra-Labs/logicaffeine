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

use crate::arena_ctx::AstContext;
use crate::ast::{AspectOperator, LogicExpr, NeoEventData, QuantifierKind, TemporalOperator, Term, ThematicRole, Stmt, Expr, Literal};
use crate::context::{Case, DiscourseContext, Entity, Gender, Number};
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::{Interner, Symbol, SymbolEq};
use crate::lexer::Lexer;
use crate::lexicon::{self, Aspect, Definiteness, Time, VerbClass};
use crate::token::{FocusKind, Token, TokenType};

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
        }
    }

    pub fn set_noun_priority_mode(&mut self, mode: bool) {
        self.noun_priority_mode = mode;
    }

    pub fn set_collective_mode(&mut self, mode: bool) {
        self.collective_mode = mode;
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
        }
    }

    pub fn set_discourse_event_var(&mut self, var: Symbol) {
        self.discourse_event_var = Some(var);
    }

    pub fn mode(&self) -> ParserMode {
        self.mode
    }

    pub fn process_block_headers(&mut self) {
        use crate::token::BlockType;

        while self.current < self.tokens.len() {
            if let TokenType::BlockHeader { block_type } = &self.tokens[self.current].kind {
                self.mode = match block_type {
                    BlockType::Main => ParserMode::Imperative,
                    BlockType::Theorem | BlockType::Definition | BlockType::Proof |
                    BlockType::Example | BlockType::Logic | BlockType::Note => ParserMode::Declarative,
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

        while !self.is_at_end() {
            let stmt = self.parse_statement()?;
            statements.push(stmt);

            if self.check(&TokenType::Period) {
                self.advance();
            }
        }

        Ok(statements)
    }

    fn parse_statement(&mut self) -> ParseResult<Stmt<'a>> {
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
        if self.check(&TokenType::While) {
            return self.parse_while_statement();
        }
        if self.check(&TokenType::Call) {
            return self.parse_call_statement();
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

        Ok(Stmt::While { cond, body })
    }

    fn parse_call_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Call"

        // Parse function name (identifier)
        let function = match &self.peek().kind {
            TokenType::Noun(sym) | TokenType::Adjective(sym) => {
                let s = *sym;
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
        use crate::ast::stmt::BinaryOpKind as ImperativeBinOp;

        // Parse left side (identifier)
        let left = self.parse_imperative_expr()?;

        // Check for "equals"
        if self.check(&TokenType::Equals) {
            self.advance();
            let right = self.parse_imperative_expr()?;
            Ok(self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                op: ImperativeBinOp::Eq,
                left,
                right,
            }))
        } else {
            // Just return the expression as the condition
            Ok(left)
        }
    }

    fn parse_let_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Let"

        // Get identifier
        let var = self.expect_identifier()?;

        // Expect "be"
        if !self.check(&TokenType::Be) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "be".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "be"

        // Parse expression value (simple: just a number for now)
        let value = self.parse_imperative_expr()?;

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

        Ok(Stmt::Let { var, value, mutable: false })
    }

    fn parse_set_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Set"

        // Get target identifier
        let target = self.expect_identifier()?;

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

        Ok(Stmt::Set { target, value })
    }

    fn parse_return_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Return"

        // Check if there's a value or just "Return."
        if self.check(&TokenType::Period) || self.is_at_end() {
            return Ok(Stmt::Return { value: None });
        }

        let value = self.parse_imperative_expr()?;
        Ok(Stmt::Return { value: Some(value) })
    }

    fn parse_assert_statement(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume "Assert"

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

        Ok(Stmt::Assert { proposition })
    }

    fn parse_imperative_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
        use crate::ast::{Expr, Literal};

        let token = self.peek().clone();
        match &token.kind {
            // Index access: "item N of collection"
            TokenType::Item => {
                self.advance(); // consume "item"

                // Parse index (must be a number)
                let index = if let TokenType::Number(sym) = &self.peek().kind {
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    num_str.parse::<usize>().unwrap_or(0)
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedExpression,
                        span: self.current_span(),
                    });
                };

                // Index 0 Guard: LOGOS uses 1-based indexing
                if index == 0 {
                    return Err(ParseError {
                        kind: ParseErrorKind::ZeroIndex,
                        span: self.current_span(),
                    });
                }

                // Expect "of"
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                // Parse collection
                let collection = self.parse_imperative_expr()?;

                Ok(self.ctx.alloc_imperative_expr(Expr::Index {
                    collection,
                    index,
                }))
            }

            // Slice access: "items N through M of collection"
            TokenType::Items => {
                self.advance(); // consume "items"

                // Parse start index (must be a number)
                let start = if let TokenType::Number(sym) = &self.peek().kind {
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    num_str.parse::<usize>().unwrap_or(0)
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedNumber,
                        span: self.current_span(),
                    });
                };

                // Index 0 Guard for start
                if start == 0 {
                    return Err(ParseError {
                        kind: ParseErrorKind::ZeroIndex,
                        span: self.current_span(),
                    });
                }

                // Expect "through"
                if !self.check_preposition_is("through") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "through".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "through"

                // Parse end index (must be a number)
                let end = if let TokenType::Number(sym) = &self.peek().kind {
                    let sym = *sym;
                    self.advance();
                    let num_str = self.interner.resolve(sym);
                    num_str.parse::<usize>().unwrap_or(0)
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedNumber,
                        span: self.current_span(),
                    });
                };

                // Index 0 Guard for end
                if end == 0 {
                    return Err(ParseError {
                        kind: ParseErrorKind::ZeroIndex,
                        span: self.current_span(),
                    });
                }

                // Expect "of"
                if !self.check_preposition_is("of") {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword { keyword: "of".to_string() },
                        span: self.current_span(),
                    });
                }
                self.advance(); // consume "of"

                // Parse collection
                let collection = self.parse_imperative_expr()?;

                Ok(self.ctx.alloc_imperative_expr(Expr::Slice {
                    collection,
                    start,
                    end,
                }))
            }

            TokenType::Number(sym) => {
                self.advance();
                let num_str = self.interner.resolve(*sym);
                let num = num_str.parse::<i64>().unwrap_or(0);
                Ok(self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Number(num))))
            }
            TokenType::Noun(sym) | TokenType::ProperName(sym) => {
                self.advance();
                Ok(self.ctx.alloc_imperative_expr(Expr::Identifier(*sym)))
            }
            _ => {
                // Try to get any identifier-like token
                if let TokenType::Adjective(sym) = &token.kind {
                    self.advance();
                    return Ok(self.ctx.alloc_imperative_expr(Expr::Identifier(*sym)));
                }
                Err(ParseError {
                    kind: ParseErrorKind::ExpectedExpression,
                    span: self.current_span(),
                })
            }
        }
    }

    fn expect_identifier(&mut self) -> ParseResult<Symbol> {
        let token = self.peek().clone();
        match &token.kind {
            TokenType::Noun(sym) | TokenType::ProperName(sym) | TokenType::Adjective(sym) => {
                self.advance();
                Ok(*sym)
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
            if let Some(result) = self.try_parse_plural_subject(&subject) {
                return Ok(result);
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

                let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate_noun,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
                });
                return self.wrap_with_definiteness(subject.definiteness, subject.noun, predicate);
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
                let term = self.noun_phrase_to_term(&object);
                object_term = Some(term.clone());
                args.push(term);

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

}

