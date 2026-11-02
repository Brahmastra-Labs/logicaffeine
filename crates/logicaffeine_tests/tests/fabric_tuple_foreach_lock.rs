//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FOREACH-OVER-A-HEAP-LIST ON THE LOOP FABRIC — the pit-of-success ratchet for iterating a LOCAL heap
//! collection (`Repeat for (a, b) in ps`) with the STATE PLANE threaded through the loop body.
//!
//! This is the Stage-X2 lift: the combinational loop fabric gains a HEAP PLANE. A list-of-tuples can only
//! be a LOCAL heap value (`Seq of (Int, Int)` is not a parameter type), so the fabric must (1) build the
//! list in a prologue heap island (`NewTuple`/`NewList` → `HeapAlloc`/`HeapWrite`), (2) iterate it —
//! element `iv` = `heap[list_ptr + 1 + iv]` — and (3) run the body with the heap threaded, so
//! `DestructureTuple` (`HeapRead(elem + 1 + j)`) and any collection op inside the loop evaluate against a
//! real heap. Each kernel MUST lower and evaluate ≡ VM. COVERAGE RATCHET: the kernel set only grows.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_loop_fabric, loop_fabric_from_source};
use logicaffeine_compile::compile::vm_outcome;

/// A foreach kernel: `key`, the function body (a `Repeat for (…) in <local list>` loop), and its VM args.
struct Kernel {
    key: &'static str,
    src: &'static str,
}

const KERNELS: &[Kernel] = &[
    // Sum of products over a local list of pairs — the canonical DestructureTuple foreach.
    Kernel {
        key: "sumprod",
        src: "## To f -> Int:\n    Let mutable s be 0.\n    Let ps be [(1, 2), (3, 4), (5, 6)].\n    Repeat for (a, b) in ps:\n        Set s to s + a * b.\n    Return s.\n",
    },
    // Sum of the FIRST components only (b unused) — the destructure must still bind both.
    Kernel {
        key: "sumfirst",
        src: "## To f -> Int:\n    Let mutable s be 0.\n    Let ps be [(10, 2), (30, 4), (50, 6)].\n    Repeat for (a, b) in ps:\n        Set s to s + a.\n    Return s.\n",
    },
    // A DATA-DEPENDENT BRANCH inside the heap-foreach (if-conversion over the state plane): max of seconds.
    Kernel {
        key: "maxsnd",
        src: "## To f -> Int:\n    Let mutable m be 0.\n    Let ps be [(1, 20), (3, 5), (9, 40), (2, 7)].\n    Repeat for (a, b) in ps:\n        If b is greater than m:\n            Set m to b.\n    Return m.\n",
    },
    // A 3-TUPLE destructure — the arity generalizes past pairs (a*b + c).
    Kernel {
        key: "triple",
        src: "## To f -> Int:\n    Let mutable s be 0.\n    Let ps be [(1, 2, 3), (4, 5, 6)].\n    Repeat for (a, b, c) in ps:\n        Set s to s + a * b + c.\n    Return s.\n",
    },
    // A SCALAR local list (no tuple) — the same heap-list iteration, element read directly (no destructure).
    Kernel {
        key: "scalars",
        src: "## To f -> Int:\n    Let mutable s be 0.\n    Let xs be [4, 8, 15, 16].\n    Repeat for x in xs:\n        Set s to s + x.\n    Return s.\n",
    },
    // COMPUTED (non-literal) tuple values — the prologue heap-build lowers dynamic element values, not just
    // constants: `n` feeds every field, so the tuples carry `(5,6)` and `(10,3)`.
    Kernel {
        key: "computed",
        src: "## To f -> Int:\n    Let mutable s be 0.\n    Let n be 5.\n    Let ps be [(n, n + 1), (n + n, n - 2)].\n    Repeat for (a, b) in ps:\n        Set s to s + a * b.\n    Return s.\n",
    },
];

fn vm_int(src: &str) -> String {
    let full = format!("{src}## Main\n    Show f().\n");
    let out = vm_outcome(&full);
    assert!(out.error.is_none(), "VM error {:?} for:\n{src}", out.error);
    out.output.trim().to_string()
}

#[test]
fn foreach_over_a_heap_list_lowers_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {}));

    let declared: BTreeSet<&str> = KERNELS.iter().map(|k| k.key).collect();
    let mut realized: BTreeSet<&str> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

    for k in KERNELS {
        let vm = vm_int(k.src);
        let lowered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let lf = loop_fabric_from_source(k.src, "f");
            eval_loop_fabric(&lf, &[])
        }));
        match lowered {
            Ok(got) => {
                if got.to_string() == vm {
                    realized.insert(k.key);
                } else {
                    failures.push(format!("{}: fabric {got} != VM {vm}", k.key));
                }
            }
            Err(e) => {
                let m = e
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("panic");
                failures.push(format!(
                    "{}: did not lower: {}",
                    k.key,
                    &m[..m.len().min(90)]
                ));
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("=== tuple-foreach failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "foreach-over-heap-list: {} kernel(s) failed (see stderr)",
        failures.len()
    );
    assert_eq!(
        realized,
        declared,
        "tuple-foreach coverage gap: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "tuple-foreach: {} kernels iterate a LOCAL heap list on the loop fabric ≡ VM",
        declared.len()
    );
}
