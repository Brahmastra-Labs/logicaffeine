//! ★ LOCK — reading a field of a loop variable does not consume the row. ★
//!
//! `Repeat for t in todos:` and then, in one branch, building a new record out of `t`'s fields
//! while another branch pushes `t` itself is how every "update one row, keep the others" loop is
//! written — a toggle, an edit, a soft delete. The generated Rust MOVED the field:
//!
//! ```text
//! next.push(Todo { id: t.id, text: t.text, .. });   // moves t.text
//! …
//! next.push(t);                                     // use of partially moved value
//! ```
//!
//! The two branches are mutually exclusive at RUN time and Rust cannot know that, so the borrow
//! checker refuses — correctly. Reading a field is not consuming the row, and the emitted code has
//! to say so.
//!
//! ★ IT PASSED EVERY EXISTING TEST. The tree-walker and the VM run it perfectly; `compile_to_rust`
//! returns `Ok` because the refusal is rustc's, not the LOGOS compiler's. So the only measure that
//! catches this is one that BUILDS the Rust and runs it — which is what this file does, and why it
//! lives in the e2e suite rather than beside the codegen.

mod common;
use common::run_logos;

fn e2e(src: &str, expected: &str) {
    let tw = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "the reference tier must agree with the test:\n{src}"
    );
    let r = run_logos(src);
    // ★ SHOW THE CODE WHEN IT WILL NOT BUILD. rustc names a variable in generated Rust the author
    // never wrote, so the error alone ("borrow of moved value: `r.name`") says nothing about which
    // LOGOS sentence produced it. The emitted source is the only thing that does.
    assert!(
        r.success,
        "the generated Rust must BUILD and RUN:\n{}\n--- LOGOS ---\n{src}\n--- EMITTED ---\n{}",
        r.stderr,
        logicaffeine_compile::compile::compile_to_rust(src).unwrap_or_else(|e| format!("{e:?}"))
    );
    assert_eq!(
        r.stdout.trim_end(),
        expected,
        "AOT must answer what the tree-walker answers:\n{src}"
    );
}

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    a done, which is Bool.\n\n";

#[test]
fn a_row_rebuilt_in_one_branch_and_kept_in_another_builds() {
    // ★ THE HEADLINE — the toggle loop, exactly as a real server writes it.
    let src = format!(
        "{TODO}## To toggle (todos: Seq of Todo) and (id: Int) -> Text:\n\
        \x20   Let mutable next be a new Seq of Todo.\n\
        \x20   Repeat for t in todos:\n\
        \x20       If t's id is equal to id:\n\
        \x20           Push (a new Todo with id t's id and text t's text and done (not t's done)) to next.\n\
        \x20       If t's id is not equal to id:\n\
        \x20           Push t to next.\n\
        \x20   Let mutable out be \"\".\n\
        \x20   Repeat for u in next:\n\
        \x20       Set out to out + u's text + \":\" + doneText(u's done) + \" \".\n\
        \x20   Return out.\n\
         \n\
         ## To doneText (b: Bool) -> Text:\n\
        \x20   If b:\n\
        \x20       Return \"y\".\n\
        \x20   Return \"n\".\n\
         \n\
         ## Main\n\
         Show toggle([a new Todo with id 1 and text \"a\" and done false, a new Todo with id 2 and text \"b\" and done false], 1).\n"
    );
    e2e(&src, "a:y b:n");
}

#[test]
fn reading_two_text_fields_of_one_row_still_leaves_the_row_usable() {
    // Two field reads, so a fix that cloned only the first would still move the second.
    let src = "## A Pair has:\n\
      \x20   a left, which is Text.\n\
      \x20   a right, which is Text.\n\
       \n\
       ## To go (ps: Seq of Pair) -> Text:\n\
      \x20   Let mutable out be \"\".\n\
      \x20   Repeat for p in ps:\n\
      \x20       Let joined be p's left + p's right.\n\
      \x20       Set out to out + joined + \"|\" + p's left.\n\
      \x20   Return out.\n\
       \n\
       ## Main\n\
       Show go([a new Pair with left \"a\" and right \"b\"]).\n";
    e2e(src, "ab|a");
}

#[test]
fn a_row_pushed_after_its_field_was_read_builds() {
    // The simplest form of the same thing, with no branching at all — a read, then the whole row.
    let src = format!(
        "{TODO}## To go (todos: Seq of Todo) -> Text:\n\
        \x20   Let mutable names be \"\".\n\
        \x20   Let mutable kept be a new Seq of Todo.\n\
        \x20   Repeat for t in todos:\n\
        \x20       Set names to names + t's text.\n\
        \x20       Push t to kept.\n\
        \x20   Let n be length of kept.\n\
        \x20   Return names + \"/{{n}}\".\n\
         \n\
         ## Main\n\
         Show go([a new Todo with id 1 and text \"a\" and done false, a new Todo with id 2 and text \"b\" and done true]).\n"
    );
    e2e(&src, "ab/2");
}

#[test]
fn a_row_copied_wholesale_into_a_new_record_builds() {
    // Every field read at once — the "clone with one change" shape a PUT handler has.
    let src = format!(
        "{TODO}## To rename (todos: Seq of Todo) and (name: Text) -> Text:\n\
        \x20   Let mutable next be a new Seq of Todo.\n\
        \x20   Repeat for t in todos:\n\
        \x20       Push (a new Todo with id t's id and text name and done t's done) to next.\n\
        \x20       Push t to next.\n\
        \x20   Let mutable out be \"\".\n\
        \x20   Repeat for u in next:\n\
        \x20       Set out to out + u's text.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show rename([a new Todo with id 1 and text \"old\" and done false], \"new\").\n"
    );
    e2e(&src, "newold");
}

// ─────────────────────────────────────────────────────────────────────────────
// The SAME rule at the other emission sites. The clone that makes the cases
// above work is installed in exactly two places — `Expr::New`'s init fields and
// `SetIndex` — so the identical flaw sits unexercised everywhere else a value is
// read. That is this repository's most-repeated shape: a fix lands where the bug
// was OBSERVED while its twin waits in a parallel spelling. These pin the rule
// per SITE rather than per symptom.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_record_field_passed_as_an_argument_does_not_consume_the_record() {
    // ★ F12(a). `codegen/expr.rs`'s argument path clones only a bare `Expr::Identifier`; a field
    // read falls through and is emitted as a MOVE, so passing `result's rows` to one function and
    // then reading `result` again is `error[E0382]: use of partially moved value`. The reference
    // tier and the VM run it perfectly — only a BUILD sees it.
    let src = "## A Result has:\n\
      \x20   a rows, which is Seq of Text.\n\
      \x20   a note, which is Text.\n\
       \n\
       ## To count (xs: Seq of Text) -> Int:\n\
      \x20   Return length of xs.\n\
       \n\
       ## Main\n\
       Let result be a new Result with rows [\"a\", \"b\"] and note \"n\".\n\
       Show count(result's rows).\n\
       Show length of result's rows.\n\
       Show result's note.\n";
    e2e(src, "2\n2\nn");
}

/// ★ THE RULE IS KEYED ON WHAT THE CALLEE TAKES, and two wrong directions are recorded here
/// because each looked obviously right.
///
/// 1. **Clone when the FIELD is provably non-Copy.** Needs `variable_types` to name the record,
///    and that map holds typed PARAMETERS and almost nothing else — a `Let`-bound record has no
///    entry — so it fires for `## To f (r: Row)` and stays silent one line below.
/// 2. **Clone unless the field is known Copy** (fail safe). Correct, but it emits clones where no
///    move occurs, and `phase_futamura` ratchets on the SIZE of a residual program, so every
///    specialization pays for it.
///
/// What actually decides is the CALLEE'S PARAMETER: a `&…` takes nothing, a `Copy` scalar takes a
/// copy, and only an owned non-`Copy` parameter consumes what it is given. That signature is
/// always known, because it is declared.
#[test]
fn a_text_field_passed_as_an_argument_does_not_consume_the_record() {
    // The same site with a `String` field rather than a collection — a different Rust type down
    // the same path, so a fix keyed on one container shape would miss it.
    let src = "## A Row has:\n\
      \x20   a name, which is Text.\n\
      \x20   a tag, which is Text.\n\
       \n\
       ## To shout (t: Text) -> Text:\n\
      \x20   Return upper(t).\n\
       \n\
       ## Main\n\
       Let r be a new Row with name \"ada\" and tag \"x\".\n\
       Show shout(r's name).\n\
       Show r's name.\n\
       Show r's tag.\n";
    e2e(src, "ADA\nada\nx");
}

#[test]
fn a_value_used_in_a_sequence_literal_is_still_usable_after_it() {
    // ★ F12(b). `Expr::List` emits `LogosSeq::from_vec(vec![…])` with each element passed straight
    // through — so `[workspace]` becomes `vec![workspace]`, a move, and the next mention of
    // `workspace` is a use-after-move. `analysis/ownership.rs` has no `Expr::List` arm at all, so
    // the source-level checker cannot see it either.
    let src = "## To join (xs: Seq of Text) -> Text:\n\
      \x20   Let mutable out be \"\".\n\
      \x20   Repeat for x in xs:\n\
      \x20       Set out to out + x + \".\".\n\
      \x20   Return out.\n\
       \n\
       ## Main\n\
       Let workspace be \"main\".\n\
       Show join([workspace]).\n\
       Show workspace.\n";
    e2e(src, "main.\nmain");
}

/// The same defect at the sequence-literal site — see the doc comment on
/// `a_text_field_passed_as_an_argument_does_not_consume_the_record` for why the obvious fix is
/// not the right one. `analysis/ownership.rs` has no `Expr::List` arm at all, so the source-level
/// checker cannot see this either.
#[test]
fn a_record_field_inside_a_sequence_literal_is_not_a_move_either() {
    // Both sites at once: a field read used as a list element.
    let src = "## A Cfg has:\n\
      \x20   a env, which is Text.\n\
       \n\
       ## To join (xs: Seq of Text) -> Text:\n\
      \x20   Let mutable out be \"\".\n\
      \x20   Repeat for x in xs:\n\
      \x20       Set out to out + x + \"|\".\n\
      \x20   Return out.\n\
       \n\
       ## Main\n\
       Let c be a new Cfg with env \"prod\".\n\
       Show join([c's env, c's env]).\n\
       Show c's env.\n";
    e2e(src, "prod|prod|\nprod");
}

#[test]
fn a_terminal_inspect_over_a_call_taking_a_sequence_builds() {
    // ★ F13. An `Inspect` is emitted as a bare `match` with no trailing semicolon, and nothing is
    // emitted after a function's last statement — so a TERMINAL `Inspect` becomes the function's
    // tail expression. A Seq argument at a borrow-role index is emitted `&*xs.borrow()`, and in
    // edition 2021 a tail expression's temporaries drop AFTER the block's locals, so the `Ref`
    // guard outlives the local it borrows: `error[E0597]: does not live long enough`.
    //
    // Every existing `Inspect <call>(…)` test passes Text or Int, which never takes the borrow
    // path — so this shape was untested rather than working.
    let src = "## To check (xs: Seq of Int) -> Result of Int and Text:\n\
      \x20   If length of xs is equal to 0:\n\
      \x20       Return a new Err with error \"empty\".\n\
      \x20   Return a new Ok with value (length of xs).\n\
       \n\
       ## To describe (n: Int) -> Text:\n\
      \x20   Let xs be a new Seq of Int.\n\
      \x20   Push n to xs.\n\
      \x20   Push n to xs.\n\
      \x20   Inspect check(xs):\n\
      \x20       When Ok (v):\n\
      \x20           Return \"ok {v}\".\n\
      \x20       When Err (e):\n\
      \x20           Return \"err {e}\".\n\
       \n\
       ## Main\n\
       Show describe(7).\n";
    e2e(src, "ok 2");
}
