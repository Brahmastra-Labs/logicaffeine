//! The offload seam of the bytecode→C walker: a function the offload table names asks its device first, at its
//! entry, through `logos_dev_call(region, args, &out)`. A DONE answer is the call's result; a DECLINED one — or a
//! parameter that cannot cross its port this time — runs the body the walker emits anyway. The accelerator backends
//! build on it; an empty table leaves the walker's C as it is.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c_via_bytecode, vm_outcome};

fn have_gcc() -> bool {
    common::require_tool("gcc")
}

/// The standard output of the C translation unit `c`, built by gcc and run in a directory of its own.
fn run_c(c: &str, label: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "logos_seam_{}_{}_{}",
        std::process::id(),
        label,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a build directory");
    std::fs::write(dir.join("prog.c"), c).expect("the C source");
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o", "prog", "prog.c", "-lm"])
        .current_dir(&dir)
        .output()
        .expect("gcc");
    assert!(
        build.status.success(),
        "[{label}] gcc: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(dir.join("prog"))
        .output()
        .expect("the program runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        run.status.success(),
        "[{label}] the program failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// What the VM shows for `src`.
fn vm_shows(src: &str) -> String {
    let r = vm_outcome(src);
    assert_eq!(r.error, None, "the VM runs {src:?}");
    r.output
}

/// The device the seam lock runs against, appended to the host translation unit: region 7 answers every call with
/// 100 × its first argument, region 9 with its Real argument plus one, region 10 with 7; every other region declines.
const SEAM_DRIVER: &str = "
int logos_dev_call(int region, const int64_t *args, int64_t *out) {
    double d;
    switch (region) {
    case 7: *out = args[0] * 100; return 1;
    case 9: memcpy(&d, &args[0], 8); d += 1.0; memcpy(out, &d, 8); return 1;
    case 10: *out = 7; return 1;
    default: return 0;
    }
}
";

/// The walker's C for `src` with the functions `table` names offloaded, as `(function, region, parameter ports,
/// result port)`.
fn offloaded(
    src: &str,
    table: &[(
        &str,
        u32,
        Vec<logicaffeine_compile::vm::SlotKind>,
        logicaffeine_compile::vm::SlotKind,
    )],
) -> logicaffeine_compile::codegen_c::from_program::OffloadedC {
    use logicaffeine_compile::codegen_c::from_program::Offload;
    logicaffeine_compile::compile::compile_to_c_via_bytecode_offloaded(src, |prog, interner| {
        table
            .iter()
            .map(|(name, region, params, ret)| Offload {
                func: prog
                    .functions
                    .iter()
                    .position(|f| interner.resolve(f.name) == *name)
                    .unwrap_or_else(|| panic!("the program has no function `{name}`")),
                region: *region,
                params: params.clone(),
                ret: *ret,
            })
            .collect()
    })
    .unwrap_or_else(|e| panic!("the walker refused {src:?}: {e:?}"))
}

/// A function the offload table names asks its device first, at its entry, for every caller: the device's answer is
/// the call's result when it gives one; when it declines, or a parameter cannot cross its port this time (an exact
/// integer past i64), the host runs the same body. A port the settled kind cannot use keeps the function on the host,
/// reported with both kinds. An empty table is the walker itself.
#[test]
fn the_walker_asks_an_offloaded_functions_device_first() {
    use logicaffeine_compile::vm::SlotKind;
    if !have_gcc() {
        return;
    }
    let run = |c: String, label: &str| run_c(&(c + SEAM_DRIVER), label);

    // `randomInt(k, k)` is `k` on every tier and opaque to the optimizer, so the calls stay calls.
    let triple = "## To triple (n: Int) -> Int:\n    Return n * 3.\n\n## Main\n    Let a be randomInt(5, 5).\n    Show triple(a).\n    Show triple(a + 1).\n";
    let answered = offloaded(triple, &[("triple", 7, vec![SlotKind::Int], SlotKind::Int)]);
    assert_eq!(
        run(answered.c.clone(), "seam_answered"),
        "500\n600\n",
        "the device's answer is the call's result"
    );
    assert!(
        answered.kept.is_empty(),
        "nothing kept on the host: {:?}",
        answered.kept
    );
    assert_eq!(
        answered.c.matches("logos_dev_call(").count(),
        2,
        "one declaration and one dispatch"
    );

    let declined = offloaded(triple, &[("triple", 8, vec![SlotKind::Int], SlotKind::Int)]);
    assert_eq!(
        run(declined.c, "seam_declined"),
        "15\n18\n",
        "a declined call runs the host body"
    );

    let half = "## To half (x: Real) -> Real:\n    Return x / 2.0.\n\n## Main\n    Let a be randomInt(3, 3).\n    Show half(a + 0.5).\n";
    let real = offloaded(half, &[("half", 9, vec![SlotKind::Float], SlotKind::Float)]);
    assert_eq!(
        run(real.c, "seam_real"),
        vm_shows("## Main\n    Show 4.5.\n"),
        "a Real crosses as its bits"
    );

    let bump = "## To bump (n: Int) -> Int:\n    Return n + 1.\n\n## Main\n    Let mutable p be randomInt(1, 1).\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show bump(p).\n    Show bump(p - p + 5).\n";
    let exact = offloaded(bump, &[("bump", 10, vec![SlotKind::Int], SlotKind::Int)]);
    assert_eq!(
        run(exact.c, "seam_exact"),
        format!("{}7\n", vm_shows("## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show p + 1.\n")),
        "an exact integer past i64 stays on the host; one within it crosses"
    );

    let measure = "## To measure (t: Text) -> Int:\n    Return length of t.\n\n## Main\n    Let a be randomInt(1, 1).\n    Show measure(\"hello\" + a).\n";
    let kept = offloaded(
        measure,
        &[("measure", 7, vec![SlotKind::Int], SlotKind::Int)],
    );
    assert_eq!(
        kept.kept.len(),
        1,
        "a Text parameter cannot cross an Int port"
    );
    assert!(
        kept.kept[0].1.contains("parameter 0")
            && kept.kept[0].1.contains("Text")
            && kept.kept[0].1.contains("Int"),
        "the report names the parameter and both kinds: {:?}",
        kept.kept
    );
    assert_eq!(
        kept.c.matches("logos_dev_call(").count(),
        1,
        "the declaration and no dispatch"
    );
    assert_eq!(
        run(kept.c, "seam_kept"),
        "6\n",
        "the host runs the function the port cannot serve"
    );

    for src in [triple, half, bump, measure] {
        let empty =
            logicaffeine_compile::compile::compile_to_c_via_bytecode_offloaded(src, |_, _| {
                Vec::new()
            })
            .expect("lowers");
        assert!(
            !empty.c.contains("logos_dev_call"),
            "an empty table leaves no trace of the seam"
        );
        assert_eq!(
            empty.c,
            compile_to_c_via_bytecode(src).expect("lowers"),
            "an empty table is the walker itself"
        );
        assert!(empty.kept.is_empty());
    }
}
