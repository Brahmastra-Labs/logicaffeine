//! FrameEval — evaluate a LOGOS expression against a paused frame's bindings.
//!
//! ONE evaluator behind every debug surface: conditional breakpoints, the debug
//! REPL, DAP `evaluate`, watch expressions, and the Studio Prove tab. It reuses
//! the tree-walker — the differential oracle for the VM — so the answer
//! provably agrees with program semantics: it IS program semantics. Read-only:
//! it observes a frame, never mutates it.

/// One name→value binding in the paused frame. The value is given as LOGOS
/// source (a literal or expression the tree-walker evaluates to seed the name).
pub struct Binding<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

/// Evaluate `expr` (LOGOS expression source) against `frame` (the paused
/// bindings), returning its rendered value. Errors are the tree-walker's own,
/// so a bad expression fails exactly as it would in a program.
pub fn frame_eval(frame: &[Binding<'_>], expr: &str) -> Result<String, String> {
    let mut src = String::from("## Main\n");
    for b in frame {
        src.push_str(&format!("    Let {} be {}.\n", b.name, b.value));
    }
    src.push_str(&format!("    Show {expr}.\n"));
    let outcome = crate::compile::tw_outcome(&src);
    match outcome.error {
        Some(e) => Err(e),
        None => Ok(outcome.output),
    }
}

/// Evaluate a yes/no question against a frame, returning an English `Yes.`/`No.`
/// (the debug REPL's `Is …?`). `Is X <pred>?` de-inverts to the condition
/// `X is <pred>`, evaluated through the tree-walker's own `If`.
pub fn frame_answer(frame: &[Binding<'_>], question: &str) -> Result<String, String> {
    let clause = question.trim().trim_end_matches('?').trim();
    // A question (`Is X <pred>`) de-inverts to `X is <pred>`; a declarative
    // condition passes through unchanged.
    let condition = if let Some(rest) = clause
        .strip_prefix("Is ")
        .or_else(|| clause.strip_prefix("is "))
    {
        match rest.split_once(' ') {
            Some((subject, predicate)) => format!("{subject} is {predicate}"),
            None => rest.to_string(),
        }
    } else {
        clause.to_string()
    };

    let mut src = String::from("## Main\n");
    for b in frame {
        src.push_str(&format!("    Let {} be {}.\n", b.name, b.value));
    }
    src.push_str(&format!(
        "    If {condition}:\n        Show \"yes\".\n    Otherwise:\n        Show \"no\".\n"
    ));
    let outcome = crate::compile::tw_outcome(&src);
    match outcome.error {
        Some(e) => Err(e),
        None => Ok(match outcome.output.as_str() {
            "yes" => "Yes.".to_string(),
            "no" => "No.".to_string(),
            other => other.to_string(),
        }),
    }
}
