//! `format(x)` — the builtin the guide teaches as "convert any value to text" — renders exactly what
//! `Show x` prints, on every tier. It did not compile on the AOT-Rust tier at all (`expected
//! function, found macro format`): a documented everyday builtin that only the interpreters ran.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const EVERY_KIND: &str = "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n\
## Main\n\
Let n be 42.\n\
Show format(n).\n\
Show \"n is \" + format(n).\n\
Show format(0 - 7).\n\
Show format(2.5).\n\
Show format(\"text\").\n\
Show format(true).\n\
Show format([1, 2, 3]).\n\
Show format(a new Point with x 1 and y 2).\n\
Let s be format(n * 2) + \"!\".\n\
Show s.\n";

#[test]
fn format_renders_like_show_on_the_interpreters() {
    let r = common::run_all_tiers(EVERY_KIND);
    assert_eq!(r.error, None);
    let show = common::run_all_tiers(
        "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n## Main\nShow a new Point with x 1 and y 2.\nShow [1, 2, 3].\n",
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    let shown: Vec<&str> = show.output.trim().lines().collect();
    assert_eq!(lines[7], shown[0], "format(record) is Show(record)");
    assert_eq!(lines[6], shown[1], "format(seq) is Show(seq)");
    assert_eq!(lines[..6], ["42", "n is 42", "-7", "2.5", "text", "true"]);
    assert_eq!(lines[8], "84!");
}

#[test]
fn format_on_the_rust_tier() {
    assert_compiled_equals_interpreted(EVERY_KIND);
}

#[test]
fn format_of_a_runtime_value_on_the_rust_tier() {
    assert_compiled_equals_interpreted(
        "## To grow (n: Int) -> Int:\n    If n is at least 40:\n        Return n.\n    Return grow(n + 3).\n\n\
         ## Main\nLet n be grow(1).\nShow \"n is \" + format(n).\nShow format(n).\n",
    );
}

#[test]
fn a_programs_own_format_wins_on_every_tier() {
    let src = "## To format (n: Int) -> Text:\n    Return \"mine\".\n\n## Main\nShow format(3).\n";
    assert_all_tiers(src, "mine");
    assert_compiled_equals_interpreted(src);
}

#[test]
fn format_of_scalars_on_the_c_tier() {
    assert_c_output(
        "## Main\nLet n be 42.\nShow format(n).\nShow \"n is \" + format(n).\nShow format(true).\n",
        "42\nn is 42\ntrue",
    );
}

#[test]
fn format_of_scalars_on_the_c_tier_lowered_from_bytecode() {
    let src = "## Main\nLet n be 42.\nShow format(n).\nShow \"n is \" + format(n).\nShow format(true).\nShow format(2.5).\n";
    let c = logicaffeine_compile::compile::compile_to_c_via_bytecode(src).expect("lowers");
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write C");
    let gcc = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .expect("gcc");
    assert!(
        gcc.status.success(),
        "{}\n{c}",
        String::from_utf8_lossy(&gcc.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("runs");
    let main_start = c.find("int main").unwrap_or(0);
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "42\nn is 42\ntrue\n2.5",
        "status {:?}\n{}",
        run.status,
        &c[main_start..]
    );
}
