//! ★ LOCK — `largo specialize`: the three Futamura projections from the command line. ★
//!
//! The offline specializer (`optimize/mix.logos`, its binding-time analysis in `optimize/mix_bta.rs`) holds all three
//! projections as library functions: `mix_projection1` specializes the self-interpreter to a program, `mix_projection2`
//! specializes the specializer to the self-interpreter (a compiler), `mix_projection3` specializes the specializer to
//! itself (a compiler generator). Until this command they were reachable only from Rust.
//!
//! ★ THE COMMAND IS THE LIBRARY, NOT A SECOND DESCRIPTION OF IT. Each projection it prints is the library's output byte
//! for byte, and a program the library refuses is refused with the library's own reason, so the command can never
//! claim more than the specializer does. When the specializer learns to carry a program's run-time input into its
//! residual, these tests stay true unchanged: they hold the command to whatever the library answers.
//!
//! ★ AND IT SAYS WHAT IT MADE. The first projection reports the residual's interpretive dispatch (`count_dispatch`,
//! Jones optimality's measure); the third projection's `--check` runs the compiler generator on the self-interpreter
//! and says whether it regenerated the second projection's compiler.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_compile::compile::{count_dispatch, mix_projection1, mix_projection2, mix_projection3, tw_outcome};

const FACTORIAL: &str = "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).\n";
const LOOP: &str = "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s.\n";
const RECORDS: &str = "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## Main\nLet s be a new Square with side 4.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Square (k):\n        Show k * k.\n";
const READS_INPUT: &str = "## To native args () -> Seq of Text\n\n## Main\nLet a be args().\nShow length of a.\n";

/// `largo specialize ARGS…` in a fresh directory holding `files`; returns (success, stdout, stderr, the directory).
fn specialize(args: &[&str], files: &[(&str, &str)]) -> (bool, String, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).expect("write source");
    }
    let mut argv = vec!["specialize"];
    argv.extend_from_slice(args);
    let out = largo_in(dir.path(), &argv);
    (out.status.success(), stdout(&out), stderr(&out), dir)
}

#[test]
fn the_first_projection_prints_what_the_library_specializes() {
    for (name, program) in [("factorial", FACTORIAL), ("loop", LOOP), ("records", RECORDS)] {
        let want = mix_projection1(program).unwrap_or_else(|e| panic!("[{name}] the library specializes it: {e}"));
        let (ok, out, err, _dir) = specialize(&["p1", "program.lg"], &[("program.lg", program)]);
        assert!(ok, "[{name}] `largo specialize p1` succeeds:\n{err}");
        assert_eq!(out, want, "[{name}] the command prints the library's residual byte for byte");
    }
}

#[test]
fn the_first_projection_reports_its_dispatch() {
    let residual = mix_projection1(FACTORIAL).expect("the library specializes factorial");
    let (ok, _out, err, _dir) = specialize(&["p1", "program.lg"], &[("program.lg", FACTORIAL)]);
    assert!(ok, "`largo specialize p1` succeeds:\n{err}");
    let want = format!("interpretive dispatch {}", count_dispatch(&residual));
    assert!(err.contains(&want), "the command reports `{want}` on stderr:\n{err}");
}

#[test]
fn the_first_projection_runs_as_the_program_runs() {
    for (name, program) in [("factorial", FACTORIAL), ("loop", LOOP), ("records", RECORDS)] {
        let (ok, residual, err, _dir) = specialize(&["p1", "program.lg"], &[("program.lg", program)]);
        assert!(ok, "[{name}] `largo specialize p1` succeeds:\n{err}");
        let (ran, want) = (tw_outcome(&residual), tw_outcome(program));
        assert_eq!(ran.error, None, "[{name}] the residual runs:\n{residual}");
        assert_eq!(ran.output, want.output, "[{name}] the residual prints what the program prints");
    }
}

#[test]
fn the_first_projection_writes_to_a_file_when_asked() {
    let want = mix_projection1(LOOP).expect("the library specializes the loop");
    let (ok, out, err, dir) = specialize(&["p1", "program.lg", "-o", "residual.lg"], &[("program.lg", LOOP)]);
    assert!(ok, "`largo specialize p1 -o` succeeds:\n{err}");
    assert!(out.is_empty(), "with -o nothing goes to stdout:\n{out}");
    let written = std::fs::read_to_string(dir.path().join("residual.lg")).expect("the residual file exists");
    assert_eq!(written, want, "the file holds the library's residual byte for byte");
}

#[test]
fn a_program_the_library_refuses_is_refused_with_the_library_reason() {
    let (ok, out, err, _dir) = specialize(&["p1", "program.lg"], &[("program.lg", READS_INPUT)]);
    match mix_projection1(READS_INPUT) {
        Ok(residual) => {
            assert!(ok, "the library specializes this program, so the command does:\n{err}");
            assert_eq!(out, residual);
        }
        Err(reason) => {
            assert!(!ok, "the library refuses this program, so the command fails");
            assert!(err.contains(&reason), "the command gives the library's reason `{reason}`:\n{err}");
        }
    }
}

#[test]
fn the_second_projection_writes_the_library_compiler() {
    let want = mix_projection2().expect("the library builds the compiler");
    let (ok, _out, err, dir) = specialize(&["p2", "-o", "compiler.lg"], &[]);
    assert!(ok, "`largo specialize p2` succeeds:\n{err}");
    let written = std::fs::read_to_string(dir.path().join("compiler.lg")).expect("the compiler file exists");
    assert!(written == want, "the file holds the library's compiler byte for byte ({} vs {} bytes)", written.len(), want.len());
}

#[test]
fn the_third_projection_writes_the_library_generator_and_checks_it() {
    let want = mix_projection3().expect("the library builds the compiler generator");
    let (ok, out, err, dir) = specialize(&["p3", "-o", "cogen.lg", "--check"], &[]);
    let written = std::fs::read_to_string(dir.path().join("cogen.lg")).expect("the generator file exists");
    assert!(written == want, "the file holds the library's compiler generator byte for byte");
    let said = format!("{out}{err}");
    assert!(
        said.contains("regenerates the second projection's compiler: yes"),
        "--check reports that the generator regenerated the compiler (success {ok}):\n{said}"
    );
    assert!(ok, "--check succeeds when the generator regenerates the compiler:\n{said}");
}

#[test]
fn a_projection_the_command_does_not_know_is_a_usage_error() {
    let (ok, _out, err, _dir) = specialize(&["p4"], &[]);
    assert!(!ok, "`largo specialize p4` fails");
    for name in ["p1", "p2", "p3"] {
        assert!(err.contains(name), "the usage error names `{name}`:\n{err}");
    }
}

#[test]
fn the_first_projection_needs_a_program() {
    let (ok, _out, err, _dir) = specialize(&["p1"], &[]);
    assert!(!ok, "`largo specialize p1` without a file fails");
    assert!(err.to_lowercase().contains("program"), "the error says a program is needed:\n{err}");
}
