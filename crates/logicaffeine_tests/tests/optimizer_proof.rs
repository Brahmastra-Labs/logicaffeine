//! The optimizer, proven — same program, optimization ON vs OFF.
//!
//! Ray Myers' central point was **"I did not see evidence of a useful optimizing
//! compiler."** The honest answer isn't a cross-language benchmark (those can be
//! unfair in either direction — we found our own `page_allocator` strawman). It's
//! this: take ONE LOGOS program, compile it with the optimizer on (default) and off
//! (`## No <opt>`), and show the generated Rust structurally change. The only variable
//! is the compiler. No port to strawman. These lock that evidence so it can't regress.
//!
//! Each demo runs through the real `largo`/`compile_project` path — the same path a
//! user hits — and relies on file-level `## No <opt>` being program-wide (fixed in the
//! parser: a `## No` in the file header applies to the whole program).

#![cfg(not(target_arch = "wasm32"))]

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::compile::compile_project;

/// Compile a single-file program through the real path; return the generated Rust.
fn emit(src: &str) -> String {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, src).unwrap();
    compile_project(&main)
        .unwrap_or_else(|e| panic!("program should compile: {e:?}"))
        .rust_code
}

/// Insert a program-wide `## No <opt>` immediately before `## Main` — the documented spot
/// for a file-level decorator (a `## No` attached to a function scopes to that function).
fn without(opt: &str, src: &str) -> String {
    src.replace("## Main", &format!("## No {opt}\n## Main"))
}

/// A map filled at the dense keys `1..=n` and read back at the same keys: the shape the dense-map collapse proves.
const DENSE_MAP: &str = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let m be a new Map of Int to Int with capacity n.
Let i be 1.
While i is less than n + 1:
    Set item i of m to i * 2.
    Set i to i + 1.
Let found be 0.
Set i to 1.
While i is less than n + 1:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found.
";
const QUICKSORT: &str = include_str!("../../../benchmarks/programs/quicksort/main.lg");

/// **Dense-map collapse** — a general optimization C/Zig don't do: the compiler proves
/// a `Map<Int,Int>`'s keys are dense and lowers it to a direct-addressed array. Turn it
/// off and the same program keeps a real hash map. This is a *structural* change to the
/// data representation, driven purely by the optimizer.
#[test]
fn densemap_collapses_hashmap_to_direct_array() {
    let on = emit(DENSE_MAP);
    let off = emit(&without("densemap", DENSE_MAP));
    assert!(
        on.contains("LogosDense"),
        "default: the dense-map collapse should fire (hash map → direct-addressed array):\n{on}"
    );
    assert!(
        !off.contains("LogosDense"),
        "`## No densemap`: the same program must keep a real hash map, no dense-array collapse:\n{off}"
    );
}

/// **Bounds-check elimination** — the abstract-interpretation oracle proves the sort's
/// indices are in range and emits `get_unchecked`; turn it off and the same program
/// pays a bounds check on every element access. Pure optimizer, same source.
#[test]
fn unchecked_elides_bounds_checks_the_oracle_proved_safe() {
    let on = emit(QUICKSORT);
    let off = emit(&without("unchecked", QUICKSORT));
    let on_n = on.matches("get_unchecked").count();
    let off_n = off.matches("get_unchecked").count();
    assert!(
        on_n > 0,
        "default: the oracle should prove indices in range and emit `get_unchecked` ({on_n} sites):\n{on}"
    );
    assert!(
        off_n == 0,
        "`## No unchecked`: every access must be bounds-checked again (found {off_n} `get_unchecked`):\n{off}"
    );
}

/// The two above are *structural* deltas; this guards that turning an optimization off
/// is not a no-op in general — the on/off generated Rust must actually differ for a
/// program the optimizer works on. (If some future refactor made `## No` a silent
/// no-op again — the exact bug we just fixed — this fails.)
#[test]
fn optimizer_toggle_is_observable_not_a_noop() {
    let on = emit(DENSE_MAP);
    let off = emit(&without("densemap", DENSE_MAP));
    assert_ne!(
        on, off,
        "optimizer on vs off must produce different Rust for a program it optimizes — \
         a `## No` that changes nothing means the toggle is broken"
    );
}

/// **Leaf-outlining** — a self-recursive `&mut [T]` in-place function (quicksort's
/// `qs`) keeps its hot partition loop embedded in the recursion, where LLVM
/// optimizes it worse than a hand-written leaf `partition` (measured ~1.3× on
/// this exact program). The compiler extracts the non-recursive work region into
/// an `#[inline(never)]` leaf helper — rendered through the ordinary codegen path
/// (real `codegen_stmt`, not a text rewrite), so bounds-check elimination and the
/// overflow de-guard carry over. This pins that the leaf is emitted, the wrapper
/// calls it, and the hot loop and final swap moved into it.
#[test]
fn recursive_partition_is_outlined_into_a_leaf_helper() {
    let rust = emit(QUICKSORT);
    assert!(
        rust.contains("fn qs__part(") && rust.contains("#[inline(never)]"),
        "the partition should be outlined into an `#[inline(never)]` leaf `qs__part`:\n{rust}"
    );
    assert!(
        rust.contains("qs__part("),
        "the recursive wrapper must call the outlined leaf:\n{rust}"
    );
    // The hot loop and the final swap live in the leaf, not the wrapper.
    let helper_start = rust.find("fn qs__part(").unwrap();
    let helper = &rust[helper_start..];
    let helper_end = helper.find("\nfn ").unwrap_or(helper.len());
    let helper = &helper[..helper_end];
    assert!(
        helper.contains("for j in"),
        "the hot loop must be in the leaf:\n{helper}"
    );
    // The final (pivot) swap is in the leaf AND bounds-eliminated: the oracle proves
    // `1 <= i <= hi <= len` past the scan (the loop-exit induction bound), so the
    // pivot-slot store `item hi of arr` lowers to `get_unchecked_mut` on `hi - 1`
    // rather than a checked `__swap_tmp` (the whole swap is bounds-branch-free).
    assert!(
        helper.contains("get_unchecked_mut((hi - 1) as usize)"),
        "the final (pivot) swap must be in the leaf, bounds-eliminated to a \
         `get_unchecked_mut` on the pivot slot `hi`:\n{helper}"
    );
    // The counter increment is de-guarded (sound: a get_unchecked'd wide-slice
    // index cannot overflow on `+1`) — no checked `logos_add_i64` in the leaf loop.
    assert!(
        !helper.contains("logos_add_i64"),
        "the counter increment must be de-guarded to plain `+ 1` in the leaf:\n{helper}"
    );
}

/// The outline must be observable-off: `## No borrow` (which removes the `&mut [T]`
/// lowering the outline rides on) leaves the function inlined, and the default
/// path outlines — a real toggle, not a silent no-op.
#[test]
fn outlining_rides_on_the_borrow_lowering() {
    let on = emit(QUICKSORT);
    let off = emit(&without("borrow", QUICKSORT));
    assert!(
        on.contains("fn qs__part("),
        "default: quicksort should outline:\n{on}"
    );
    assert!(
        !off.contains("fn qs__part("),
        "`## No borrow`: without the `&mut [T]` lowering there is no leaf to outline:\n{off}"
    );
}
