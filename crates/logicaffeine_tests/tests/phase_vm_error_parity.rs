//! Error-string parity: the Err text and the partial output emitted before the
//! error are part of the LOGOS spec. Every program here runs through BOTH
//! engines via the SAME front-end; outcome (output + error) must be identical.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_system::testworld::{self, Capabilities, TestWorld};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The differential contract: output AND error must match.
fn assert_outcome_parity(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        norm(&vm.output),
        norm(&tw.output),
        "partial output diverged for:\n{}\nvm: {:?}\ntw: {:?}",
        src,
        vm,
        tw
    );
    assert_eq!(vm.error, tw.error, "error diverged for:\n{}", src);
}

#[test]
fn err_parity_div_by_zero() {
    assert_outcome_parity("## Main\nLet x be 1 / 0.\nShow x.\n");
}

#[test]
fn err_parity_mod_by_zero() {
    assert_outcome_parity("## Main\nLet x be 1 % 0.\nShow x.\n");
}

#[test]
fn err_parity_index_out_of_bounds() {
    assert_outcome_parity("## Main\nLet xs be [1, 2].\nShow item 5 of xs.\n");
}

#[test]
fn err_parity_index_zero() {
    assert_outcome_parity("## Main\nLet xs be [1, 2].\nShow item 0 of xs.\n");
}

#[test]
fn err_parity_negative_index_prints_wrapped_usize() {
    // A negative index wraps through `as usize`; both engines print the
    // wrapped number — pinned behavior.
    assert_outcome_parity("## Main\nLet xs be [1, 2].\nLet i be 0 - 1.\nShow item i of xs.\n");
}

#[test]
fn err_parity_set_undefined_variable() {
    // The tree-walker fails the assignment at RUNTIME, after evaluating the
    // value and after earlier statements produced output.
    assert_outcome_parity("## Main\nShow 1.\nSet ghost to 5.\nShow 2.\n");
}

#[test]
fn err_parity_partial_output_before_error() {
    assert_outcome_parity("## Main\nShow \"a\".\nShow \"b\".\nLet x be 1 / 0.\nShow \"c\".\n");
}

#[test]
fn outcome_parity_policy_check_passes_and_fails() {
    // Predicate passes.
    assert_outcome_parity(
        "## Definition\n\
         A User has:\n\
         \x20   a role, which is Text.\n\
         \n\
         ## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         \n\
         ## Main\n\
         Let u be a new User with role \"admin\".\n\
         Check that the u is admin.\n\
         Show \"passed\".\n",
    );
    // Predicate fails: Security Check Failed + partial output preserved.
    assert_outcome_parity(
        "## Definition\n\
         A User has:\n\
         \x20   a role, which is Text.\n\
         \n\
         ## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         \n\
         ## Main\n\
         Show \"before\".\n\
         Let u be a new User with role \"guest\".\n\
         Check that the u is admin.\n\
         Show \"after\".\n",
    );
}

#[test]
fn outcome_parity_registry_struct_defaults() {
    // The struct definition lives in the discovery registry (## Definition),
    // not in a Stmt::StructDef — default-fill must still work in the VM.
    assert_outcome_parity(
        "## Definition\n\
         A Point has:\n\
         \x20   an x, which is an Int.\n\
         \x20   a y, which is an Int.\n\
         \n\
         ## Main\n\
         Let p be a new Point with x 3.\n\
         Show p's x.\n\
         Show p's y.\n",
    );
}

#[test]
fn outcome_parity_read_console_and_file_without_vfs() {
    // Console reads yield empty Text in both engines.
    assert_outcome_parity("## Main\nRead input from the console.\nShow input.\nShow 7.\n");
    // File reads without a VFS error identically (after the partial output).
    assert_outcome_parity(
        "## Main\nShow \"start\".\nRead data from file \"data.txt\".\nShow data.\n",
    );
}

#[test]
fn err_parity_division_in_loop_partial_output() {
    // The error fires mid-loop; both engines keep the iterations already shown.
    assert_outcome_parity(
        "## Main\n\
         Let mutable i be 3.\n\
         While i is at least 0:\n\
         \x20   Show 6 / i.\n\
         \x20   Set i to i - 1.\n",
    );
}

#[test]
fn outcome_parity_push_to_struct_field() {
    // Positive: push into a struct's List field through the shared allocation.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a items, which is a List of Int.\n\
         \n\
         ## Main\n\
         Let xs be [1].\n\
         Let b be a new Box with items xs.\n\
         Push 2 to b's items.\n\
         Show length of b's items.\n",
    );
    // Field is not a List.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a n, which is an Int.\n\
         \n\
         ## Main\n\
         Let b be a new Box with n 1.\n\
         Push 2 to b's n.\n\
         Show 9.\n",
    );
}

#[test]
fn outcome_parity_collection_stmt_shape_errors() {
    // Each collection statement's non-identifier error string is the spec.
    assert_outcome_parity(
        "## Definition\nA Box has:\n\x20   a m, which is a Map of Text to Int.\n\n## Main\nLet mm be a new Map of Text to Int.\nLet b be a new Box with m mm.\nSet item \"k\" of b's m to 5.\nShow 1.\n",
    );
    assert_outcome_parity(
        "## Definition\nA Box has:\n\x20   a s, which is a Set of Int.\n\n## Main\nLet ss be a new Set of Int.\nLet b be a new Box with s ss.\nAdd 5 to b's s.\nShow 1.\n",
    );
}

#[test]
fn outcome_parity_struct_field_set_via_index_syntax() {
    // `Set item "field" of structVar to v` — the index-syntax struct-field
    // write (the decompiler's CMapSet rendering), with VALUE semantics.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a x, which is an Int.\n\
         \n\
         ## Main\n\
         Let b be a new Box with x 1.\n\
         Set item \"x\" of b to 9.\n\
         Show item \"x\" of b.\n\
         Show b's x.\n",
    );
    // Through a global mutated inside a function (write-back required).
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a x, which is an Int.\n\
         \n\
         ## To poke:\n\
         \x20   Set item \"x\" of b to 7.\n\
         \n\
         ## Main\n\
         Let b be a new Box with x 1.\n\
         poke().\n\
         Show b's x.\n",
    );
}

// ── The file model ──────────────────────────────────────────────────────────────
//
// `Read … from file`, `Write … to file` and `Mount … at` are one model on both tiers: the
// tree-walker's `Interpreter::with_vfs(MemVfs)` and the VM's `Vfs*` builtins read and write the
// SAME in-memory filesystem — the sealed world `largo test` installs around a `## Test`
// (`logicaffeine_system::testworld`). Outside a sealed world that filesystem holds nothing, so a
// write lands nowhere and a read finds nothing, identically on both tiers.

/// `src` on `run` inside a fresh sealed world of its own (each tier writes into its own world, so
/// a file one tier wrote can never be what the other reads back).
fn sealed(run: fn(&str) -> RunOutcome, src: &str) -> RunOutcome {
    testworld::install(TestWorld::new(Capabilities::default(), 0));
    let out = run(src);
    testworld::uninstall();
    out
}

/// The differential contract inside a sealed world, pinned to the tree-walker's OWN outcome first:
/// a row is satisfied only by both tiers doing what the reference does, never by both failing
/// alike.
fn assert_sealed_parity(src: &str, output: &str, error: Option<&str>) {
    let tw = sealed(tw_outcome, src);
    assert_eq!(
        norm(&tw.output),
        norm(output),
        "the tree-walker's own output for:\n{src}\ntw: {tw:?}"
    );
    assert_eq!(
        tw.error.as_deref(),
        error,
        "the tree-walker's own error for:\n{src}\ntw: {tw:?}"
    );
    let vm = sealed(vm_outcome, src);
    assert_eq!(
        norm(&vm.output),
        norm(&tw.output),
        "partial output diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}"
    );
    assert_eq!(
        vm.error, tw.error,
        "error diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}"
    );
}

/// The VM's file statements finish where they run, whatever drives the VM: on the networking driver
/// (`vm_outcome_net`, the one a program with `Listen`/`Send` takes, run by an executor of its own) a write
/// and a read do exactly what the tree-walker does, in a sealed world and outside one, as on the plain driver.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn outcome_parity_files_on_the_networking_driver() {
    use logicaffeine_compile::compile::vm_outcome_net;
    let rows = [
        (
            "## Main\nShow \"start\".\nWrite \"hi\" to file \"a.txt\".\nRead x from file \"a.txt\".\nShow x.\n",
            "start\nhi",
            false,
        ),
        ("## Main\nShow \"start\".\nRead x from file \"missing.txt\".\nShow x.\n", "start", true),
    ];
    for (src, output, fails) in rows {
        let tw = sealed(tw_outcome, src);
        assert_eq!(
            norm(&tw.output),
            norm(output),
            "the tree-walker's own output for:\n{src}\ntw: {tw:?}"
        );
        assert_eq!(
            tw.error.is_some(),
            fails,
            "whether the tree-walker fails:\n{src}\ntw: {tw:?}"
        );
        for (driver, run) in [
            ("plain", vm_outcome as fn(&str) -> RunOutcome),
            ("networking", vm_outcome_net),
        ] {
            let vm = sealed(run, src);
            assert_eq!(
                (norm(&vm.output), vm.error.as_deref()),
                (norm(&tw.output), tw.error.as_deref()),
                "the VM's {driver} driver in a sealed world diverged for:\n{src}"
            );
        }
        let (net, plain) = (vm_outcome_net(src), vm_outcome(src));
        assert_eq!(
            (norm(&net.output), net.error),
            (norm(&plain.output), plain.error),
            "the two VM drivers outside a sealed world for:\n{src}"
        );
    }
}

#[test]
fn outcome_parity_file_write_then_read_back() {
    // The round trip: what `Write` put there is what `Read` hands back.
    assert_sealed_parity(
        "## Main\nWrite \"hi\" to file \"a.txt\".\nRead x from file \"a.txt\".\nShow x.\n",
        "hi",
        None,
    );
    // Content and path are displayed, not typed: a number writes as its text.
    assert_sealed_parity(
        "## Main\nWrite 42 to file \"n.txt\".\nRead x from file \"n.txt\".\nShow x.\n",
        "42",
        None,
    );
    // The path can come from a binding.
    assert_sealed_parity(
        "## Main\nLet p be \"v.txt\".\nWrite \"via\" to file p.\nRead x from file p.\nShow x.\n",
        "via",
        None,
    );
    // The program goes on after a write, and a write reports nothing of its own.
    assert_sealed_parity(
        "## Main\nShow \"before\".\nWrite \"hi\" to file \"a.txt\".\nShow \"after\".\n",
        "before\nafter",
        None,
    );
    // Outside a sealed world there is no filesystem at all: the write lands nowhere and the read
    // finds nothing — on both tiers, in the same words.
    assert_outcome_parity(
        "## Main\nWrite \"hi\" to file \"a.txt\".\nRead x from file \"a.txt\".\nShow x.\n",
    );
}

#[test]
fn outcome_parity_file_write_replaces_and_reads_the_last_content() {
    // A second `Write` to the same path replaces the first: the language has no file-append
    // statement (`Append … to …` is the sequence operation), so replacement IS the write model.
    assert_sealed_parity(
        "## Main\n\
         Write \"first\" to file \"a.txt\".\n\
         Write \"second\" to file \"a.txt\".\n\
         Read x from file \"a.txt\".\n\
         Show x.\n",
        "second",
        None,
    );
    // Two paths are two files.
    assert_sealed_parity(
        "## Main\n\
         Write \"one\" to file \"1.txt\".\n\
         Write \"two\" to file \"2.txt\".\n\
         Read a from file \"1.txt\".\n\
         Read b from file \"2.txt\".\n\
         Show a.\n\
         Show b.\n",
        "one\ntwo",
        None,
    );
}

#[test]
fn outcome_parity_file_write_then_read_a_missing_file() {
    // The write succeeds; the read of ANOTHER path fails in the tree-walker's words, after the
    // partial output.
    assert_sealed_parity(
        "## Main\nShow \"start\".\nWrite \"hi\" to file \"a.txt\".\nRead y from file \"b.txt\".\nShow y.\n",
        "start",
        Some("Read error: Not found: b.txt"),
    );
    // And the same program outside a sealed world.
    assert_outcome_parity(
        "## Main\nShow \"start\".\nWrite \"hi\" to file \"a.txt\".\nRead y from file \"b.txt\".\nShow y.\n",
    );
}

#[test]
fn outcome_parity_mount_reads_the_journal_or_starts_empty() {
    // `Mount c at "…"` binds `c` to the file's text when it exists …
    assert_sealed_parity(
        "## Main\nWrite \"7\" to file \"c.journal\".\nMount c at \"c.journal\".\nShow c.\n",
        "7",
        None,
    );
    // … and to empty Text when it does not — no error, the program goes on.
    assert_sealed_parity(
        "## Main\nMount c at \"missing.journal\".\nShow length of c.\nShow \"after\".\n",
        "0\nafter",
        None,
    );
    // The path can come from a binding.
    assert_sealed_parity(
        "## Main\nLet p be \"e.journal\".\nWrite \"e\" to file p.\nMount c at p.\nShow c.\n",
        "e",
        None,
    );
    // The `Let … be mounted at …` spelling is the same statement.
    assert_sealed_parity(
        "## Main\n\
         Write \"9\" to file \"d.journal\".\n\
         Let d: Persistent Text be mounted at \"d.journal\".\n\
         Show d.\n",
        "9",
        None,
    );
    // Outside a sealed world a mount starts empty on both tiers.
    assert_outcome_parity("## Main\nMount c at \"c.journal\".\nShow length of c.\n");
}

#[test]
fn vm_file_statements_never_name_the_interpreter_api() {
    // Whatever a file statement answers on the VM, it is never the tree-walker's constructor: an
    // author reads what the program did, not how an engine was built.
    for src in [
        "## Main\nWrite \"hi\" to file \"a.txt\".\n",
        "## Main\nRead x from file \"a.txt\".\n",
        "## Main\nMount c at \"c.journal\".\n",
    ] {
        for out in [vm_outcome(src), sealed(vm_outcome, src)] {
            if let Some(error) = &out.error {
                assert!(
                    !error.contains("Interpreter::with_vfs")
                        && !error.contains("VFS not initialized"),
                    "the VM leaked an engine API for:\n{src}\n{error}"
                );
            }
        }
    }
}
