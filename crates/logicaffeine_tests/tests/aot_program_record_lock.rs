//! LOCK — a program built ahead of time compiles, and answers what the tree-walker answers, however
//! its functions reach the program's own bindings.
//!
//! A `## To` function that reads or sets a binding `## Main` declares is handed the program's
//! bindings as one lent record (`program_bindings`). The compiler writes those lends, so every
//! shape a program can put them in must build: a `Set` of a call that lends the record, an
//! argument that reads the record beside the lend, a lend nested in another lend's arguments, a
//! Text or a list read out of the record, a bignum name that must not make a same-named machine word
//! in another function a bignum, and the page machinery only a browser runs, which a server binary
//! does not compile at all.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

/// `src` on the tree-walker and as compiled Rust, both answering `expected`.
fn both(src: &str, expected: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let lines: Vec<&str> = tw.output.lines().collect();
    assert_eq!(lines, expected, "the tree-walker's answer:\n{src}");
    common::assert_output_lines(src, expected);
}

#[test]
fn a_set_of_a_call_lends_the_program_record() {
    both(
        "## To next (x: Int) -> Int:\n    Return item (x + 1) of G.\n\n\
         ## To walk (from: Int) -> Int:\n    Let mutable n be from.\n    Let mutable steps be 0.\n\
         \x20   While n != -1:\n        Set n to next(n).\n        Set steps to steps + 1.\n    Return steps.\n\n\
         ## Main\nLet mutable G be [2, 3, -1, 0].\nShow walk(0).\n",
        &["2"],
    );
}

#[test]
fn an_argument_reading_the_program_record_is_taken_before_the_lend() {
    both(
        "## To twice (x: Int) -> Int:\n    Return x * 2 + item 1 of G.\n\n\
         ## To run -> Int:\n    Return twice(item 2 of G).\n\n\
         ## Main\nLet mutable G be [1, 20].\nShow run().\n",
        &["41"],
    );
}

#[test]
fn a_call_statement_reading_the_record_beside_the_lend_is_sequenced() {
    both(
        "## To add (x: Int):\n    Set C to C + x.\n\n\
         ## To run:\n    add(C + 1).\n    add(C * 2).\n\n\
         ## Main\nLet mutable C be 1.\nrun().\nShow C.\n",
        &["9"],
    );
}

#[test]
fn a_lend_nested_in_another_lends_arguments_is_taken_first() {
    both(
        "## To tick (x: Int) -> Int:\n    Set C to C + x.\n    Return C.\n\n\
         ## To pair (a: Int) and (b: Int) -> Int:\n    Return a * 100 + b + C.\n\n\
         ## To run -> Int:\n    Return pair(tick(1), tick(10)).\n\n\
         ## Main\nLet mutable C be 0.\nShow run().\nShow C.\n",
        &["122", "11"],
    );
}

#[test]
fn a_text_or_a_list_read_out_of_the_record_is_a_copy() {
    both(
        "## To lastError -> Text:\n    Return E.\n\n\
         ## To swap:\n    Set Q to P.\n    Push 9 to Q.\n\n\
         ## Main\nLet mutable E be \"none\".\nLet mutable P be [1, 2].\nLet mutable Q be [0].\n\
         swap().\nShow lastError().\nShow P.\nShow Q.\n",
        &["none", "[1, 2]", "[1, 2, 9]"],
    );
}

/// json's serializer binds a bignum `n` out of `JNum`; `walk`'s `n` is a machine word that only
/// shares the name. Were a bignum name a bignum in every function, `after` would take and return
/// bignums, `next` would hold one, and `Set n to next` would not compile — vdom.lg's tree walks
/// are this program.
#[test]
fn a_bignum_name_is_a_bignum_only_in_its_own_function() {
    both(
        "## To after (c: Int) and (top: Int) -> Int:\n    Let mutable n be c.\n    While n != top:\n\
         \x20       Let s be item (n + 1) of NEXT.\n        If s != -1:\n            Return s.\n\
         \x20       Set n to item (n + 1) of PARENT.\n    Return -1.\n\n\
         ## To walk (root: Int) -> Int:\n    Let mutable steps be 0.\n    Let mutable n be item (root + 1) of FIRST.\n\
         \x20   While n != -1:\n        Let i be n + 1.\n        Set steps to steps + 1.\n        Let mutable next be -1.\n\
         \x20       If steps < 100:\n            Set next to item i of FIRST.\n        If next == -1:\n\
         \x20           Set next to after(n, root).\n        Set n to next.\n    Return steps.\n\n\
         ## Main\nShow jsonSerialize(a new JNum with n (2 ** 70)).\nShow jsonSerialize(jsonParse(\"42\")).\n\
         Let FIRST be [1, 3, -1, -1].\nLet NEXT be [-1, 2, -1, -1].\nLet PARENT be [-1, 0, 0, 1].\nLet n be 0.\nShow walk(n).\n",
        &["1180591620717411303424", "42", "3"],
    );
}

/// What needs no document compiles whether or not anything calls it — the author's own declaration
/// above all, even one spelled exactly as the standard library spells it.
#[test]
fn a_function_nothing_calls_still_compiles() {
    let src = "## To native write (path: Text) and (content: Text) -> Result of Unit and Text\n\n\
               ## To spare (x: Int) -> Int:\n    Return x * 3.\n\n## Main\nShow 1.\n";
    let rust = compile_to_rust(src).expect("it compiles to Rust");
    assert!(
        rust.contains("fn write(path: String, content: String)"),
        "the declared native is emitted:\n{rust}"
    );
    assert!(
        rust.contains("fn spare("),
        "the uncalled function is emitted:\n{rust}"
    );
}

/// A binding spelled like a browser builtin (`body`) is a name, not a call to the document: the
/// function holding it needs no document and compiles.
#[test]
fn a_name_spelled_like_a_browser_builtin_is_not_a_document_call() {
    let src =
        "## To frame (body: Text) -> Text:\n    Return \"<\" + body + \">\".\n\n## Main\nShow 1.\n";
    let rust = compile_to_rust(src).expect("it compiles to Rust");
    assert!(
        rust.contains("fn frame("),
        "a parameter named `body` is not the document's body:\n{rust}"
    );
}

/// A server that renders a page's component carries vdom.lg's drawing and serializing, and none of
/// the machinery only a browser document runs — its patch application least of all.
#[test]
fn the_page_machinery_a_server_never_runs_is_not_compiled() {
    let src = "## Component Card (title: Text)\nState:\n    Let n be 1.\nActions:\n    To bump:\n        Set n to n + 1.\n\
               View:\n    <li on-click={bump}>{title} {n}</li>\n\n\
               ## Route GET /c -> View:\n    <ul><Card title={\"a\"}/></ul>\n\n\
               ## Main\nShow route(\"GET\", \"/c\").\n";
    both(src, &["<ul><li>a 1</li></ul>"]);
    let rust = compile_to_rust(src).expect("it compiles to Rust");
    for browser in [
        "applyPatchWords",
        "fn vdApply(",
        "fn __render_Card(",
        "createElement",
    ] {
        assert!(
            !rust.contains(browser),
            "a server binary carries `{browser}`"
        );
    }
    assert!(
        rust.contains("fn vdSerializeChildren("),
        "and it carries the serializer it renders with"
    );
}
