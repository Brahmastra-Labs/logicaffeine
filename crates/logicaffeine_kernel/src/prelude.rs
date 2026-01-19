//! Standard Library for the Kernel.
//!
//! Defines fundamental types and logical connectives:
//! - Entity: domain of individuals (for FOL)
//! - Nat: natural numbers
//! - True, False: propositional constants
//! - Eq: propositional equality
//! - And, Or: logical connectives

use crate::context::Context;
use crate::term::{Term, Universe};

/// Standard library definitions.
pub struct StandardLibrary;

impl StandardLibrary {
    /// Register all standard library definitions in the context.
    pub fn register(ctx: &mut Context) {
        Self::register_entity(ctx);
        Self::register_nat(ctx);
        Self::register_bool(ctx);
        Self::register_tlist(ctx);
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
    /// Duration : Type 0 (nanoseconds, i64 - physical time)
    /// Date : Type 0 (days since epoch, i32 - calendar date)
    /// Moment : Type 0 (nanoseconds since epoch, i64 - instant in UTC)
    ///
    /// add, sub, mul, div, mod : Int -> Int -> Int
    fn register_primitives(ctx: &mut Context) {
        // Opaque types (no constructors, cannot be pattern-matched)
        ctx.add_inductive("Int", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Float", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Text", Term::Sort(Universe::Type(0)));

        // Temporal types (opaque, no constructors)
        ctx.add_inductive("Duration", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Date", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Moment", Term::Sort(Universe::Type(0)));

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

        // Temporal operations
        let duration = Term::Global("Duration".to_string());
        let date = Term::Global("Date".to_string());
        let moment = Term::Global("Moment".to_string());
        let bool_type = Term::Global("Bool".to_string());

        // Duration -> Duration -> Duration
        let bin_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };

        // Duration -> Int -> Duration
        let duration_int_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };

        // Duration arithmetic: add, sub, mul, div
        ctx.add_declaration("add_duration", bin_duration_type.clone());
        ctx.add_declaration("sub_duration", bin_duration_type.clone());
        ctx.add_declaration("mul_duration", duration_int_duration_type.clone());
        ctx.add_declaration("div_duration", duration_int_duration_type);

        // Date -> Int -> Date (add days offset)
        let date_int_date_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(date.clone()),
            }),
        };
        ctx.add_declaration("date_add_days", date_int_date_type);

        // Date -> Date -> Int (difference in days)
        let date_date_int_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(date.clone()),
                body_type: Box::new(int.clone()),
            }),
        };
        ctx.add_declaration("date_sub_date", date_date_int_type);

        // Moment -> Duration -> Moment
        let moment_duration_moment_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration.clone()),
                body_type: Box::new(moment.clone()),
            }),
        };
        ctx.add_declaration("moment_add_duration", moment_duration_moment_type);

        // Moment -> Moment -> Duration
        let moment_moment_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(moment.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };
        ctx.add_declaration("moment_sub_moment", moment_moment_duration_type);

        // Comparison operations: X -> X -> Bool
        let date_date_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(date),
                body_type: Box::new(bool_type.clone()),
            }),
        };
        ctx.add_declaration("date_lt", date_date_bool_type);

        let moment_moment_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(moment),
                body_type: Box::new(bool_type.clone()),
            }),
        };
        ctx.add_declaration("moment_lt", moment_moment_bool_type);

        let duration_duration_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration),
                body_type: Box::new(bool_type),
            }),
        };
        ctx.add_declaration("duration_lt", duration_duration_bool_type);
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

    /// Bool : Type 0
    /// true : Bool
    /// false : Bool
    fn register_bool(ctx: &mut Context) {
        let bool_type = Term::Global("Bool".to_string());

        // Bool : Type 0
        ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));

        // true : Bool
        ctx.add_constructor("true", "Bool", bool_type.clone());

        // false : Bool
        ctx.add_constructor("false", "Bool", bool_type);
    }

    /// TList : Type 0 -> Type 0
    /// TNil : Π(A : Type 0). TList A
    /// TCons : Π(A : Type 0). A -> TList A -> TList A
    fn register_tlist(ctx: &mut Context) {
        let type0 = Term::Sort(Universe::Type(0));
        let a = Term::Var("A".to_string());

        // TList : Type 0 -> Type 0
        let tlist_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(type0.clone()),
        };
        ctx.add_inductive("TList", tlist_type);

        // TList A
        let tlist_a = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(a.clone()),
        );

        // TNil : Π(A : Type 0). TList A
        let tnil_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(tlist_a.clone()),
        };
        ctx.add_constructor("TNil", "TList", tnil_type);

        // TCons : Π(A : Type 0). A -> TList A -> TList A
        let tcons_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(tlist_a.clone()),
                    body_type: Box::new(tlist_a),
                }),
            }),
        };
        ctx.add_constructor("TCons", "TList", tcons_type);

        // Convenience aliases for tactic lists (Syntax -> Derivation)
        Self::register_tactic_list_helpers(ctx);
    }

    /// TTactics : Type 0 = TList (Syntax -> Derivation)
    /// TacNil : TTactics
    /// TacCons : (Syntax -> Derivation) -> TTactics -> TTactics
    ///
    /// Convenience wrappers to avoid explicit type arguments for tactic lists.
    fn register_tactic_list_helpers(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // TTactics = TList (Syntax -> Derivation)
        let ttactics = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(tactic_type.clone()),
        );

        // TTactics : Type 0
        ctx.add_definition("TTactics".to_string(), Term::Sort(Universe::Type(0)), ttactics.clone());

        // TacNil : TTactics = TNil (Syntax -> Derivation)
        let tac_nil_body = Term::App(
            Box::new(Term::Global("TNil".to_string())),
            Box::new(tactic_type.clone()),
        );
        ctx.add_definition("TacNil".to_string(), ttactics.clone(), tac_nil_body);

        // TacCons : (Syntax -> Derivation) -> TTactics -> TTactics
        let tac_cons_type = Term::Pi {
            param: "t".to_string(),
            param_type: Box::new(tactic_type.clone()),
            body_type: Box::new(Term::Pi {
                param: "ts".to_string(),
                param_type: Box::new(ttactics.clone()),
                body_type: Box::new(ttactics.clone()),
            }),
        };

        // TacCons t ts = TCons (Syntax -> Derivation) t ts
        let tac_cons_body = Term::Lambda {
            param: "t".to_string(),
            param_type: Box::new(tactic_type.clone()),
            body: Box::new(Term::Lambda {
                param: "ts".to_string(),
                param_type: Box::new(ttactics.clone()),
                body: Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("TCons".to_string())),
                            Box::new(tactic_type),
                        )),
                        Box::new(Term::Var("t".to_string())),
                    )),
                    Box::new(Term::Var("ts".to_string())),
                )),
            }),
        };
        ctx.add_definition("TacCons".to_string(), tac_cons_type, tac_cons_body);
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

    // -------------------------------------------------------------------------
    // Reflection System (Deep Embedding)
    // -------------------------------------------------------------------------

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
        Self::register_tact_try(ctx);
        Self::register_tact_repeat(ctx);
        Self::register_tact_then(ctx);
        Self::register_tact_first(ctx);
        Self::register_tact_solve(ctx);
        Self::register_try_ring(ctx);
        Self::register_try_lia(ctx);
        Self::register_try_cc(ctx);
        Self::register_try_simp(ctx);
        Self::register_try_omega(ctx);
        Self::register_try_auto(ctx);
        Self::register_try_induction(ctx);
        Self::register_induction_helpers(ctx);
        Self::register_try_inversion_tactic(ctx);
        Self::register_operator_tactics(ctx);
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

    // -------------------------------------------------------------------------
    // De Bruijn Operations (Substitution)
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Beta Reduction (Computation)
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Bounded Evaluation
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Reification (Quote)
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Diagonalization (Self-Reference)
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Inference Rules (Proof Trees)
    // -------------------------------------------------------------------------

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
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DInversion : Syntax -> Derivation
        // Proves False when no constructor can build the given hypothesis.
        // - hyp_type: the Syntax representation of the hypothesis (e.g., SApp (SName "Even") three)
        // - Returns proof of False if verified that all constructors are impossible
        ctx.add_constructor(
            "DInversion",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DRewrite : Derivation -> Syntax -> Syntax -> Derivation
        // Stores: eq_proof, original_goal, new_goal
        // Given eq_proof : Eq A x y, rewrites goal by replacing x with y
        ctx.add_constructor(
            "DRewrite",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // original_goal
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax.clone()), // new_goal
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DDestruct : Syntax -> Syntax -> Derivation -> Derivation
        // Case analysis without induction hypotheses
        // - ind_type: the inductive type
        // - motive: the property to prove
        // - cases: DCase chain with proofs for each constructor
        ctx.add_constructor(
            "DDestruct",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DApply : Syntax -> Derivation -> Syntax -> Syntax -> Derivation
        // Manual backward chaining
        // - hyp_name: name of hypothesis
        // - hyp_proof: proof of the hypothesis
        // - original_goal: the goal we started with
        // - new_goal: the antecedent we need to prove
        ctx.add_constructor(
            "DApply",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // hyp_name
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // hyp_proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax.clone()), // original_goal
                        body_type: Box::new(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(syntax), // new_goal
                            body_type: Box::new(derivation),
                        }),
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

    // -------------------------------------------------------------------------
    // Core Tactics
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Tactic Combinators
    // -------------------------------------------------------------------------

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

    /// tact_try : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try the tactic, but never fail.
    /// If the tactic fails (concludes to Error), return identity (DAxiom goal).
    /// Computational behavior defined in reduction.rs.
    fn register_tact_try(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_try : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_try_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_try", tact_try_type);
    }

    /// tact_repeat : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: apply tactic repeatedly until it fails or makes no progress.
    /// Returns identity if tactic fails immediately.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_repeat(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_repeat : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_repeat_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_repeat", tact_repeat_type);
    }

    /// tact_then : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: sequence two tactics (the ";" operator).
    /// Apply t1 to goal, then apply t2 to the result.
    /// Fails if either tactic fails.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_then(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_then : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_then_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type.clone()), // t1
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(tactic_type), // t2
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax),
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("tact_then", tact_then_type);
    }

    /// tact_first : TList (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try tactics from a list until one succeeds.
    /// Returns Error if all tactics fail or list is empty.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_first(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // TList (Syntax -> Derivation)
        let tactic_list_type = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(tactic_type),
        );

        // tact_first : TList (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_first_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_list_type), // tactics
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_first", tact_first_type);
    }

    /// tact_solve : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: tactic MUST completely solve the goal.
    /// If tactic returns Error, returns Error.
    /// If tactic returns a proof, returns that proof.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_solve(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_solve : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_solve_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_solve", tact_solve_type);
    }

    // -------------------------------------------------------------------------
    // Ring Tactic (Polynomial Equality)
    // -------------------------------------------------------------------------

    /// DRingSolve : Syntax -> Derivation
    /// try_ring : Syntax -> Derivation
    ///
    /// Ring tactic: proves polynomial equalities by normalization.
    /// Computational behavior defined in reduction.rs.
    fn register_try_ring(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DRingSolve : Syntax -> Derivation
        // Proof constructor for ring-solved equalities
        ctx.add_constructor(
            "DRingSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_ring : Syntax -> Derivation
        // Ring tactic: given a goal, try to prove it by polynomial normalization
        let try_ring_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_ring", try_ring_type);
    }

    // -------------------------------------------------------------------------
    // LIA Tactic (Linear Integer Arithmetic)
    // -------------------------------------------------------------------------

    /// DLiaSolve : Syntax -> Derivation
    /// try_lia : Syntax -> Derivation
    ///
    /// LIA tactic: proves linear inequalities by Fourier-Motzkin elimination.
    /// Computational behavior defined in reduction.rs.
    fn register_try_lia(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DLiaSolve : Syntax -> Derivation
        // Proof constructor for LIA-solved inequalities
        ctx.add_constructor(
            "DLiaSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_lia : Syntax -> Derivation
        // LIA tactic: given a goal, try to prove it by linear arithmetic
        let try_lia_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_lia", try_lia_type);
    }

    /// DccSolve : Syntax -> Derivation
    /// try_cc : Syntax -> Derivation
    ///
    /// Congruence Closure tactic: proves equalities over uninterpreted functions.
    /// Handles hypotheses via implications: (implies (Eq x y) (Eq (f x) (f y)))
    /// Computational behavior defined in reduction.rs.
    fn register_try_cc(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DccSolve : Syntax -> Derivation
        // Proof constructor for congruence closure proofs
        ctx.add_constructor(
            "DccSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_cc : Syntax -> Derivation
        // CC tactic: given a goal, try to prove it by congruence closure
        let try_cc_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_cc", try_cc_type);
    }

    /// DSimpSolve : Syntax -> Derivation
    /// try_simp : Syntax -> Derivation
    ///
    /// Simplifier tactic: proves equalities by term rewriting.
    /// Uses bottom-up rewriting with arithmetic evaluation and hypothesis substitution.
    /// Handles: reflexivity, constant folding (2+3=5), and hypothesis-based substitution.
    /// Computational behavior defined in reduction.rs.
    fn register_try_simp(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DSimpSolve : Syntax -> Derivation
        // Proof constructor for simplifier proofs
        ctx.add_constructor(
            "DSimpSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_simp : Syntax -> Derivation
        // Simp tactic: given a goal, try to prove it by simplification
        let try_simp_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_simp", try_simp_type);
    }

    /// DOmegaSolve : Syntax -> Derivation
    /// try_omega : Syntax -> Derivation
    ///
    /// Omega tactic: proves linear integer arithmetic with proper floor/ceil rounding.
    /// Unlike lia (which uses rationals), omega handles integers correctly:
    /// - x > 1 means x >= 2 (strict-to-nonstrict conversion)
    /// - 3x <= 10 means x <= 3 (floor division)
    /// Computational behavior defined in reduction.rs.
    fn register_try_omega(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DOmegaSolve : Syntax -> Derivation
        // Proof constructor for omega-solved inequalities
        ctx.add_constructor(
            "DOmegaSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_omega : Syntax -> Derivation
        // Omega tactic: given a goal, try to prove it by integer arithmetic
        let try_omega_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_omega", try_omega_type);
    }

    /// DAutoSolve : Syntax -> Derivation
    /// try_auto : Syntax -> Derivation
    ///
    /// Auto tactic: tries all decision procedures in sequence.
    /// Order: simp → ring → cc → omega → lia
    /// Returns the first successful derivation, or error if all fail.
    /// Computational behavior defined in reduction.rs.
    fn register_try_auto(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DAutoSolve : Syntax -> Derivation
        // Proof constructor for auto-solved goals
        ctx.add_constructor(
            "DAutoSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_auto : Syntax -> Derivation
        // Auto tactic: given a goal, try all tactics in sequence
        let try_auto_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_auto", try_auto_type);
    }

    /// try_induction : Syntax -> Syntax -> Derivation -> Derivation
    ///
    /// Generic induction tactic for any inductive type.
    /// Arguments:
    /// - ind_type: The inductive type (SName "Nat" or SApp (SName "List") A)
    /// - motive: The property to prove (SLam param_type body)
    /// - cases: DCase chain with one derivation per constructor
    ///
    /// Returns a DElim derivation if verification passes.
    fn register_try_induction(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_induction : Syntax -> Syntax -> Derivation -> Derivation
        let try_induction_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()), // ind_type
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // motive
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // cases
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("try_induction", try_induction_type);
    }

    /// Helper functions for building induction goals.
    ///
    /// These functions help construct the subgoals for induction:
    /// - induction_base_goal: Computes the base case goal
    /// - induction_step_goal: Computes the step case goal for a constructor
    /// - induction_num_cases: Returns number of constructors for an inductive
    fn register_induction_helpers(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let nat = Term::Global("Nat".to_string());

        // induction_base_goal : Syntax -> Syntax -> Syntax
        // Given ind_type and motive, returns the base case goal (first constructor)
        ctx.add_declaration(
            "induction_base_goal",
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

        // induction_step_goal : Syntax -> Syntax -> Nat -> Syntax
        // Given ind_type, motive, constructor index, returns the case goal
        ctx.add_declaration(
            "induction_step_goal",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(nat),
                        body_type: Box::new(syntax.clone()),
                    }),
                }),
            },
        );

        // induction_num_cases : Syntax -> Nat
        // Returns number of constructors for an inductive type
        ctx.add_declaration(
            "induction_num_cases",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(Term::Global("Nat".to_string())),
            },
        );
    }

    // -------------------------------------------------------------------------
    // Inversion Tactic
    // -------------------------------------------------------------------------

    /// try_inversion : Syntax -> Derivation
    ///
    /// Inversion tactic: given a hypothesis type, derives False if no constructor
    /// can possibly build that type.
    ///
    /// Example: try_inversion (SApp (SName "Even") three) proves False because
    /// neither even_zero (requires 0) nor even_succ (requires Even 1) can build Even 3.
    ///
    /// Computational behavior defined in reduction.rs.
    fn register_try_inversion_tactic(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_inversion : Syntax -> Derivation
        ctx.add_declaration(
            "try_inversion",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            },
        );
    }

    // -------------------------------------------------------------------------
    // Operator Tactics (rewrite, destruct, apply)
    // -------------------------------------------------------------------------

    /// Operator tactics for manual proof control.
    ///
    /// try_rewrite : Derivation -> Syntax -> Derivation
    /// try_rewrite_rev : Derivation -> Syntax -> Derivation
    /// try_destruct : Syntax -> Syntax -> Derivation -> Derivation
    /// try_apply : Syntax -> Derivation -> Syntax -> Derivation
    fn register_operator_tactics(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_rewrite : Derivation -> Syntax -> Derivation
        // Given eq_proof (concluding Eq A x y) and goal, replaces x with y in goal
        ctx.add_declaration(
            "try_rewrite",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // goal
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // try_rewrite_rev : Derivation -> Syntax -> Derivation
        // Given eq_proof (concluding Eq A x y) and goal, replaces y with x in goal (reverse direction)
        ctx.add_declaration(
            "try_rewrite_rev",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // goal
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // try_destruct : Syntax -> Syntax -> Derivation -> Derivation
        // Case analysis without induction hypotheses
        ctx.add_declaration(
            "try_destruct",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // try_apply : Syntax -> Derivation -> Syntax -> Derivation
        // Manual backward chaining - applies hypothesis to transform goal
        ctx.add_declaration(
            "try_apply",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // hyp_name
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // hyp_proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax), // goal
                        body_type: Box::new(derivation),
                    }),
                }),
            },
        );
    }
}
