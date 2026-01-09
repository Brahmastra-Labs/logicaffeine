//! Standard Library for the Kernel.
//!
//! Defines fundamental types and logical connectives:
//! - Entity: domain of individuals (for FOL)
//! - Nat: natural numbers
//! - True, False: propositional constants
//! - Eq: propositional equality
//! - And, Or: logical connectives

use super::context::Context;
use super::term::{Term, Universe};

/// Standard library definitions.
pub struct StandardLibrary;

impl StandardLibrary {
    /// Register all standard library definitions in the context.
    pub fn register(ctx: &mut Context) {
        Self::register_entity(ctx);
        Self::register_nat(ctx);
        Self::register_true(ctx);
        Self::register_false(ctx);
        Self::register_not(ctx);
        Self::register_eq(ctx);
        Self::register_and(ctx);
        Self::register_or(ctx);
        Self::register_ex(ctx);
        Self::register_primitives(ctx);
        Self::register_reflection(ctx);
    }

    /// Primitive types and operations.
    ///
    /// Int : Type 0 (64-bit signed integer)
    /// Float : Type 0 (64-bit floating point)
    /// Text : Type 0 (UTF-8 string)
    ///
    /// add, sub, mul, div, mod : Int -> Int -> Int
    fn register_primitives(ctx: &mut Context) {
        // Opaque types (no constructors, cannot be pattern-matched)
        ctx.add_inductive("Int", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Float", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Text", Term::Sort(Universe::Type(0)));

        let int = Term::Global("Int".to_string());

        // Binary Int -> Int -> Int type
        let bin_int_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(int.clone()),
            }),
        };

        // Register arithmetic builtins as declarations (axioms with computational behavior)
        ctx.add_declaration("add", bin_int_type.clone());
        ctx.add_declaration("sub", bin_int_type.clone());
        ctx.add_declaration("mul", bin_int_type.clone());
        ctx.add_declaration("div", bin_int_type.clone());
        ctx.add_declaration("mod", bin_int_type);
    }

    /// Entity : Type 0
    ///
    /// The domain of individuals for first-order logic.
    /// Proper names like "Socrates" are declared as Entity.
    /// Predicates like "Man" are declared as Entity → Prop.
    fn register_entity(ctx: &mut Context) {
        ctx.add_inductive("Entity", Term::Sort(Universe::Type(0)));
    }

    /// Nat : Type 0
    /// Zero : Nat
    /// Succ : Nat → Nat
    fn register_nat(ctx: &mut Context) {
        let nat = Term::Global("Nat".to_string());

        // Nat : Type 0
        ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

        // Zero : Nat
        ctx.add_constructor("Zero", "Nat", nat.clone());

        // Succ : Nat → Nat
        ctx.add_constructor(
            "Succ",
            "Nat",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(nat.clone()),
                body_type: Box::new(nat),
            },
        );
    }

    /// True : Prop
    /// I : True
    fn register_true(ctx: &mut Context) {
        ctx.add_inductive("True", Term::Sort(Universe::Prop));
        ctx.add_constructor("I", "True", Term::Global("True".to_string()));
    }

    /// False : Prop
    /// (no constructors)
    fn register_false(ctx: &mut Context) {
        ctx.add_inductive("False", Term::Sort(Universe::Prop));
    }

    /// Not : Prop -> Prop
    /// Not P := P -> False
    fn register_not(ctx: &mut Context) {
        // Type: Π(P : Prop). Prop
        let not_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        };

        // Body: λ(P : Prop). Π(_ : P). False
        let not_body = Term::Lambda {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(Term::Var("P".to_string())),
                body_type: Box::new(Term::Global("False".to_string())),
            }),
        };

        ctx.add_definition("Not".to_string(), not_type, not_body);
    }

    /// Eq : Π(A : Type 0). A → A → Prop
    /// refl : Π(A : Type 0). Π(x : A). Eq A x x
    fn register_eq(ctx: &mut Context) {
        // Eq : Π(A : Type 0). A → A → Prop
        let eq_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(Term::Var("A".to_string())),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(Term::Var("A".to_string())),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
            }),
        };
        ctx.add_inductive("Eq", eq_type);

        // refl : Π(A : Type 0). Π(x : A). Eq A x x
        let refl_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(Term::Var("A".to_string())),
                body_type: Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Eq".to_string())),
                            Box::new(Term::Var("A".to_string())),
                        )),
                        Box::new(Term::Var("x".to_string())),
                    )),
                    Box::new(Term::Var("x".to_string())),
                )),
            }),
        };
        ctx.add_constructor("refl", "Eq", refl_type);

        // Eq_rec : Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
        // The eliminator for equality - Leibniz's Law
        Self::register_eq_rec(ctx);

        // Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
        Self::register_eq_sym(ctx);

        // Eq_trans : Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
        Self::register_eq_trans(ctx);
    }

    /// Eq_rec : Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
    /// The eliminator for equality - Leibniz's Law / Substitution of Equals
    fn register_eq_rec(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());
        let p = Term::Var("P".to_string());

        // P x = P applied to x
        let p_x = Term::App(Box::new(p.clone()), Box::new(x.clone()));

        // P y = P applied to y
        let p_y = Term::App(Box::new(p.clone()), Box::new(y.clone()));

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // P : A → Prop
        let p_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(a.clone()),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        };

        // Full type: Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
        let eq_rec_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "P".to_string(),
                    param_type: Box::new(p_type),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(p_x),
                        body_type: Box::new(Term::Pi {
                            param: "y".to_string(),
                            param_type: Box::new(a),
                            body_type: Box::new(Term::Pi {
                                param: "_".to_string(),
                                param_type: Box::new(eq_a_x_y),
                                body_type: Box::new(p_y),
                            }),
                        }),
                    }),
                }),
            }),
        };

        ctx.add_declaration("Eq_rec", eq_rec_type);
    }

    /// Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
    /// Symmetry of equality
    fn register_eq_sym(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // Eq A y x
        let eq_a_y_x = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(y.clone()),
            )),
            Box::new(x.clone()),
        );

        // Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
        let eq_sym_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(a),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(eq_a_x_y),
                        body_type: Box::new(eq_a_y_x),
                    }),
                }),
            }),
        };

        ctx.add_declaration("Eq_sym", eq_sym_type);
    }

    /// Eq_trans : Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
    /// Transitivity of equality
    fn register_eq_trans(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());
        let z = Term::Var("z".to_string());

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // Eq A y z
        let eq_a_y_z = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(y.clone()),
            )),
            Box::new(z.clone()),
        );

        // Eq A x z
        let eq_a_x_z = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(z.clone()),
        );

        // Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
        let eq_trans_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(a.clone()),
                    body_type: Box::new(Term::Pi {
                        param: "z".to_string(),
                        param_type: Box::new(a),
                        body_type: Box::new(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(eq_a_x_y),
                            body_type: Box::new(Term::Pi {
                                param: "_".to_string(),
                                param_type: Box::new(eq_a_y_z),
                                body_type: Box::new(eq_a_x_z),
                            }),
                        }),
                    }),
                }),
            }),
        };

        ctx.add_declaration("Eq_trans", eq_trans_type);
    }

    /// And : Prop → Prop → Prop
    /// conj : Π(P : Prop). Π(Q : Prop). P → Q → And P Q
    fn register_and(ctx: &mut Context) {
        // And : Prop → Prop → Prop
        let and_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("And", and_type);

        // conj : Π(P : Prop). Π(Q : Prop). P → Q → And P Q
        let conj_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "p".to_string(),
                    param_type: Box::new(Term::Var("P".to_string())),
                    body_type: Box::new(Term::Pi {
                        param: "q".to_string(),
                        param_type: Box::new(Term::Var("Q".to_string())),
                        body_type: Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("And".to_string())),
                                Box::new(Term::Var("P".to_string())),
                            )),
                            Box::new(Term::Var("Q".to_string())),
                        )),
                    }),
                }),
            }),
        };
        ctx.add_constructor("conj", "And", conj_type);
    }

    /// Or : Prop → Prop → Prop
    /// left : Π(P : Prop). Π(Q : Prop). P → Or P Q
    /// right : Π(P : Prop). Π(Q : Prop). Q → Or P Q
    fn register_or(ctx: &mut Context) {
        // Or : Prop → Prop → Prop
        let or_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("Or", or_type);

        // left : Π(P : Prop). Π(Q : Prop). P → Or P Q
        let left_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "p".to_string(),
                    param_type: Box::new(Term::Var("P".to_string())),
                    body_type: Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Or".to_string())),
                            Box::new(Term::Var("P".to_string())),
                        )),
                        Box::new(Term::Var("Q".to_string())),
                    )),
                }),
            }),
        };
        ctx.add_constructor("left", "Or", left_type);

        // right : Π(P : Prop). Π(Q : Prop). Q → Or P Q
        let right_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "q".to_string(),
                    param_type: Box::new(Term::Var("Q".to_string())),
                    body_type: Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Or".to_string())),
                            Box::new(Term::Var("P".to_string())),
                        )),
                        Box::new(Term::Var("Q".to_string())),
                    )),
                }),
            }),
        };
        ctx.add_constructor("right", "Or", right_type);
    }

    /// Ex : Π(A : Type 0). (A → Prop) → Prop
    /// witness : Π(A : Type 0). Π(P : A → Prop). Π(x : A). P x → Ex A P
    ///
    /// Ex is the existential type for propositions.
    /// Ex A P means "there exists an x:A such that P(x)"
    fn register_ex(ctx: &mut Context) {
        // Ex : Π(A : Type 0). (A → Prop) → Prop
        let ex_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "P".to_string(),
                param_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(Term::Var("A".to_string())),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("Ex", ex_type);

        // witness : Π(A : Type 0). Π(P : A → Prop). Π(x : A). P x → Ex A P
        //
        // To construct an existential proof, provide:
        // - A: the type being quantified over
        // - P: the predicate
        // - x: the witness (an element of A)
        // - proof: evidence that P(x) holds
        let a = Term::Var("A".to_string());
        let p = Term::Var("P".to_string());
        let x = Term::Var("x".to_string());

        // P x = P applied to x
        let p_x = Term::App(Box::new(p.clone()), Box::new(x.clone()));

        // Ex A P = Ex applied to A, then to P
        let ex_a_p = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Ex".to_string())),
                Box::new(a.clone()),
            )),
            Box::new(p.clone()),
        );

        let witness_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "P".to_string(),
                param_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(a.clone()),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
                body_type: Box::new(Term::Pi {
                    param: "x".to_string(),
                    param_type: Box::new(a),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(p_x),
                        body_type: Box::new(ex_a_p),
                    }),
                }),
            }),
        };
        ctx.add_constructor("witness", "Ex", witness_type);
    }

    // =========================================================================
    // PHASE 87: REFLECTION (DEEP EMBEDDING)
    // =========================================================================

    /// Reflection types for deep embedding.
    ///
    /// Univ : Type 0 (representation of universes)
    /// Syntax : Type 0 (representation of terms with De Bruijn indices)
    /// syn_size : Syntax -> Int (compute size of syntax tree)
    /// syn_max_var : Syntax -> Int (compute max free variable index)
    fn register_reflection(ctx: &mut Context) {
        Self::register_univ(ctx);
        Self::register_syntax(ctx);
        Self::register_syn_size(ctx);
        Self::register_syn_max_var(ctx);
        Self::register_syn_lift(ctx);
        Self::register_syn_subst(ctx);
        Self::register_syn_beta(ctx);
        Self::register_syn_step(ctx);
        Self::register_syn_eval(ctx);
        Self::register_syn_quote(ctx);
        Self::register_syn_diag(ctx);
        Self::register_derivation(ctx);
        Self::register_concludes(ctx);
        Self::register_try_refl(ctx);
        Self::register_try_compute(ctx);
        Self::register_try_cong(ctx);
        Self::register_tact_fail(ctx);
        Self::register_tact_orelse(ctx);
    }

    /// Univ : Type 0 (representation of universes)
    /// UProp : Univ
    /// UType : Int -> Univ
    fn register_univ(ctx: &mut Context) {
        let univ = Term::Global("Univ".to_string());
        let int = Term::Global("Int".to_string());

        // Univ : Type 0
        ctx.add_inductive("Univ", Term::Sort(Universe::Type(0)));

        // UProp : Univ
        ctx.add_constructor("UProp", "Univ", univ.clone());

        // UType : Int -> Univ
        ctx.add_constructor(
            "UType",
            "Univ",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(univ),
            },
        );
    }

    /// Syntax : Type 0 (representation of terms with De Bruijn indices)
    ///
    /// SVar : Int -> Syntax (De Bruijn variable index)
    /// SGlobal : Int -> Syntax (global reference by ID)
    /// SSort : Univ -> Syntax (universe)
    /// SApp : Syntax -> Syntax -> Syntax (application)
    /// SLam : Syntax -> Syntax -> Syntax (lambda: param_type, body)
    /// SPi : Syntax -> Syntax -> Syntax (pi: param_type, body_type)
    fn register_syntax(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());
        let univ = Term::Global("Univ".to_string());

        // Syntax : Type 0
        ctx.add_inductive("Syntax", Term::Sort(Universe::Type(0)));

        // SVar : Int -> Syntax (De Bruijn index)
        ctx.add_constructor(
            "SVar",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SGlobal : Int -> Syntax (global reference by ID)
        ctx.add_constructor(
            "SGlobal",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SSort : Univ -> Syntax
        ctx.add_constructor(
            "SSort",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(univ),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SApp : Syntax -> Syntax -> Syntax
        ctx.add_constructor(
            "SApp",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SLam : Syntax -> Syntax -> Syntax (param_type, body)
        ctx.add_constructor(
            "SLam",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SPi : Syntax -> Syntax -> Syntax (param_type, body_type)
        ctx.add_constructor(
            "SPi",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SLit : Int -> Syntax (integer literal in quoted syntax)
        ctx.add_constructor(
            "SLit",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SName : Text -> Syntax (named global reference)
        let text = Term::Global("Text".to_string());
        ctx.add_constructor(
            "SName",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(text),
                body_type: Box::new(syntax),
            },
        );
    }

    /// syn_size : Syntax -> Int
    ///
    /// Computes the number of nodes in a syntax tree.
    /// Computational behavior defined in reduction.rs.
    fn register_syn_size(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_size : Syntax -> Int
        let syn_size_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(int),
        };

        ctx.add_declaration("syn_size", syn_size_type);
    }

    /// syn_max_var : Syntax -> Int
    ///
    /// Returns the maximum free variable index in a syntax term.
    /// Returns -1 if the term is closed (all variables bound).
    /// Computational behavior defined in reduction.rs.
    fn register_syn_max_var(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        let syn_max_var_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(int),
        };

        ctx.add_declaration("syn_max_var", syn_max_var_type);
    }

    // =========================================================================
    // PHASE 88: SUBSTITUTION (DE BRUIJN OPERATIONS)
    // =========================================================================

    /// syn_lift : Int -> Int -> Syntax -> Syntax
    ///
    /// Shifts free variables (index >= cutoff) by amount.
    /// - syn_lift amount cutoff term
    /// - Variables with index < cutoff are bound -> unchanged
    /// - Variables with index >= cutoff are free -> add amount
    /// Computational behavior defined in reduction.rs.
    fn register_syn_lift(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_lift : Int -> Int -> Syntax -> Syntax
        let syn_lift_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax),
                }),
            }),
        };

        ctx.add_declaration("syn_lift", syn_lift_type);
    }

    /// syn_subst : Syntax -> Int -> Syntax -> Syntax
    ///
    /// Substitutes replacement for variable at index in term.
    /// - syn_subst replacement index term
    /// - If term is SVar k and k == index, return replacement
    /// - If term is SVar k and k != index, return term unchanged
    /// - For binders, increment index and lift replacement
    /// Computational behavior defined in reduction.rs.
    fn register_syn_subst(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_subst : Syntax -> Int -> Syntax -> Syntax
        let syn_subst_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax),
                }),
            }),
        };

        ctx.add_declaration("syn_subst", syn_subst_type);
    }

    // =========================================================================
    // PHASE 89: COMPUTATION (BETA REDUCTION)
    // =========================================================================

    /// syn_beta : Syntax -> Syntax -> Syntax
    ///
    /// Beta reduction: substitute arg for variable 0 in body.
    /// - syn_beta body arg = syn_subst arg 0 body
    /// Computational behavior defined in reduction.rs.
    fn register_syn_beta(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_beta : Syntax -> Syntax -> Syntax
        let syn_beta_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(syntax),
            }),
        };

        ctx.add_declaration("syn_beta", syn_beta_type);
    }

    /// syn_step : Syntax -> Syntax
    ///
    /// Single-step head reduction. Finds first beta redex and reduces.
    /// - If SApp (SLam T body) arg: perform beta reduction
    /// - If SApp f x where f is reducible: reduce f
    /// - Otherwise: return unchanged (stuck or value)
    /// Computational behavior defined in reduction.rs.
    fn register_syn_step(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_step : Syntax -> Syntax
        let syn_step_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_step", syn_step_type);
    }

    // =========================================================================
    // PHASE 90: BOUNDED EVALUATION (THE CLOCK)
    // =========================================================================

    /// syn_eval : Int -> Syntax -> Syntax
    ///
    /// Bounded evaluation: reduce for up to N steps.
    /// - syn_eval fuel term
    /// - If fuel <= 0: return term unchanged
    /// - Otherwise: step and repeat until normal form or fuel exhausted
    /// Computational behavior defined in reduction.rs.
    fn register_syn_eval(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_eval : Int -> Syntax -> Syntax
        let syn_eval_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(syntax),
            }),
        };

        ctx.add_declaration("syn_eval", syn_eval_type);
    }

    // =========================================================================
    // PHASE 91: REIFICATION (THE QUOTE)
    // =========================================================================

    /// syn_quote : Syntax -> Syntax
    ///
    /// Converts a Syntax value to Syntax code that constructs it.
    /// Computational behavior defined in reduction.rs.
    fn register_syn_quote(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_quote : Syntax -> Syntax
        let syn_quote_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_quote", syn_quote_type);
    }

    // =========================================================================
    // PHASE 93: DIAGONAL LEMMA (SELF-REFERENCE)
    // =========================================================================

    /// syn_diag : Syntax -> Syntax
    ///
    /// The diagonal function: syn_diag x = syn_subst (syn_quote x) 0 x
    /// This is the key construction for self-reference and the diagonal lemma.
    fn register_syn_diag(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        let syn_diag_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_diag", syn_diag_type);
    }

    // =========================================================================
    // PHASE 92: INFERENCE RULES (THE LAW)
    // =========================================================================

    /// Derivation : Type 0 (deep embedding of proof trees)
    ///
    /// DAxiom : Syntax -> Derivation (introduce an axiom)
    /// DModusPonens : Derivation -> Derivation -> Derivation (A->B, A |- B)
    /// DUnivIntro : Derivation -> Derivation (P(x) |- forall x. P(x))
    /// DUnivElim : Derivation -> Syntax -> Derivation (forall x. P(x), t |- P(t))
    fn register_derivation(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Derivation : Type 0
        ctx.add_inductive("Derivation", Term::Sort(Universe::Type(0)));

        // DAxiom : Syntax -> Derivation
        ctx.add_constructor(
            "DAxiom",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DModusPonens : Derivation -> Derivation -> Derivation
        ctx.add_constructor(
            "DModusPonens",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DUnivIntro : Derivation -> Derivation
        ctx.add_constructor(
            "DUnivIntro",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DUnivElim : Derivation -> Syntax -> Derivation
        ctx.add_constructor(
            "DUnivElim",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DRefl : Syntax -> Syntax -> Derivation
        // DRefl T a proves (Eq T a a)
        ctx.add_constructor(
            "DRefl",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DInduction : Syntax -> Derivation -> Derivation -> Derivation
        // DInduction motive base step proves (∀n:Nat. motive n) via induction
        // - motive: λn:Nat. P(n) as Syntax
        // - base: proof of P(Zero)
        // - step: proof of ∀k:Nat. P(k) → P(Succ k)
        ctx.add_constructor(
            "DInduction",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // motive
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // base proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // step proof
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DCompute : Syntax -> Derivation
        // DCompute goal proves goal by computation
        // - Validates that goal is (Eq T A B) and eval(A) == eval(B)
        ctx.add_constructor(
            "DCompute",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DCong : Syntax -> Derivation -> Derivation
        // DCong context eq_proof proves congruence
        // - context: SLam T body (function with hole at SVar 0)
        // - eq_proof: proof of (Eq T a b)
        // - Result: proof of (Eq T (body[0:=a]) (body[0:=b]))
        ctx.add_constructor(
            "DCong",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // context
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // eq_proof
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // Phase 101b: Generic Elimination
        //
        // DCase : Derivation -> Derivation -> Derivation
        // Cons cell for case proofs in DElim
        ctx.add_constructor(
            "DCase",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // head (case proof)
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // tail (rest of cases)
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DCaseEnd : Derivation
        // Nil for case proofs in DElim
        ctx.add_constructor("DCaseEnd", "Derivation", derivation.clone());

        // DElim : Syntax -> Syntax -> Derivation -> Derivation
        // Generic elimination for any inductive type
        // - ind_type: the inductive type (e.g., SName "Nat" or SApp (SName "List") A)
        // - motive: SLam param_type body (the property to prove)
        // - cases: DCase chain with one proof per constructor
        ctx.add_constructor(
            "DElim",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation),
                    }),
                }),
            },
        );
    }

    /// concludes : Derivation -> Syntax
    ///
    /// Extracts the conclusion from a derivation.
    /// Computational behavior defined in reduction.rs.
    fn register_concludes(ctx: &mut Context) {
        let derivation = Term::Global("Derivation".to_string());
        let syntax = Term::Global("Syntax".to_string());

        // concludes : Derivation -> Syntax
        let concludes_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(derivation),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("concludes", concludes_type);
    }

    // =========================================================================
    // PHASE 96: TACTICS (THE WIZARD)
    // =========================================================================

    /// try_refl : Syntax -> Derivation
    ///
    /// Reflexivity tactic: given a goal, try to prove it by reflexivity.
    /// - If goal matches (Eq T a b) and a == b, returns DRefl T a
    /// - Otherwise returns DAxiom (SName "Error")
    /// Computational behavior defined in reduction.rs.
    fn register_try_refl(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_refl : Syntax -> Derivation
        let try_refl_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_refl", try_refl_type);
    }

    /// try_compute : Syntax -> Derivation
    ///
    /// Computation tactic: given a goal (Eq T A B), proves it by evaluating
    /// both sides and checking equality.
    /// Returns DCompute goal.
    /// Computational behavior defined in reduction.rs.
    fn register_try_compute(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_compute : Syntax -> Derivation
        let try_compute_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_compute", try_compute_type);
    }

    /// try_cong : Syntax -> Derivation -> Derivation
    ///
    /// Congruence tactic: given a context (SLam T body) and proof of (Eq T a b),
    /// produces proof of (Eq T (body[0:=a]) (body[0:=b])).
    /// Returns DCong context eq_proof.
    /// Computational behavior defined in reduction.rs.
    fn register_try_cong(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_cong : Syntax -> Derivation -> Derivation
        let try_cong_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax), // context
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("try_cong", try_cong_type);
    }

    // =========================================================================
    // PHASE 98: TACTIC COMBINATORS (THE STRATEGIST)
    // =========================================================================

    /// tact_fail : Syntax -> Derivation
    ///
    /// A tactic that always fails by returning DAxiom (SName "Error").
    /// Useful for testing combinators and as a base case.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_fail(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // tact_fail : Syntax -> Derivation
        let tact_fail_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("tact_fail", tact_fail_type);
    }

    /// tact_orelse : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try first tactic, if it fails (concludes to Error) try second.
    /// Enables composition of tactics with fallback behavior.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_orelse(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_orelse : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_orelse_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type.clone()), // t1
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(tactic_type), // t2
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax), // goal
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("tact_orelse", tact_orelse_type);
    }
}
