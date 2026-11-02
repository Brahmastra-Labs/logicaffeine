//! G9 — `largo check --deep` must complete (never panic) on real project
//! entry files, with findings mapped back to the user's own lines.
//!
//! The prelude auto-import splices stdlib modules AFTER a leading `# Title` /
//! `--` prose head (and `## NoPrelude` strips its marker), so the expanded
//! source is not always `prelude + user_source`. The deep-check pipeline must
//! invert whatever edit `apply_prelude` actually made — a debug build of
//! `largo` turns any drift into a loud panic, which these shapes lock out.
//!
//! Every test drives the real binary against a scaffolded project. One shared
//! project directory + lock keeps the flycheck cache warm and guarantees the
//! suite never runs two cargo checks at once.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use std::path::PathBuf;
use std::process::Output;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn workbench() -> &'static (PathBuf, Mutex<()>) {
    static BENCH: OnceLock<(PathBuf, Mutex<()>)> = OnceLock::new();
    BENCH.get_or_init(|| {
        let dir = std::env::temp_dir().join("logos_check_deep_regression");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Largo.toml"),
            "[package]\nname = \"deepreg\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
        )
        .unwrap();
        (dir, Mutex::new(()))
    })
}

/// Write `program` as the entry and run `largo check --deep`, serialized so
/// only one cargo check ever runs at a time.
fn deep_check(program: &str) -> Output {
    let (dir, lock) = workbench();
    let _guard: MutexGuard<'_, ()> = lock.lock().unwrap_or_else(|p| p.into_inner());
    std::fs::write(dir.join("src/main.lg"), program).unwrap();
    largo_in(dir, &["check", "--deep"])
}

/// The invariant every shape shares: the process finished on its own terms —
/// no panic in stderr, no 101 abort.
fn assert_no_panic(out: &Output) {
    let err = stderr(out);
    assert!(
        !err.contains("panicked"),
        "check --deep must never panic:\n{err}"
    );
    assert_ne!(
        out.status.code(),
        Some(101),
        "check --deep must never abort:\nstdout: {}\nstderr: {err}",
        stdout(out)
    );
}

/// A clean program deep-checks clean: exit 0 and an explicit verdict.
fn assert_deep_clean(out: &Output) {
    assert_no_panic(out);
    let all = format!("{}{}", stdout(out), stderr(out));
    assert_eq!(
        out.status.code(),
        Some(0),
        "a clean program must pass deep check:\n{all}"
    );
    assert!(
        all.contains("Deep check passed"),
        "deep check must report its verdict:\n{all}"
    );
}

#[test]
fn leading_title_head_with_stdlib_splice_deep_checks_clean() {
    // A `# Title` head means the prelude splices AFTER it, not at byte 0.
    let out = deep_check("# My App\n\n## Main\nShow upper(\"hello\").\n");
    assert_deep_clean(&out);
}

#[test]
fn leading_comment_prose_head_with_stdlib_splice_deep_checks_clean() {
    let out = deep_check("-- a note about this program\n\n## Main\nShow upper(\"hello\").\n");
    assert_deep_clean(&out);
}

#[test]
fn crlf_entry_with_title_head_deep_checks_clean() {
    let out = deep_check("# My App\r\n\r\n## Main\r\nShow upper(\"x\").\r\n");
    assert_deep_clean(&out);
}

#[test]
fn no_prelude_opt_out_deep_checks_clean() {
    // `## NoPrelude` is STRIPPED, so the parsed text is shorter than the
    // user's file — spans must shift forward past the removed marker.
    let out = deep_check("## NoPrelude\n## Main\nShow 1 plus 1.\n");
    assert_deep_clean(&out);
}

#[test]
fn serve_handler_site_shape_deep_checks_clean() {
    // The canonical web-app entry: title head + http.lg splice + `Serve`.
    // The staged check project must carry the same runtime features the real
    // build does, or every handler drowns in phantom findings.
    let out = deep_check(
        "# Site\n\n\
         ## To handle (request: Request) -> Response:\n\
         \x20   If request's path is equal to \"/\":\n\
         \x20       Return html(\"<h1>home</h1>\").\n\
         \x20   Return plain(request's path).\n\n\
         ## Main\n\
         Serve on port 18099 with handle.\n",
    );
    assert_deep_clean(&out);
}

#[test]
fn plain_section_first_stdlib_program_deep_checks_clean() {
    // No prose head: the prelude genuinely prepends. Still had to be locked —
    // the spliced strings.lg lowers `indexOf`/`countSub` to the `memchr`
    // crate, which the staged check project must declare like the build does.
    let out = deep_check("## Main\nShow upper(\"hello\").\n");
    assert_deep_clean(&out);
}

#[test]
fn multi_module_splice_with_title_head_deep_checks_clean() {
    // Two stdlib modules (strings.lg + math.lg) spliced after the head.
    let out =
        deep_check("# My App\n\n## Main\nShow repeatText(\"ab\", 2).\nShow clamp(5, 1, 3).\n");
    assert_deep_clean(&out);
}

#[test]
fn deep_finding_is_source_mapped_through_the_splice() {
    // A real finding in a program whose prelude spliced mid-file must point
    // at the USER's line — line 5, where `bogusFn` lives — not at prelude
    // text and not at a shifted offset.
    let out = deep_check("# My App\n\n## Main\nShow upper(\"hello\").\nShow bogusFn(1).\n");
    assert_no_panic(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unknown function is a finding, not a crash:\nstdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("bogusFn"),
        "the finding names the unknown function:\n{err}"
    );
    assert!(
        err.contains("main.lg:5"),
        "the finding maps to the user's own line 5:\n{err}"
    );
}
