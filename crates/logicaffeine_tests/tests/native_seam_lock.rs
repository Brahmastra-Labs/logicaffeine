//! ★ LOCK — a `## To native` declaration reaches its implementation on every engine, or is refused by name. ★
//!
//! A native declaration has no body: the engine supplies the function. An engine that runs the empty body
//! instead answers `nothing`, and the program carries on with a value no engine computed — `get` answering
//! `none` whatever the environment holds, `now()` answering `nothing` where an Int belongs. So there are
//! exactly two outcomes for a call, on every engine:
//!
//! * The engine implements the name, through the one table the tree-walker and the VM both read
//!   (`semantics::builtins::system_native`), the native kernels, or the socket seam.
//! * The call is refused in one sentence, the same on every engine: a name no engine implements in the
//!   Rust backends' own compile-time words, a Rust function (`is native "crate::path"`) as `Escape to
//!   Rust:` is refused wherever Rust does not link it.
//!
//! The environment half is the documented promise behind sessions: `LOGOS_SESSION_SECRET` sets the key a
//! session cookie is signed with (`docs/web.md`), so a cookie signed under a known key on each engine must be
//! the HMAC-SHA-256 an independent implementation computes.

#[path = "support/under_test.rs"]
mod under_test;

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// A variable every test process runs with, and one no process sets.
const READS: &str = "## Main\n\
     Inspect get(\"PATH\"):\n\
     \x20   When Some (p):\n\
     \x20       Show length of p is greater than 0.\n\
     \x20   When None:\n\
     \x20       Show \"unset\".\n\
     Inspect get(\"LOGOS_NATIVE_SEAM_NEVER_SET\"):\n\
     \x20   When Some (v):\n\
     \x20       Show v.\n\
     \x20   When None:\n\
     \x20       Show \"unset\".\n";

#[test]
fn get_reads_the_environment_on_the_tree_walker_and_the_vm() {
    for (engine, out) in [("tree-walker", tw_outcome(READS)), ("vm", vm_outcome(READS))] {
        assert_eq!(out.error, None, "{engine} must run:\n{READS}");
        assert_eq!(
            out.output.trim(),
            "true\nunset",
            "{engine} must read the variable that is set and answer none for the one that is not"
        );
    }
}

#[test]
fn now_is_the_unix_time_in_milliseconds_on_the_tree_walker_and_the_vm() {
    // The bare `now` is a moment; time.lg's `now()` is an Int count of milliseconds since 1970.
    let src = "## Main\nLet t be now().\nShow t is greater than 1700000000000.\nShow t is less than 9000000000000.\n";
    for (engine, out) in [("tree-walker", tw_outcome(src)), ("vm", vm_outcome(src))] {
        assert_eq!(out.error, None, "{engine} must run `now()`:\n{src}");
        assert_eq!(out.output.trim(), "true\ntrue", "{engine}: `now()` is milliseconds");
    }
}

/// The Rust backends' refusal of a native no engine implements, which every other engine answers too.
const UNKNOWN: &str = "Unknown system native function: 'blarf'. Use `is native \"crate::path\"` syntax \
                       for user-defined native functions.";

#[test]
fn a_native_no_engine_implements_is_refused_in_one_sentence_on_every_engine() {
    let src = "## To native blarf (x: Int) -> Int\n\n## Main\nShow \"before\".\nShow blarf(1).\n";
    for (engine, out) in [("tree-walker", tw_outcome(src)), ("vm", vm_outcome(src))] {
        assert_eq!(out.output.trim(), "before", "{engine} runs up to the call");
        assert_eq!(
            out.error.as_deref(),
            Some(UNKNOWN),
            "{engine} must refuse the call by name, never answer the empty body's nothing"
        );
    }
    let rust = compile_to_rust(src).expect("the Rust backend writes its refusal into the program");
    assert!(
        rust.contains(&format!("compile_error!({UNKNOWN:?})")),
        "the Rust backends refuse in the same sentence:\n{rust}"
    );
}

#[test]
fn a_rust_function_is_refused_where_rust_does_not_link_it() {
    let src = "## To cube (n: Int) -> Int is native \"my_crate::cube\".\n\n## Main\nShow \"before\".\nShow cube(2).\n";
    let refusal = "`cube` is the Rust function `my_crate::cube`, which only the Rust backends compile. \
                   Use `largo build` or `largo run` to compile and run this program.";
    for (engine, out) in [("tree-walker", tw_outcome(src)), ("vm", vm_outcome(src))] {
        assert_eq!(out.output.trim(), "before", "{engine} runs up to the call");
        assert_eq!(out.error.as_deref(), Some(refusal), "{engine}");
    }
    let rust = compile_to_rust(src).expect("the Rust backend links the path");
    assert!(
        rust.contains("my_crate::cube(n)"),
        "the Rust backend calls the function it names:\n{rust}"
    );
}

#[test]
fn a_native_only_rust_links_is_refused_by_the_rust_function_it_names() {
    // A program that declares a FIPS-202 hash native takes the Rust implementation in place of crypto.lg's
    // pure-LOGOS sponge, which only the Rust backends link.
    let src = "## To native sha3_256 (a: Seq of Int) -> Seq of Int\n\n## Main\nShow \"before\".\nShow sha3_256([97, 98, 99]).\n";
    let refusal = "`sha3_256` is the Rust function `logicaffeine_system::keccak::sha3_256`, which only the Rust \
                   backends compile. Use `largo build` or `largo run` to compile and run this program.";
    for (engine, out) in [("tree-walker", tw_outcome(src)), ("vm", vm_outcome(src))] {
        assert_eq!(out.output.trim(), "before", "{engine} runs up to the call");
        assert_eq!(out.error.as_deref(), Some(refusal), "{engine}");
    }
    let rust = compile_to_rust(src).expect("the Rust backend links the hash");
    assert!(
        rust.contains("logicaffeine_system::keccak::sha3_256("),
        "the Rust backend calls the function the refusal names:\n{rust}"
    );
}

#[test]
fn a_program_of_its_own_named_like_a_native_is_called_as_written() {
    // Declarer wins: a `get` with a body is the program's function, not the environment's.
    let src = "## To get (name: Text) -> Text:\n    Return \"mine:\" + name.\n\n## Main\nShow get(\"PATH\").\n";
    for (engine, out) in [("tree-walker", tw_outcome(src)), ("vm", vm_outcome(src))] {
        assert_eq!(out.error, None, "{engine}");
        assert_eq!(out.output.trim(), "mine:PATH", "{engine}");
    }
}

/// Each `## To native` the standard library declares, as its name and its parameters' types.
fn stdlib_natives() -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for source in logicaffeine_compile::loader::prelude_module_sources() {
        for line in source.lines() {
            let Some(rest) = line.strip_prefix("## To native ") else {
                continue;
            };
            let name = rest.split([' ', '(']).next().unwrap_or("").to_string();
            let params = rest
                .match_indices('(')
                .filter_map(|(at, _)| {
                    let inner = &rest[at + 1..at + rest[at..].find(')')?];
                    inner.split_once(": ").map(|(_, ty)| ty.trim().to_string())
                })
                .collect();
            out.push((name, params));
        }
    }
    out
}

/// An argument of `ty`, from the locals the census program binds.
fn argument_of(ty: &str) -> &'static str {
    match ty {
        "Int" | "Nat" => "0",
        "Text" => "\"\"",
        "Seq of Int" => "ints",
        "Seq of Word16" => "words",
        other => panic!("a native takes a {other}; give the census an argument of that type"),
    }
}

/// The refusal a compiled program carries for a native call no seam took, if it carries one.
fn refusal_in(program: &logicaffeine_compile::vm::CompiledProgram) -> Option<String> {
    program.constants.iter().find_map(|c| match c {
        logicaffeine_compile::vm::Constant::Text(t)
            if t.starts_with("Unknown system native function:")
                || t.contains("which only the Rust backends compile") =>
        {
            Some(t.clone())
        }
        _ => None,
    })
}

/// A call to `name` from a branch no run takes (the VM compiles the call either way, and nothing a
/// socket or a clock would do happens).
fn unreached_call(name: &str, args: &[&str]) -> String {
    format!(
        "## Main\nLet ints be [0].\nLet words be a new Seq of Word16.\n\
         If length of args() is less than 0:\n    Show {name}({}).\n",
        args.join(", ")
    )
}

#[test]
fn every_standard_library_native_reaches_an_implementation_on_the_vm() {
    let natives = stdlib_natives();
    assert!(
        natives.iter().any(|(n, _)| n == "get") && natives.iter().any(|(n, _)| n == "tcpConnect"),
        "the census must read the standard library's natives: {natives:?}"
    );
    let mut refused = Vec::new();
    for (name, params) in &natives {
        let args: Vec<&str> = params.iter().map(|t| argument_of(t)).collect();
        let src = unreached_call(name, &args);
        let program = logicaffeine_compile::compile::compile_to_program(&src)
            .unwrap_or_else(|e| panic!("a call to `{name}` must compile: {e:?}\n{src}"));
        if let Some(why) = refusal_in(&program) {
            refused.push(format!("{name}: {why}"));
        }
    }
    assert!(
        refused.is_empty(),
        "{} standard-library natives reach no implementation on the VM:\n{}",
        refused.len(),
        refused.join("\n")
    );
}

#[test]
fn the_native_census_sees_a_native_no_engine_implements() {
    let src = format!("## To native blarf (x: Int) -> Int\n\n{}", unreached_call("blarf", &["0"]));
    let program = logicaffeine_compile::compile::compile_to_program(&src).expect("compiles");
    assert_eq!(
        refusal_in(&program).as_deref(),
        Some(UNKNOWN),
        "the census must fail for a native no engine implements"
    );
}

/// A login that writes the visitor's name into the session and shows the cookie the reply sets.
const LOGIN: &str = "## Session\n\
     \x20   a user, which is Text.\n\n\
     ## To mk (m: Text) and (p: Text) and (c: Text) -> Request:\n\
     \x20   Let h be a new Map of Text to Text.\n\
     \x20   Set item \"cookie\" of h to c.\n\
     \x20   Return a new Request with method m and path p and body \"\" and headers h and query (a new Map of Text to Text).\n\n\
     ## Route GET /login -> Response:\n\
     \x20   Set the session's user to \"ada\".\n\
     \x20   Return plain(\"in\").\n\n\
     ## Main\n\
     Let a be __lgx_route_handler(mk(\"GET\", \"/login\", \"\")).\n\
     Show headerIn(a's headers, \"set-cookie\").\n";

/// The payload `616461.` (the name, hex, then the empty CSRF nonce field) signed under the key `probe1`:
/// HMAC-SHA-256 as Python's `hmac` module computes it.
const SIGNED_UNDER_PROBE1: &str = "lgxsession=616461.|cabf97ab9ada47c04be930f6d6961b3ea2639f1ffd30cabc3e4468526d15c37a; \
                                   Path=/; HttpOnly; SameSite=Lax; Secure";

fn project(source: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.lg"), source).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"seam\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    dir
}

/// `largo` in `dir` with no `LOGOS_*` variable of the test's own environment, so the run sees exactly the
/// ones the test sets.
fn largo_in(dir: &Path) -> Command {
    let mut cmd = Command::new(under_test::largo());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("LOGOS_") && key != "LOGOS_WORKSPACE" {
            cmd.env_remove(&key);
        }
    }
    cmd.current_dir(dir);
    cmd
}

#[test]
fn logos_session_secret_signs_the_session_cookie_on_every_engine() {
    let dir = project(LOGIN);
    for engine in ["tree", "vm", "jit"] {
        let out = largo_in(dir.path())
            .env("LOGOS_SESSION_SECRET", "probe1")
            .args(["-q", "run", "--interpret", "--engine", engine])
            .output()
            .expect("spawn largo");
        assert!(
            out.status.success(),
            "`largo run --interpret --engine {engine}` exited {:?}:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            SIGNED_UNDER_PROBE1,
            "{engine}: the cookie must be signed with the key LOGOS_SESSION_SECRET names"
        );
    }
}
