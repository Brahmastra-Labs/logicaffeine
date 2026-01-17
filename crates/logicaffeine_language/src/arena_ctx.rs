//! Arena context for AST allocation.
//!
//! This module provides [`AstContext`], a collection of typed arenas used during
//! parsing to allocate AST nodes. All nodes are bump-allocated for efficiency,
//! with the `'a` lifetime tracking the arena's scope.
//!
//! The context contains separate arenas for:
//! - Logical expressions ([`LogicExpr`])
//! - Terms ([`Term`])
//! - Noun phrases ([`NounPhrase`])
//! - Symbols, thematic roles, and prepositional phrases
//! - Imperative statements (optional, for LOGOS mode)

use logicaffeine_base::Arena;
use crate::ast::{AspectOperator, LogicExpr, ModalVector, NounPhrase, QuantifierKind, TemporalOperator, VoiceOperator, Term, ThematicRole, Stmt, Expr, TypeExpr};
use logicaffeine_base::Symbol;
use crate::token::TokenType;

/// Collection of typed arenas for AST allocation during parsing.
///
/// The context holds references to multiple arenas, each specialized for
/// a particular AST node type. This separation allows efficient allocation
/// while maintaining type safety.
///
/// # Modes
///
/// The context supports two modes:
/// - **Declarative** (default): For natural language parsing to logic
/// - **Imperative**: Adds statement and expression arenas for LOGOS programs
#[derive(Clone, Copy)]
pub struct AstContext<'a> {
    /// Arena for logical expressions ([`LogicExpr`]).
    pub exprs: &'a Arena<LogicExpr<'a>>,
    /// Arena for first-order terms ([`Term`]).
    pub terms: &'a Arena<Term<'a>>,
    /// Arena for noun phrases ([`NounPhrase`]).
    pub nps: &'a Arena<NounPhrase<'a>>,
    /// Arena for interned symbols.
    pub syms: &'a Arena<Symbol>,
    /// Arena for thematic role assignments.
    pub roles: &'a Arena<(ThematicRole, Term<'a>)>,
    /// Arena for prepositional phrase modifiers.
    pub pps: &'a Arena<&'a LogicExpr<'a>>,
    /// Optional arena for imperative statements (LOGOS mode).
    pub stmts: Option<&'a Arena<Stmt<'a>>>,
    /// Optional arena for imperative expressions (LOGOS mode).
    pub imperative_exprs: Option<&'a Arena<Expr<'a>>>,
    /// Optional arena for type expressions (LOGOS mode).
    pub type_exprs: Option<&'a Arena<TypeExpr<'a>>>,
}

impl<'a> AstContext<'a> {
    /// Creates a new context for declarative (natural language) parsing.
    pub fn new(
        exprs: &'a Arena<LogicExpr<'a>>,
        terms: &'a Arena<Term<'a>>,
        nps: &'a Arena<NounPhrase<'a>>,
        syms: &'a Arena<Symbol>,
        roles: &'a Arena<(ThematicRole, Term<'a>)>,
        pps: &'a Arena<&'a LogicExpr<'a>>,
    ) -> Self {
        AstContext { exprs, terms, nps, syms, roles, pps, stmts: None, imperative_exprs: None, type_exprs: None }
    }

    /// Creates a context with imperative statement arenas for LOGOS programs.
    pub fn with_imperative(
        exprs: &'a Arena<LogicExpr<'a>>,
        terms: &'a Arena<Term<'a>>,
        nps: &'a Arena<NounPhrase<'a>>,
        syms: &'a Arena<Symbol>,
        roles: &'a Arena<(ThematicRole, Term<'a>)>,
        pps: &'a Arena<&'a LogicExpr<'a>>,
        stmts: &'a Arena<Stmt<'a>>,
        imperative_exprs: &'a Arena<Expr<'a>>,
    ) -> Self {
        AstContext { exprs, terms, nps, syms, roles, pps, stmts: Some(stmts), imperative_exprs: Some(imperative_exprs), type_exprs: None }
    }

    /// Creates a context with type expression arena for typed LOGOS programs.
    pub fn with_types(
        exprs: &'a Arena<LogicExpr<'a>>,
        terms: &'a Arena<Term<'a>>,
        nps: &'a Arena<NounPhrase<'a>>,
        syms: &'a Arena<Symbol>,
        roles: &'a Arena<(ThematicRole, Term<'a>)>,
        pps: &'a Arena<&'a LogicExpr<'a>>,
        stmts: &'a Arena<Stmt<'a>>,
        imperative_exprs: &'a Arena<Expr<'a>>,
        type_exprs: &'a Arena<TypeExpr<'a>>,
    ) -> Self {
        AstContext {
            exprs, terms, nps, syms, roles, pps,
            stmts: Some(stmts),
            imperative_exprs: Some(imperative_exprs),
            type_exprs: Some(type_exprs),
        }
    }

    /// Allocates an imperative statement.
    ///
    /// # Panics
    /// Panics if imperative arenas were not initialized.
    pub fn alloc_stmt(&self, stmt: Stmt<'a>) -> &'a Stmt<'a> {
        self.stmts.expect("imperative arenas not initialized").alloc(stmt)
    }

    /// Allocates an imperative expression.
    ///
    /// # Panics
    /// Panics if imperative arenas were not initialized.
    pub fn alloc_imperative_expr(&self, expr: Expr<'a>) -> &'a Expr<'a> {
        self.imperative_exprs.expect("imperative arenas not initialized").alloc(expr)
    }

    /// Allocates a type expression.
    ///
    /// # Panics
    /// Panics if type expression arena was not initialized.
    pub fn alloc_type_expr(&self, ty: TypeExpr<'a>) -> &'a TypeExpr<'a> {
        self.type_exprs.expect("type_exprs arena not initialized").alloc(ty)
    }

    /// Allocates a slice of type expressions.
    ///
    /// # Panics
    /// Panics if type expression arena was not initialized.
    pub fn alloc_type_exprs<I>(&self, types: I) -> &'a [TypeExpr<'a>]
    where
        I: IntoIterator<Item = TypeExpr<'a>>,
        I::IntoIter: ExactSizeIterator,
    {
        self.type_exprs.expect("type_exprs arena not initialized").alloc_slice(types)
    }

    /// Allocates a logical expression.
    pub fn alloc_expr(&self, expr: LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(expr)
    }

    /// Allocates a first-order term.
    pub fn alloc_term(&self, term: Term<'a>) -> &'a Term<'a> {
        self.terms.alloc(term)
    }

    /// Allocates a slice of terms from an iterator.
    pub fn alloc_terms<I>(&self, terms: I) -> &'a [Term<'a>]
    where
        I: IntoIterator<Item = Term<'a>>,
        I::IntoIter: ExactSizeIterator,
    {
        self.terms.alloc_slice(terms)
    }

    /// Allocates a noun phrase.
    pub fn alloc_np(&self, np: NounPhrase<'a>) -> &'a NounPhrase<'a> {
        self.nps.alloc(np)
    }

    /// Allocates a slice of symbols from an iterator.
    pub fn alloc_syms<I>(&self, syms: I) -> &'a [Symbol]
    where
        I: IntoIterator<Item = Symbol>,
        I::IntoIter: ExactSizeIterator,
    {
        self.syms.alloc_slice(syms)
    }

    /// Allocates a slice of thematic role assignments.
    pub fn alloc_roles<I>(&self, roles: I) -> &'a [(ThematicRole, Term<'a>)]
    where
        I: IntoIterator<Item = (ThematicRole, Term<'a>)>,
        I::IntoIter: ExactSizeIterator,
    {
        self.roles.alloc_slice(roles)
    }

    /// Allocates a slice of prepositional phrase modifiers.
    pub fn alloc_pps<I>(&self, pps: I) -> &'a [&'a LogicExpr<'a>]
    where
        I: IntoIterator<Item = &'a LogicExpr<'a>>,
        I::IntoIter: ExactSizeIterator,
    {
        self.pps.alloc_slice(pps)
    }

    /// Creates an atomic predicate: `P(t1, t2, ...)`.
    pub fn predicate(&self, name: Symbol, args: &'a [Term<'a>]) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Predicate { name, args, world: None })
    }

    /// Creates a binary operation: `left op right`.
    #[inline(always)]
    pub fn binary(&self, left: &'a LogicExpr<'a>, op: TokenType, right: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::BinaryOp { left, op, right })
    }

    /// Creates a unary operation: `op operand`.
    #[inline(always)]
    pub fn unary(&self, op: TokenType, operand: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::UnaryOp { op, operand })
    }

    /// Creates a quantified formula: `∀x.body` or `∃x.body`.
    #[inline(always)]
    pub fn quantifier(&self, kind: QuantifierKind, variable: Symbol, body: &'a LogicExpr<'a>, island_id: u32) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Quantifier { kind, variable, body, island_id })
    }

    /// Creates a temporal operator: `PAST(body)` or `FUTURE(body)`.
    #[inline(always)]
    pub fn temporal(&self, operator: TemporalOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Temporal { operator, body })
    }

    /// Creates an aspectual operator: `PROG(body)`, `PERF(body)`, etc.
    #[inline(always)]
    pub fn aspectual(&self, operator: AspectOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Aspectual { operator, body })
    }

    /// Creates a voice operator: `PASSIVE(body)`.
    #[inline(always)]
    pub fn voice(&self, operator: VoiceOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Voice { operator, body })
    }

    /// Creates a modal operator: `□operand` or `◇operand`.
    #[inline(always)]
    pub fn modal(&self, vector: ModalVector, operand: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Modal { vector, operand })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{QuantifierKind, TemporalOperator, AspectOperator, ModalVector, ModalDomain};
    use logicaffeine_base::Interner;
    use crate::token::TokenType;

    fn setup<'a>(
        expr_arena: &'a Arena<LogicExpr<'a>>,
        term_arena: &'a Arena<Term<'a>>,
        np_arena: &'a Arena<NounPhrase<'a>>,
        sym_arena: &'a Arena<Symbol>,
        role_arena: &'a Arena<(ThematicRole, Term<'a>)>,
        pp_arena: &'a Arena<&'a LogicExpr<'a>>,
    ) -> AstContext<'a> {
        AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena)
    }

    #[test]
    fn binary_builder_creates_binary_op() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let q = interner.intern("Q");

        let left = ctx.alloc_expr(LogicExpr::Atom(p));
        let right = ctx.alloc_expr(LogicExpr::Atom(q));
        let result = ctx.binary(left, TokenType::And, right);

        assert!(matches!(result, LogicExpr::BinaryOp { op: TokenType::And, .. }));
    }

    #[test]
    fn unary_builder_creates_unary_op() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let operand = ctx.alloc_expr(LogicExpr::Atom(p));
        let result = ctx.unary(TokenType::Not, operand);

        assert!(matches!(result, LogicExpr::UnaryOp { op: TokenType::Not, .. }));
    }

    #[test]
    fn quantifier_builder_creates_quantifier() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let x = interner.intern("x");
        let p = interner.intern("P");
        let body = ctx.alloc_expr(LogicExpr::Atom(p));
        let result = ctx.quantifier(QuantifierKind::Universal, x, body, 0);

        assert!(matches!(result, LogicExpr::Quantifier { kind: QuantifierKind::Universal, .. }));
    }

    #[test]
    fn temporal_builder_creates_temporal() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let body = ctx.alloc_expr(LogicExpr::Atom(p));
        let result = ctx.temporal(TemporalOperator::Past, body);

        assert!(matches!(result, LogicExpr::Temporal { operator: TemporalOperator::Past, .. }));
    }

    #[test]
    fn aspectual_builder_creates_aspectual() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let body = ctx.alloc_expr(LogicExpr::Atom(p));
        let result = ctx.aspectual(AspectOperator::Progressive, body);

        assert!(matches!(result, LogicExpr::Aspectual { operator: AspectOperator::Progressive, .. }));
    }

    #[test]
    fn modal_builder_creates_modal() {
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let np_arena: Arena<NounPhrase> = Arena::new();
        let sym_arena: Arena<Symbol> = Arena::new();
        let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
        let pp_arena: Arena<&LogicExpr> = Arena::new();
        let ctx = setup(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let operand = ctx.alloc_expr(LogicExpr::Atom(p));
        let vector = ModalVector { domain: ModalDomain::Alethic, force: 1.0, flavor: crate::ast::ModalFlavor::Root };
        let result = ctx.modal(vector, operand);

        assert!(matches!(result, LogicExpr::Modal { .. }));
    }
}
