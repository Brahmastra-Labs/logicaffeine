use super::*;
use std::collections::HashMap;

fn empty_types() -> HashMap<Symbol, TypeAbstraction> {
    HashMap::new()
}

fn union(tags: &[TypeTag]) -> TypeAbstraction {
    TypeAbstraction::Union(tags.iter().cloned().collect())
}

#[test]
fn literal_types_are_concrete() {
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Number(5)),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Int)
    );
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Float(1.5)),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Float)
    );
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Boolean(true)),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Bool)
    );
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Nothing),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Nothing)
    );
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Char('a')),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Char)
    );
    assert_eq!(
        eval_type(
            &Expr::Literal(Literal::Duration(60)),
            &empty_types(),
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Concrete(TypeTag::Duration)
    );
}

#[test]
fn arithmetic_and_comparison_result_types() {
    let five = Expr::Literal(Literal::Number(5));
    let three = Expr::Literal(Literal::Number(3));
    // Int + Int : Int
    let add = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &five,
        right: &three,
    };
    assert_eq!(
        eval_type(&add, &empty_types(), &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Concrete(TypeTag::Int)
    );
    // Int < Int : Bool
    let lt = Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: &five,
        right: &three,
    };
    assert_eq!(
        eval_type(&lt, &empty_types(), &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Concrete(TypeTag::Bool)
    );
    // Float + Float : Float
    let f1 = Expr::Literal(Literal::Float(1.0));
    let f2 = Expr::Literal(Literal::Float(2.0));
    let fadd = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &f1,
        right: &f2,
    };
    assert_eq!(
        eval_type(&fadd, &empty_types(), &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Concrete(TypeTag::Float)
    );
    // Int + Float : Top (we do not assume a coercion — conservative)
    let mixed = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &five,
        right: &f1,
    };
    assert_eq!(
        eval_type(&mixed, &empty_types(), &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Top
    );
    // Text + Int : Text (interpolating concat)
    let txt = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let concat = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &txt,
        right: &three,
    };
    assert_eq!(
        eval_type(&concat, &empty_types(), &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Concrete(TypeTag::Text)
    );
}

#[test]
fn let_binds_value_type_in_env() {
    // `Let x be 5.` ⇒ env[x] = Concrete(Int); a later use of x reads Int.
    let x = Symbol::EMPTY;
    let mut env = empty_types();
    let value = Expr::Literal(Literal::Number(5));
    env.insert(x, eval_type(&value, &env, &HashMap::new(), &HashMap::new()));
    assert_eq!(env.get(&x), Some(&TypeAbstraction::Concrete(TypeTag::Int)));
    // and an identifier reference resolves through the env
    assert_eq!(
        eval_type(&Expr::Identifier(x), &env, &HashMap::new(), &HashMap::new()),
        TypeAbstraction::Concrete(TypeTag::Int)
    );
    // an unbound identifier is Top (no information)
    let unbound = empty_types();
    assert_eq!(
        eval_type(
            &Expr::Identifier(x),
            &unbound,
            &HashMap::new(),
            &HashMap::new()
        ),
        TypeAbstraction::Top
    );
}

#[test]
fn type_join_merges_distinct_into_union_and_collapses() {
    let i = TypeAbstraction::Concrete(TypeTag::Int);
    let b = TypeAbstraction::Concrete(TypeTag::Bool);
    // distinct concretes join to a union
    assert_eq!(i.join(&b), union(&[TypeTag::Int, TypeTag::Bool]));
    // identical concretes stay concrete
    assert_eq!(i.join(&i), i);
    // a singleton union normalizes back to a concrete
    assert_eq!(union(&[TypeTag::Int]), i);
    // join with Top is Top; join with Bottom is identity
    assert_eq!(i.join(&TypeAbstraction::Top), TypeAbstraction::Top);
    assert_eq!(i.join(&TypeAbstraction::Bottom), i);
    assert_eq!(TypeAbstraction::Bottom.join(&i), i);
}

#[test]
fn type_meet_intersects() {
    let i = TypeAbstraction::Concrete(TypeTag::Int);
    let b = TypeAbstraction::Concrete(TypeTag::Bool);
    let ib = union(&[TypeTag::Int, TypeTag::Bool]);
    // Concrete ⊓ matching Union = Concrete
    assert_eq!(i.meet(&ib), i);
    // disjoint concretes meet to Bottom
    assert_eq!(i.meet(&b), TypeAbstraction::Bottom);
    // Top is identity for meet
    assert_eq!(TypeAbstraction::Top.meet(&i), i);
    // Bottom annihilates
    assert_eq!(TypeAbstraction::Bottom.meet(&i), TypeAbstraction::Bottom);
}

#[test]
fn type_lattice_order_and_laws() {
    let i = TypeAbstraction::Concrete(TypeTag::Int);
    let ib = union(&[TypeTag::Int, TypeTag::Bool]);
    // Bottom ⊑ everything ⊑ Top
    assert!(TypeAbstraction::Bottom.leq(&i));
    assert!(i.leq(&TypeAbstraction::Top));
    // Concrete ⊑ Union containing it
    assert!(i.leq(&ib));
    assert!(!ib.leq(&i));
    // reflexive
    assert!(i.leq(&i));
    // join is an upper bound; widen over-approximates join (finite lattice → widen == join)
    let b = TypeAbstraction::Concrete(TypeTag::Bool);
    assert!(i.leq(&i.join(&b)));
    assert!(b.leq(&i.join(&b)));
    assert_eq!(i.widen(&b), i.join(&b));
}
