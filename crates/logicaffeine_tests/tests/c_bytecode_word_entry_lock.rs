//! A function the bytecode-C walker runs on machine words until a value outgrows one has a word entry, `fn_N_w`: a
//! caller on machine words hands it its integers as they are and reads back a machine word, or the exact value the
//! callee escaped to with the escape flag set. These programs cross that entry every way a value can — a word in and
//! a word out, an escape inside the callee, an escape deep in a recursion carried out through every frame, an escape
//! at the outermost call, an exact argument taking the tagged entry, a result that is no integer — and each runs on
//! the walker as it runs on the VM. gcc-gated.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c_via_bytecode, vm_outcome};

const CROSSINGS: &[(&str, &str)] = &[
    (
        "tak",
        "## To tak (x: Int) (y: Int) (z: Int) -> Int:\n    If y is less than x:\n        Return tak(tak(x - 1, y, z), tak(y - 1, z, x), tak(z - 1, x, y)).\n    Return z.\n\n## Main\n    Show tak(18, 12, 6).\n    Show tak(0 - 3, 0 - 7, 5).\n",
    ),
    (
        "decrement_escapes_in_the_callee",
        "## To h (y: Int) -> Int:\n    Return y.\n\n## To g (y: Int) -> Int:\n    Return h(y - 1).\n\n## Main\n    Let m be parseInt(\"-9223372036854775808\").\n    Show g(m).\n    Show g(5).\n",
    ),
    (
        "product_escapes_deep_in_the_recursion",
        "## To f (n: Int) -> Int:\n    If n is 0:\n        Return 1.\n    Return f(n - 1) * 3.\n\n## Main\n    Show f(10).\n    Show f(50).\n    Show f(39).\n    Show f(40).\n",
    ),
    (
        "escape_at_the_outermost_call",
        "## To inc (x: Int) -> Int:\n    Return x + 1.\n\n## Main\n    Let m be parseInt(\"9223372036854775807\").\n    Show inc(m).\n    Show inc(5).\n    Show inc(m) - 1.\n",
    ),
    (
        "a_word_after_an_escape",
        "## To inc (x: Int) -> Int:\n    Return x + 1.\n\n## Main\n    Let m be parseInt(\"9223372036854775807\").\n    Repeat for i from 1 to 3:\n        Show inc(i).\n        Show inc(m).\n",
    ),
    (
        "an_exact_argument_takes_the_tagged_entry",
        "## To dec (x: Int) -> Int:\n    Return x - 1.\n\n## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show dec(p).\n    Show dec(3).\n",
    ),
    (
        "a_parsed_integer_takes_the_tagged_entry",
        "## To dec (x: Int) -> Int:\n    Return x - 1.\n\n## Main\n    Show dec(parseInt(\"5\")).\n    Show dec(parseInt(\"-9223372036854775808\")).\n    Show dec(7).\n",
    ),
    (
        "a_parsed_integer_into_a_result_that_is_no_integer",
        "## To label (x: Int) -> Text:\n    Return \"n={x}\".\n\n## Main\n    Show label(parseInt(\"5\")).\n    Show label(parseInt(\"-9223372036854775808\")).\n    Show label(7).\n",
    ),
    (
        "odd_and_negative_words_cross_untagged",
        "## To twice (x: Int) -> Int:\n    Return x + x.\n\n## Main\n    Show twice(7).\n    Show twice(0 - 3).\n    Show twice(1).\n    Show twice(parseInt(\"4611686018427387904\")).\n    Show twice(parseInt(\"-4611686018427387905\")).\n",
    ),
    (
        "a_result_that_is_no_integer",
        "## To label (x: Int) -> Text:\n    Return \"n={x - 1}\".\n\n## Main\n    Show label(parseInt(\"-9223372036854775808\")).\n    Show label(5).\n",
    ),
    (
        "an_escape_inside_a_for_each_loop",
        "## To total (xs: Seq of Int) -> Int:\n    Let mutable s be 1.\n    For each x in xs:\n        Set s to s * x.\n    Return s.\n\n## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 30:\n        Push i to xs.\n    Show total(xs).\n    Let mutable ys be a new Seq of Int.\n    Push 2 to ys.\n    Push 3 to ys.\n    Show total(ys).\n",
    ),
    (
        "an_escape_inside_a_loop_over_a_set",
        "## To product (s: Set of Int) -> Int:\n    Let mutable p be 1.\n    For each x in s:\n        Set p to p * x.\n    Return p.\n\n## Main\n    Let mutable s be a new Set of Int.\n    Repeat for i from 1 to 30:\n        Add i to s.\n    Show product(s).\n    Let mutable t be a new Set of Int.\n    Add 6 to t.\n    Add 7 to t.\n    Show product(t).\n",
    ),
    (
        "an_escape_while_the_function_owns_a_list",
        "## To build (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Let mutable p be 1.\n    Repeat for i from 1 to n:\n        Set p to p * 3.\n        Push i to xs.\n    Return p + length of xs.\n\n## Main\n    Show build(50).\n    Show build(5).\n",
    ),
    (
        "mutual_recursion",
        "## To isEven (n: Int) -> Bool:\n    If n is 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n is 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\n    Show isEven(10).\n    Show isOdd(7).\n    Show isEven(3).\n",
    ),
    (
        "an_escape_returned_through_a_word_caller",
        "## To grow (n: Int) -> Int:\n    If n is 0:\n        Return 1.\n    Return grow(n - 1) * 1000.\n\n## To outer (n: Int) -> Int:\n    Return grow(n) + 1.\n\n## Main\n    Show outer(3).\n    Show outer(9).\n",
    ),
];

fn have_gcc() -> bool {
    common::require_tool("gcc")
}

/// Standard output, standard error and exit status.
type Observed = (String, String, i32);

fn vm_observed(src: &str) -> Observed {
    let r = vm_outcome(src);
    match r.error {
        None => (r.output, String::new(), 0),
        Some(why) => (r.output, format!("{why}\n"), 1),
    }
}

fn walker_observed(src: &str, label: &str) -> Result<Observed, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c_via_bytecode(src).map_err(|e| format!("emit refused: {e:?}"))?;
    let dir =
        std::env::temp_dir().join(format!("logos_bcw_{}_{}_{}", std::process::id(), label, n));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("prog.c"), &c).map_err(|e| e.to_string())?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o", "prog", "prog.c", "-lm"])
        .current_dir(&dir)
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let out = std::process::Command::new(dir.join("prog"))
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    ))
}

#[test]
fn every_crossing_of_a_word_entry_runs_as_on_the_vm() {
    if !have_gcc() {
        return;
    }
    let mut differences = Vec::new();
    for (label, src) in CROSSINGS {
        let want = vm_observed(src);
        match walker_observed(src, label) {
            Ok(got) if got == want => {}
            Ok(got) => differences.push(format!(
                "[{label}]\n  walker {got:?}\n  vm     {want:?}\n  source {src:?}"
            )),
            Err(why) => differences.push(format!(
                "[{label}] the walker did not run it: {why}\n  vm     {want:?}\n  source {src:?}"
            )),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} programs run differently on the walker:\n{}",
        differences.len(),
        CROSSINGS.len(),
        differences.join("\n")
    );
}

/// The definition of the function whose line opens with `header` and ends with its `{` (not a forward declaration),
/// through its closing brace at column 0.
fn body_of<'c>(c: &'c str, header: &str) -> &'c str {
    let at = c
        .lines()
        .scan(0usize, |off, line| {
            let here = *off;
            *off += line.len() + 1;
            Some((here, line))
        })
        .find(|(_, line)| line.starts_with(header) && line.ends_with('{'))
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("the emitted C defines no `{header}…{{`:\n{c}"));
    let rest = &c[at..];
    &rest[..rest.find("\n}\n").map_or(rest.len(), |e| e + 2)]
}

/// The part of a body that runs on machine words: everything before the first label of a stub that leaves them
/// (`D<pc>:;`, `A<pc>:;`) or of the region held exact (`X<pc>:;`).
fn on_words(body: &str) -> &str {
    let leaves = body
        .lines()
        .scan(0usize, |off, line| {
            let here = *off;
            *off += line.len() + 1;
            Some((here, line))
        })
        .find(|(_, line)| {
            let t = line.trim_start();
            t.len() > 3
                && matches!(t.as_bytes()[0], b'D' | b'A' | b'X')
                && t.ends_with(":;")
                && t[1..t.len() - 2].chars().all(|c| c.is_ascii_digit())
        })
        .map(|(at, _)| at);
    &body[..leaves.unwrap_or(body.len())]
}

/// Takeuchi's function runs on machine words until a value outgrows one, so its body is its word entry and the part
/// held exact is a function of its own: its calls on machine words — Main's, and its own three — take the words as
/// they are (no argument is tagged, no parameter has a tag to test or strip, and the result comes back in a register
/// beside its escape flag), the word entry keeps no label of the region held exact, and its tagged entry only enters
/// one or the other.
#[test]
fn a_call_on_machine_words_boxes_nothing() {
    let (_, tak) = CROSSINGS[0];
    let c = compile_to_c_via_bytecode(tak).unwrap_or_else(|e| panic!("emit refused: {e:?}"));
    let entry = body_of(&c, "static lc_wret fn_0_w(");
    let words = on_words(entry);
    assert_eq!(
        words.matches("fn_0_w(").count(),
        1 + 3,
        "the word entry's own calls on machine words go through it:\n{entry}"
    );
    for boxing in ["lc_xof(", ">>= 1", "& 1) goto"] {
        assert!(
            !words.contains(boxing),
            "the word entry's region on machine words boxes or unboxes (`{boxing}`):\n{entry}"
        );
    }
    let main = body_of(&c, "int main(");
    assert!(
        on_words(main).contains("fn_0_w(") && !on_words(main).contains("lc_xof("),
        "Main calls on machine words through the word entry, untagged:\n{main}"
    );
    assert!(
        !entry
            .lines()
            .any(|l| l.starts_with('X') && l.ends_with(":;")),
        "the word entry carries the region held exact inside it:\n{entry}"
    );
    let held = body_of(&c, "static lc_wret fn_0_x(");
    assert!(
        held.lines()
            .any(|l| l.starts_with('X') && l.ends_with(":;")),
        "the region held exact is not in its own function:\n{held}"
    );
    let tagged = body_of(&c, "static int64_t fn_0(");
    assert!(
        tagged.contains("fn_0_w(") && tagged.contains("fn_0_x(") && !tagged.contains("goto"),
        "the tagged entry only enters the word entry or the region held exact:\n{tagged}"
    );
}
