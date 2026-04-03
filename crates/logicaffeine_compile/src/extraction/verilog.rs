//! Verilog Extraction: Kernel Term → SystemVerilog
//!
//! Converts kernel proof terms (which ARE circuits under Curry-Howard)
//! into synthesizable SystemVerilog source code.
//!
//! # Extraction Rules
//!
//! | Kernel Term | SystemVerilog Output |
//! |---|---|
//! | `Global("bit_and")` applied to a, b | `a & b` |
//! | `Global("bit_or")` applied to a, b | `a \| b` |
//! | `Global("bit_not")` applied to a | `~a` |
//! | `Global("bit_xor")` applied to a, b | `a ^ b` |
//! | `Global("bit_mux")` applied to s, a, b | `s ? a : b` |
//! | `Global("B0")` | `1'b0` |
//! | `Global("B1")` | `1'b1` |
//! | `Var(name)` | `name` |

use logicaffeine_kernel::Term;

/// Convert a kernel Term to a SystemVerilog expression string.
///
/// This handles the hardware-specific subset of kernel Terms:
/// gate operations, constants, variables, and their compositions.
pub fn term_to_verilog(term: &Term) -> String {
    // Collect the application chain: flatten App(App(App(f, a), b), c) → (f, [a, b, c])
    let (head, args) = collect_app_chain(term);

    if let Term::Global(name) = &head {
        match (name.as_str(), args.len()) {
            // Constants
            ("B0", 0) => return "1'b0".to_string(),
            ("B1", 0) => return "1'b1".to_string(),
            ("Tt", 0) => return "/* unit */".to_string(),

            // Unary operations
            ("bit_not", 1) => {
                let a = term_to_verilog(&args[0]);
                return format!("(~{})", a);
            }

            // Binary operations
            ("bit_and", 2) => {
                let a = term_to_verilog(&args[0]);
                let b = term_to_verilog(&args[1]);
                return format!("({} & {})", a, b);
            }
            ("bit_or", 2) => {
                let a = term_to_verilog(&args[0]);
                let b = term_to_verilog(&args[1]);
                return format!("({} | {})", a, b);
            }
            ("bit_xor", 2) => {
                let a = term_to_verilog(&args[0]);
                let b = term_to_verilog(&args[1]);
                return format!("({} ^ {})", a, b);
            }

            // Ternary: bit_mux sel then_v else_v → sel ? then_v : else_v
            ("bit_mux", 3) => {
                let sel = term_to_verilog(&args[0]);
                let then_v = term_to_verilog(&args[1]);
                let else_v = term_to_verilog(&args[2]);
                return format!("({} ? {} : {})", sel, then_v, else_v);
            }

            // BVec operations
            ("bv_and", 3) => {
                // bv_and n v1 v2 — skip the Nat width argument
                let a = term_to_verilog(&args[1]);
                let b = term_to_verilog(&args[2]);
                return format!("({} & {})", a, b);
            }
            ("bv_or", 3) => {
                let a = term_to_verilog(&args[1]);
                let b = term_to_verilog(&args[2]);
                return format!("({} | {})", a, b);
            }
            ("bv_xor", 3) => {
                let a = term_to_verilog(&args[1]);
                let b = term_to_verilog(&args[2]);
                return format!("({} ^ {})", a, b);
            }
            ("bv_not", 2) => {
                let a = term_to_verilog(&args[1]);
                return format!("(~{})", a);
            }

            // Partial application or unknown global — emit as function call
            (fname, _) if !args.is_empty() => {
                let arg_strs: Vec<String> = args.iter().map(|a| term_to_verilog(a)).collect();
                return format!("{}({})", fname, arg_strs.join(", "));
            }

            // Bare global (no args) — emit as identifier
            (gname, 0) => return gname.to_string(),

            _ => {}
        }
    }

    // Variable reference
    if let Term::Var(name) = term {
        return name.clone();
    }

    // Lambda — emit as comment (lambdas become module ports at higher level)
    if let Term::Lambda { param, body, .. } = term {
        let body_sv = term_to_verilog(body);
        return format!("/* λ{} */ {}", param, body_sv);
    }

    // Match on Bit — ternary operator
    if let Term::Match { discriminant, cases, .. } = term {
        if cases.len() == 2 {
            let disc = term_to_verilog(discriminant);
            let case_b0 = term_to_verilog(&cases[0]);
            let case_b1 = term_to_verilog(&cases[1]);
            return format!("({} ? {} : {})", disc, case_b1, case_b0);
        }
    }

    // Literal
    if let Term::Lit(lit) = term {
        return format!("{}", lit);
    }

    // Fallback
    format!("/* unsupported: {} */", term)
}

/// Flatten a left-associative application chain.
/// App(App(App(f, a), b), c) → (f, [a, b, c])
fn collect_app_chain(term: &Term) -> (Term, Vec<Term>) {
    let mut args = Vec::new();
    let mut current = term.clone();

    while let Term::App(func, arg) = current {
        args.push(*arg);
        current = *func;
    }

    args.reverse();
    (current, args)
}
