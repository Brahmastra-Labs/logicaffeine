//! The Rust heap-sort lowering keeps its guarded affine indices in `i64`.
//! Direct C must consume the same proof instead of executing every sift step
//! through the exact tagged-integer runtime.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_traced, vm_outcome};
use logicaffeine_compile::optimization::Opt;

fn heap_source(n: usize) -> String {
    include_str!("../../../benchmarks/programs/heap_sort/main.lg").replace(
        "Let arguments be args().\nLet n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

const EXACT_FALLBACK: &str = r#"
## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
    Let mutable result be arr.
    Let mutable root be start.
    While 2 * root + 1 is at most end:
        Let child be 2 * root + 1.
        Let mutable swapIdx be root.
        If item (swapIdx + 1) of result is less than item (child + 1) of result:
            Set swapIdx to child.
        If child + 1 is at most end:
            If item (swapIdx + 1) of result is less than item (child + 2) of result:
                Set swapIdx to child + 1.
        If swapIdx equals root:
            Return result.
        Let tmp be item (root + 1) of result.
        Set item (root + 1) of result to item (swapIdx + 1) of result.
        Set item (swapIdx + 1) of result to tmp.
        Set root to swapIdx.
    Return result.

## Main
Let xs be a new Seq of Int.
Let huge be 2 ** 100.
Let ys be siftDown(xs, huge, 0).
Show length of ys.
"#;

fn c_output(source: &str) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_heap_word_")
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
fn direct_c_heap_sift_consumes_the_shared_word_proof() {
    let source = include_str!("../../../benchmarks/programs/heap_sort/main.lg");
    let (c, trace) = compile_to_c_traced(source).expect("heap sort compiles to traced direct C");
    let marker = c
        .find("/* heap-sift indices: guarded native-word fast path */")
        .expect("direct C emitted no guarded heap-sift fast path");
    let fast_tail = &c[marker..];
    let fallback = fast_tail
        .find("\n    } else {\n")
        .expect("the guarded word path has no exact fallback");
    let fast = &fast_tail[..fallback];
    assert!(
        fast.contains("int64_t") && fast.contains("seq_i64_get"),
        "the fast path did not use native index locals:\n{fast}"
    );
    assert!(
        !fast.contains("logos_lint_"),
        "tagged exact arithmetic leaked back into the hot sift path:\n{fast}"
    );
    assert!(
        trace.lowering.fired(Opt::Peephole),
        "the heap-sift representation lowering disappeared from the C trace: {:?}",
        trace.lowering.keywords()
    );
}

#[test]
fn guarded_word_path_and_bigint_fallback_match_the_vm() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [
        ("heap", heap_source(128)),
        ("exact-fallback", EXACT_FALLBACK.to_string()),
    ] {
        let vm = vm_outcome(&source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(&source),
            vm.output,
            "{name}: direct C diverged from the VM"
        );
    }
}
