//! ★ LOCK — a whole program run on the tree-walker hosts its own page LOWERED. ★
//!
//! The interpreter seats the page at the root of the run: the program executes top to bottom as
//! always, and every statement that acts on the page — the page's own mount included — is carried
//! out by the one driver every lowered tier shares (`page_host::LoweredHost`), with the same
//! interpreter as the engine it calls into. The page's main is the program up to its first
//! statement that acts on the page (`page_host::page_main_len`); a reload runs that main again and
//! mounts the `## Page` again.

use logicaffeine_compile::ast::stmt::Stmt;
use logicaffeine_compile::interpreter::Interpreter;
use logicaffeine_compile::page_host::{drives_the_page, page_main_len};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;

/// What one run on the lowered seat left: its output, its error (the run's own, else the page's
/// unhandled action failure), the document, and how many compiler-written globals it holds.
struct SeatRun {
    output: Vec<String>,
    error: Option<String>,
    document: String,
    page_globals: usize,
}

fn run_on_the_lowered_seat(src: &str) -> SeatRun {
    dom::reset();
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) =
            parsed.unwrap_or_else(|e| panic!("the program must parse:\n{src}\n=> {e}"));
        let mut interp = Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        let ran = interp.run_sync(stmts);
        let unhandled = interp.take_page_failure();
        SeatRun {
            error: ran.err().or(unhandled),
            document: dom::render(),
            page_globals: interp
                .global_bindings()
                .iter()
                .filter(|(name, _, _)| name.starts_with("__"))
                .count(),
            output: interp.output.clone(),
        }
    })
}

const COUNTER: &str = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={bump}>b</button>

";

#[test]
fn a_main_script_drives_its_page_through_the_lowered_host() {
    let run = run_on_the_lowered_seat(&format!(
        "{COUNTER}## Main\nClick \"#b\".\nClick \"#b\".\nShow the text of \"#n\".\n"
    ));
    assert_eq!(run.error, None);
    assert_eq!(
        run.output,
        ["2"],
        "the script's clicks reached the page it reads back"
    );
    assert!(
        run.document.contains("<p id=\"n\">2</p>"),
        "{}",
        run.document
    );
    assert!(
        run.page_globals > 0,
        "the page lives in its lowered globals"
    );
}

#[test]
fn a_reload_on_the_lowered_seat_mounts_the_page_again_once_and_fresh() {
    let run = run_on_the_lowered_seat(&format!(
        "{COUNTER}## Main\nClick \"#b\".\nReload the page.\nShow the text of \"#n\".\nClick \"#b\".\nShow the text of \"#n\".\n"
    ));
    assert_eq!(run.error, None);
    assert_eq!(run.output, ["0", "1"], "fresh after the reload, and live");
    assert_eq!(
        run.document.matches("id=\"n\"").count(),
        1,
        "one page: {}",
        run.document
    );
}

#[test]
fn an_unhandled_action_failure_is_the_programs_failure_and_the_page_keeps_answering() {
    let src = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
    To boom:
        Set n to 100.
        Let xs be [1].
        Show item 5 of xs.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={bump}>b</button>
    <button id=\"x\" on-click={boom}>x</button>

## Main
Click \"#x\".
Click \"#b\".
Show the text of \"#n\".
";
    let run = run_on_the_lowered_seat(src);
    assert_eq!(
        run.output,
        ["1"],
        "the trapping turn rolled back and the next click landed"
    );
    let error = run
        .error
        .expect("the unhandled failure is the program's failure");
    assert!(error.contains('5'), "it is the trap's own message: {error}");
}

#[test]
fn the_pages_main_is_the_program_up_to_its_first_statement_that_acts_on_the_page() {
    let src = format!("{COUNTER}## Main\nLet before be 1.\nClick \"#b\".\nLet after be 2.\n");
    with_parsed_program(&src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        let main = page_main_len(stmts, interner);
        assert!(main < stmts.len());
        assert!(
            stmts[..main].iter().all(|s| !drives_the_page(s, interner)),
            "nothing in the page's main acts on the page"
        );
        assert!(
            drives_the_page(&stmts[main], interner),
            "the main ends at the first statement that does"
        );
        assert!(
            matches!(stmts[main], Stmt::Ui(_)),
            "and for a page file that is the page's own mount, placed before the script's first click"
        );
        let before = interner.lookup("before").expect("interned");
        assert!(
            stmts[..main]
                .iter()
                .any(|s| matches!(s, Stmt::Let { var, .. } if *var == before)),
            "a binding written before the page mounts belongs to the page's main"
        );
        let after = interner.lookup("after").expect("interned");
        assert!(
            !stmts[..main]
                .iter()
                .any(|s| matches!(s, Stmt::Let { var, .. } if *var == after)),
            "a binding written after the page is driven does not"
        );
    });
}

#[test]
fn a_driving_statement_with_a_computed_operand_still_acts_on_the_page() {
    let src = format!("{COUNTER}## Main\nLet which be \"b\".\nClick \"#\" + which.\n");
    with_parsed_program(&src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        let click = stmts
            .iter()
            .rev()
            .find(|s| matches!(s, Stmt::Ui(_)))
            .expect("the click");
        assert!(drives_the_page(click, interner));
    });
    let run = run_on_the_lowered_seat(&format!(
        "{COUNTER}## Main\nLet which be \"b\".\nClick \"#\" + which.\nShow the text of \"#n\".\n"
    ));
    assert_eq!(run.error, None);
    assert_eq!(run.output, ["1"]);
}
