use super::*;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Literal};
use crate::optimize::facts::{self, Fact};

#[test]
fn rebuilding_an_expression_preserves_every_optimizer_fact() {
    const EXPRESSION_FACTS: [Fact; 5] = [
        Fact::RawIntOp,
        Fact::IntStrengthAdvance,
        Fact::BlackBox,
        Fact::InBounds,
        Fact::TileLegal,
    ];

    let exprs = Arena::new();
    let stmts = Arena::new();
    let types = Arena::new();
    let left = exprs.alloc(Expr::Literal(Literal::Number(20)));
    let right = exprs.alloc(Expr::Literal(Literal::Number(22)));
    let original = exprs.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    });
    let registry = TypeRegistry::new();
    let instantiations = Instantiations::default();
    let mut interner = Interner::new();
    let twins = registry.namespaced_twins(&interner);
    let mut mono = Mono {
        arenas: Arenas {
            exprs: &exprs,
            stmts: &stmts,
            types: &types,
        },
        registry: &registry,
        inst: &instantiations,
        options: Options { lower_option: true },
        interner: &mut interner,
        generic_fns: HashMap::new(),
        taken: HashSet::new(),
        fn_instances: HashMap::new(),
        pending: std::collections::VecDeque::new(),
        instances_of: HashMap::new(),
        type_instances: HashMap::new(),
        out_registry: registry.clone(),
        instance_info: HashMap::new(),
        subst: HashMap::new(),
        param_types: HashMap::new(),
        error: None,
        twins,
    };
    let _scope = facts::scope();
    for fact in EXPRESSION_FACTS {
        facts::mark_expr(fact, original);
    }

    let mapped = mono.map_expr(original);

    assert!(
        !std::ptr::eq(original, mapped),
        "the test must exercise the rebuilding path"
    );
    for fact in EXPRESSION_FACTS {
        assert!(
            facts::holds_expr(fact, mapped),
            "monomorphization dropped {fact:?}"
        );
    }
}
