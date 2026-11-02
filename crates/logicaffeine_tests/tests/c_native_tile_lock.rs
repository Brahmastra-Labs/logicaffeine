//! Direct C must consume the optimizer's tile-order proof instead of silently
//! compiling the proven fast clone back into the authored scalar loop.  The
//! emitted fast path uses native counters; its exact fallback preserves LOGOS
//! semantics when the outer binding is not the zero machine word the tiler
//! requires.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_traced, vm_outcome};
use logicaffeine_compile::optimization::Opt;

const DYNAMIC_MATRIX: &str = include_str!("../../../benchmarks/programs/matrix_mult/main.lg");

const MATRIX: &str = r#"
## Main
Let n be 7.
Let a be a new Seq of Int.
Let b be a new Seq of Int.
Let c be a new Seq of Int.
Let seed be 42.
Let i be 0.
While i is less than n * n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 100 to a.
    Set i to i + 1.
Set i to 0.
While i is less than n * n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 100 to b.
    Push 0 to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let k be 0.
    While k is less than n:
        Let j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item idx of c to (item idx of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let checksum be 0.
Set i to 1.
While i is at most n * n:
    Set checksum to (checksum * 31 + item i of c) & 268435455.
    Set i to i + 1.
Show checksum.
"#;

const NONZERO_OUTER: &str = r#"
## Main
Let n be 5.
Let total be 0.
Let i be 1.
While i is less than n:
    Let k be 0.
    While k is less than n:
        Let j be 0.
        While j is less than n:
            Set total to total + i * 100 + k * 10 + j.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Show total.
Show i.
"#;

fn c_output(source: &str) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_native_tile_")
        .tempdir()
        .expect("temporary C directory");
    let c_path = dir.path().join("main.c");
    let bin_path = dir.path().join("main");
    std::fs::write(&c_path, c).expect("write generated C");
    let build = common::build_c_release(&c_path, &bin_path).expect("C compiler starts");
    assert!(
        build.status.success(),
        "generated C builds:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("generated C starts");
    assert!(
        run.status.success(),
        "generated C exits cleanly:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn direct_c_consumes_the_shared_tile_proof() {
    let (c, trace) =
        compile_to_c_traced(DYNAMIC_MATRIX).expect("matrix compiles to traced direct C");
    assert!(
        trace.optimizer.fired(Opt::Tile),
        "the shared optimizer did not prove the matrix nest tile-safe: {:?}",
        trace.optimizer.keywords()
    );
    assert!(
        trace.lowering.fired(Opt::Tile),
        "direct C silently ignored the optimizer's TileLegal fact: {:?}",
        trace.lowering.keywords()
    );
    assert!(
        trace.lowering.fired(Opt::Unchecked),
        "direct C silently ignored the matrix clone's proven-in-bounds accesses: {:?}",
        trace.lowering.keywords()
    );
    assert!(
        c.contains("shared TileLegal: 32x32x32 native C tile"),
        "the C tile marker disappeared even though Tile fired"
    );
}

#[test]
fn tiled_and_nonzero_fallback_paths_match_the_vm() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [("matrix", MATRIX), ("nonzero-outer", NONZERO_OUTER)] {
        let vm = vm_outcome(source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(source),
            vm.output,
            "{name}: direct C diverged from the exact VM"
        );
    }
}
