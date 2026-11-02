use super::*;
use logicaffeine_base::Arena;

#[test]
fn resolve_term_constant() {
    let mut interner = Interner::new();
    let sym = interner.intern("Socrates");
    let term = Term::Constant(sym);
    assert_eq!(term.resolve(&interner), TermView::Constant("Socrates"));
}

#[test]
fn resolve_term_variable() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let term = Term::Variable(x);
    assert_eq!(term.resolve(&interner), TermView::Variable("x"));
}

#[test]
fn resolve_term_function() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let father = interner.intern("father");
    let john = interner.intern("John");
    let term = Term::Function(father, term_arena.alloc_slice([Term::Constant(john)]));

    assert_eq!(
        term.resolve(&interner),
        TermView::Function("father", vec![TermView::Constant("John")])
    );
}

#[test]
fn resolve_term_group() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let j = interner.intern("John");
    let m = interner.intern("Mary");
    let term = Term::Group(term_arena.alloc_slice([Term::Constant(j), Term::Constant(m)]));

    assert_eq!(
        term.resolve(&interner),
        TermView::Group(vec![TermView::Constant("John"), TermView::Constant("Mary")])
    );
}

#[test]
fn resolve_term_possessed() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let john = interner.intern("John");
    let dog = interner.intern("dog");
    let term = Term::Possessed {
        possessor: term_arena.alloc(Term::Constant(john)),
        possessed: dog,
    };

    assert_eq!(
        term.resolve(&interner),
        TermView::Possessed {
            possessor: Box::new(TermView::Constant("John")),
            possessed: "dog",
        }
    );
}

#[test]
fn term_view_equality_is_bit_exact() {
    let a = TermView::Constant("test");
    let b = TermView::Constant("test");
    let c = TermView::Constant("Test");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn nested_function_resolve() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let f = interner.intern("f");
    let g = interner.intern("g");
    let x = interner.intern("x");

    let inner = Term::Function(g, term_arena.alloc_slice([Term::Variable(x)]));
    let outer = Term::Function(f, term_arena.alloc_slice([inner]));

    assert_eq!(
        outer.resolve(&interner),
        TermView::Function(
            "f",
            vec![TermView::Function("g", vec![TermView::Variable("x")])]
        )
    );
}
