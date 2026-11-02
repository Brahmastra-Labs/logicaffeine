//! ★ LOCK — a function's RETURN type must not change how its ARGUMENTS are checked. ★
//!
//! `## To f (x: Real) -> Real:` accepts `f(3)`: an Int literal widens to the declared `Real`, the
//! coercion LOGOS performs everywhere else. Change **only the return type** to `ComplexF` and the
//! same call is refused — "expected Int, found Real" — with the argument list untouched.
//!
//! ★ THE RETURN TYPE IS NOT AN INPUT TO ARGUMENT CHECKING. Whatever a function answers, its
//! parameters mean what they say; a rule that lets the answer reach back and re-type the inputs
//! is one that cannot be reasoned about locally, and it fails in the direction that costs most —
//! a program that is correct is refused, and the message points at an argument the author wrote
//! properly.
//!
//! ★ THIS IS NOT ABOUT COMPLEX NUMBERS, which is why the cases below are structured as a
//! comparison rather than a single reproduction. `analysis::types::from_type_name` maps a type
//! NAME to a `LogosType` and falls through to `Unknown`; `ComplexF` is absent from that table.
//! So the question this file answers is *which* absences bite: a modelled builtin, an unmodelled
//! builtin, and a user record all take different paths to a type, and only one of them is broken.
//! Pinning the reproduction alone would have left the next unmodelled name to be found by a user.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

/// Does this program reach the DEPLOY tier intact?
///
/// ★ `compile_to_rust` RETURNS `Ok` FOR A PROGRAM IT REFUSED — the refusal is a `compile_error!`
/// embedded in the Rust it hands back — so an `is_ok()` assertion proves nothing at all. Both
/// halves are checked here: the call must succeed AND the emitted program must carry no refusal.
fn aot_refusal(src: &str) -> Option<String> {
    match compile_to_rust(src) {
        Err(e) => Some(format!("{e:?}")),
        Ok(rust) => rust
            .lines()
            .find(|l| l.contains("compile_error!"))
            .map(|l| l.trim().to_string()),
    }
}

/// A function taking a `Real` and answering `ret`, called with an Int literal.
fn calls_with_int(ret: &str, body: &str) -> String {
    format!(
        "## A Todo has:\n\
         \x20   an id, which is Int.\n\n\
         ## To f (x: Real) -> {ret}:\n\
         \x20   {body}\n\n\
         ## Main\n\
         Let r be f(3).\n\
         Show \"ok\".\n"
    )
}

/// The baseline the whole file is measured against: this is ordinary, ubiquitous LOGOS.
#[test]
fn an_int_literal_widens_to_a_real_parameter() {
    let src = calls_with_int("Real", "Return x.");
    assert_eq!(tw_outcome(&src).error, None, "reference tier:\n{src}");
    assert_eq!(
        aot_refusal(&src),
        None,
        "`f(3)` into `(x: Real)` must also reach the deploy tier:\n{src}"
    );
}

/// A user-declared record as the return type — the commonest return type there is.
#[test]
fn a_user_record_return_type_does_not_disturb_the_arguments() {
    let src = calls_with_int("Todo", "Return a new Todo with id 1.");
    assert_eq!(tw_outcome(&src).error, None, "reference tier:\n{src}");
    assert_eq!(
        aot_refusal(&src),
        None,
        "a record return type must leave argument coercion alone:\n{src}"
    );
}

/// A builtin numeric type the analysis layer DOES model.
#[test]
fn a_modelled_builtin_return_type_does_not_disturb_the_arguments() {
    let src = calls_with_int("Complex", "Return complex(x, x).");
    assert_eq!(tw_outcome(&src).error, None, "reference tier:\n{src}");
    assert_eq!(
        aot_refusal(&src),
        None,
        "`Complex` IS in `from_type_name`. If this fails too, the rule is broader than an \
         unmodelled NAME and the case below is characterising the wrong thing:\n{src}"
    );
}

/// ★ THE BUG — the same call, refused because of what the function ANSWERS.
#[test]
fn an_unmodelled_builtin_return_type_does_not_disturb_the_arguments() {
    let src = calls_with_int("ComplexF", "Return complex(x, x).");
    assert_eq!(
        tw_outcome(&src).error,
        None,
        "the reference tier accepts it — the split is the point:\n{src}"
    );
    assert_eq!(
        aot_refusal(&src),
        None,
        "changing ONLY the return type to `ComplexF` refused an argument list that is correct and \
         unchanged — and refused it ONLY on the tier a program ships as. `ComplexF` is absent from \
         `analysis::types::from_type_name`, so it resolves to `Unknown`, and an unknown ANSWER must \
         not re-type the INPUTS. Green suite, broken deploy:\n{src}"
    );
}

/// ★ DISAMBIGUATION — is it the RETURN TYPE, or the BODY?
///
/// The four cases above vary the return type AND the body together (a `Real` body cannot return a
/// record), so on their own they cannot say which one moved. These two hold the return type at
/// `Real` and vary only whether the body USES the parameter. If the failure follows the body, the
/// characterisation above is wrong and the lock is pointing at the wrong thing.
#[test]
fn a_real_returning_function_that_ignores_its_parameter_still_coerces() {
    let src = calls_with_int("Real", "Return 1.0.");
    assert_eq!(tw_outcome(&src).error, None, "reference tier:\n{src}");
    assert_eq!(
        aot_refusal(&src),
        None,
        "the parameter is declared `Real` whether or not the body reads it:\n{src}"
    );
}

/// And the mirror: a body that RETURNS the parameter, which is what the `-> Real` case above does.
#[test]
fn a_real_returning_function_that_returns_its_parameter_coerces() {
    let src = calls_with_int("Real", "Return x + 0.0.");
    assert_eq!(tw_outcome(&src).error, None, "reference tier:\n{src}");
    assert_eq!(
        aot_refusal(&src),
        None,
        "arithmetic on the parameter changes nothing:\n{src}"
    );
}
