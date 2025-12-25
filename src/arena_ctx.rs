use crate::arena::Arena;
use crate::ast::{AspectOperator, LogicExpr, ModalVector, NounPhrase, QuantifierKind, TemporalOperator, VoiceOperator, Term, ThematicRole, Stmt, Expr};
use crate::intern::Symbol;
use crate::token::TokenType;

#[derive(Clone, Copy)]
pub struct AstContext<'a> {
    pub exprs: &'a Arena<LogicExpr<'a>>,
    pub terms: &'a Arena<Term<'a>>,
    pub nps: &'a Arena<NounPhrase<'a>>,
    pub syms: &'a Arena<Symbol>,
    pub roles: &'a Arena<(ThematicRole, Term<'a>)>,
    pub pps: &'a Arena<&'a LogicExpr<'a>>,
    pub stmts: Option<&'a Arena<Stmt<'a>>>,
    pub imperative_exprs: Option<&'a Arena<Expr<'a>>>,
}

impl<'a> AstContext<'a> {
    pub fn new(
        exprs: &'a Arena<LogicExpr<'a>>,
        terms: &'a Arena<Term<'a>>,
        nps: &'a Arena<NounPhrase<'a>>,
        syms: &'a Arena<Symbol>,
        roles: &'a Arena<(ThematicRole, Term<'a>)>,
        pps: &'a Arena<&'a LogicExpr<'a>>,
    ) -> Self {
        AstContext { exprs, terms, nps, syms, roles, pps, stmts: None, imperative_exprs: None }
    }

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
        AstContext { exprs, terms, nps, syms, roles, pps, stmts: Some(stmts), imperative_exprs: Some(imperative_exprs) }
    }

    pub fn alloc_stmt(&self, stmt: Stmt<'a>) -> &'a Stmt<'a> {
        self.stmts.expect("imperative arenas not initialized").alloc(stmt)
    }

    pub fn alloc_imperative_expr(&self, expr: Expr<'a>) -> &'a Expr<'a> {
        self.imperative_exprs.expect("imperative arenas not initialized").alloc(expr)
    }

    pub fn alloc_expr(&self, expr: LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(expr)
    }

    pub fn alloc_term(&self, term: Term<'a>) -> &'a Term<'a> {
        self.terms.alloc(term)
    }

    pub fn alloc_terms<I>(&self, terms: I) -> &'a [Term<'a>]
    where
        I: IntoIterator<Item = Term<'a>>,
        I::IntoIter: ExactSizeIterator,
    {
        self.terms.alloc_slice(terms)
    }

    pub fn alloc_np(&self, np: NounPhrase<'a>) -> &'a NounPhrase<'a> {
        self.nps.alloc(np)
    }

    pub fn alloc_syms<I>(&self, syms: I) -> &'a [Symbol]
    where
        I: IntoIterator<Item = Symbol>,
        I::IntoIter: ExactSizeIterator,
    {
        self.syms.alloc_slice(syms)
    }

    pub fn alloc_roles<I>(&self, roles: I) -> &'a [(ThematicRole, Term<'a>)]
    where
        I: IntoIterator<Item = (ThematicRole, Term<'a>)>,
        I::IntoIter: ExactSizeIterator,
    {
        self.roles.alloc_slice(roles)
    }

    pub fn alloc_pps<I>(&self, pps: I) -> &'a [&'a LogicExpr<'a>]
    where
        I: IntoIterator<Item = &'a LogicExpr<'a>>,
        I::IntoIter: ExactSizeIterator,
    {
        self.pps.alloc_slice(pps)
    }

    pub fn predicate(&self, name: Symbol, args: &'a [Term<'a>]) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Predicate { name, args })
    }

    #[inline(always)]
    pub fn binary(&self, left: &'a LogicExpr<'a>, op: TokenType, right: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::BinaryOp { left, op, right })
    }

    #[inline(always)]
    pub fn unary(&self, op: TokenType, operand: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::UnaryOp { op, operand })
    }

    #[inline(always)]
    pub fn quantifier(&self, kind: QuantifierKind, variable: Symbol, body: &'a LogicExpr<'a>, island_id: u32) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Quantifier { kind, variable, body, island_id })
    }

    #[inline(always)]
    pub fn temporal(&self, operator: TemporalOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Temporal { operator, body })
    }

    #[inline(always)]
    pub fn aspectual(&self, operator: AspectOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Aspectual { operator, body })
    }

    #[inline(always)]
    pub fn voice(&self, operator: VoiceOperator, body: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Voice { operator, body })
    }

    #[inline(always)]
    pub fn modal(&self, vector: ModalVector, operand: &'a LogicExpr<'a>) -> &'a LogicExpr<'a> {
        self.exprs.alloc(LogicExpr::Modal { vector, operand })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{QuantifierKind, TemporalOperator, AspectOperator, ModalVector, ModalDomain};
    use crate::intern::Interner;
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
        let vector = ModalVector { domain: ModalDomain::Alethic, force: 1.0 };
        let result = ctx.modal(vector, operand);

        assert!(matches!(result, LogicExpr::Modal { .. }));
    }
}
