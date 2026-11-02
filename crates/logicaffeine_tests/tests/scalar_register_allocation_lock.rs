//! scalar_register_allocation_lock — the scalar encoder maps the VM's registers onto the routable core's
//! sixteen by liveness, so a program runs on the chip whenever no more than sixteen of its values are
//! live at once, not only when the VM happened to number every register below sixteen.
//!
//! WHY. The VM compiler gives each `Let` a fresh register and places a named value above the
//! temporaries that computed it; it has registers to spare. The HFT decision inlined into a stimulus
//! loop holds about a dozen values live at once and still numbers its registers past fifty, so the
//! encoder refused it for `r16` although the core had room.
//!
//! What this file locks:
//!   1. a program that already fits keeps its exact encoding — the allocator is the identity there, so
//!      every image already streamed to a board stays byte-for-byte what it was;
//!   2. a program numbered past fifteen but never holding more than sixteen live values encodes and runs
//!      on the soft-core model exactly as on the VM — across straight-line code, branches and loops;
//!   3. a program that holds seventeen values live at once is refused, naming why;
//!   4. allocation is deterministic.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::decision_workload::soft_core_realbist_source;
use logicaffeine_compile::codegen_sva::scalar_asm::{
    allocate_scalar_registers, compile_to_scalar_words,
};
use logicaffeine_compile::codegen_sva::soft_core_model::run_soft_core;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::vm::{Compiler, Op};

fn compiled(src: &str) -> Vec<Op> {
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.map_err(|a| format!("{a}"))?;
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).map(|p| p.code)
    })
    .unwrap_or_else(|e| panic!("does not compile: {e}"))
}

/// The highest register an op names (its register fields only — never a constant index or jump target).
fn max_register(code: &[Op]) -> u64 {
    const REG_FIELDS: [&str; 9] = [
        "dst", "lhs", "rhs", "src", "cond", "place", "delta", "a", "b",
    ];
    code.iter()
        .flat_map(|op| {
            let d = format!("{op:?}");
            let fields = d
                .split_once('{')
                .map(|(_, f)| f.trim_end_matches('}').to_string())
                .unwrap_or_default();
            fields
                .split(',')
                .filter_map(|kv| kv.split_once(':'))
                .filter(|(k, _)| REG_FIELDS.contains(&k.trim()))
                .filter_map(|(_, v)| v.trim().parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .max()
        .unwrap_or(0)
}

fn on_chip_equals_vm(name: &str, src: &str) {
    let words = compile_to_scalar_words(src).unwrap_or_else(|e| panic!("{name}: {e}"));
    let chip: Vec<String> = run_soft_core(&words, 5_000_000)
        .iter()
        .map(|v| v.to_string())
        .collect();
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{name}: VM error {:?}", vm.error);
    assert_eq!(
        chip.join(" "),
        vm.output.split_whitespace().collect::<Vec<_>>().join(" "),
        "{name}: chip != VM"
    );
}

/// `n` values defined one after another, all read by a final sum: `n` live at once at the end.
fn all_live(n: usize) -> String {
    let lets: String = (0..n)
        .map(|k| format!("    Let v{k} be {} * 3 + 1.\n", k + 2))
        .collect();
    let sum = (0..n)
        .map(|k| format!("v{k}"))
        .collect::<Vec<_>>()
        .join(" + ");
    format!("## Main\n{lets}    Show {sum}.\n")
}

/// `n` values each dead once the next is made: never more than a few live, however many are numbered.
/// Masked to 15 bits (a mask the 16-bit immediate can load) so the 32-bit core and the VM's unbounded integers agree.
fn chained(n: usize) -> String {
    let mut s = String::from("## Main\n    Let v0 be 5.\n");
    for k in 1..n {
        s.push_str(&format!(
            "    Let v{k} be (v{} * 3 + {k}) & 32767.\n",
            k - 1
        ));
    }
    s.push_str(&format!("    Show v{}.\n", n - 1));
    s
}

#[test]
fn a_program_that_fits_keeps_its_exact_encoding() {
    for src in [
        "## Main\n    Show 2 + 3 * 4 - 1.\n",
        "## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most 5:\n        Set sum to sum + i.\n        Set i to i + 1.\n    Show sum.\n",
        "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    ] {
        let code = compiled(src);
        assert_eq!(format!("{:?}", allocate_scalar_registers(&code).expect("fits")), format!("{code:?}"),
            "a fitting program must not be renumbered");
    }
}

#[test]
fn a_program_numbered_past_the_register_file_runs_identically_once_allocated() {
    let long = chained(30);
    assert!(
        max_register(&compiled(&long)) > 15,
        "the fixture must number past r15"
    );
    on_chip_equals_vm("chained-30", &long);
    on_chip_equals_vm("all-live-16", &all_live(15));
    on_chip_equals_vm("loop-carried", "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Let mutable c be 3.\n    Let mutable i be 0.\n    While i is less than 20:\n        Let t1 be a + b.\n        Let t2 be t1 * c.\n        Let t3 be t2 - a.\n        Let t4 be t3 + i.\n        Set a to b.\n        Set b to c.\n        Set c to t4 & 1023.\n        If c is greater than 500:\n            Let t5 be c - 500.\n            Let t6 be t5 * 2.\n            Set c to t6 & 511.\n        Show c.\n        Set i to i + 1.\n");
    on_chip_equals_vm("hft-decision-loop", &soft_core_realbist_source(200));
}

#[test]
fn seventeen_values_live_at_once_are_refused_with_the_reason() {
    let err = compile_to_scalar_words(&all_live(17))
        .expect_err("seventeen live values cannot share sixteen registers");
    assert!(
        err.contains("live"),
        "the refusal must say it is liveness, not numbering: {err}"
    );
}

#[test]
fn allocation_is_deterministic() {
    let code = compiled(&soft_core_realbist_source(30000));
    let a = allocate_scalar_registers(&code).expect("the HFT loop fits sixteen registers");
    assert_eq!(
        format!("{a:?}"),
        format!("{:?}", allocate_scalar_registers(&code).unwrap())
    );
    assert!(
        max_register(&code) > 15 && max_register(&a) <= 15,
        "numbered past fifteen, allocated below sixteen"
    );
}
