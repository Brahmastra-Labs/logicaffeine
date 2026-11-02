//! Regions: a scalar function of the optimized program, lowered to a control-flow graph of checked cells,
//! computes exactly what the VM computes when it calls that function.
//!
//! For each curated function and each argument vector, the VM runs `Show f(args).` and the region runs the
//! same call:
//!
//! - a region that returns a value returns the VM's value, printed the VM's way;
//! - a region that hands the call back names a divergence the VM's own run shows. A zero divisor or a refused
//!   exponent is a VM failure with that message. An `i64` overflow is an op the kernel promotes to a
//!   `BigInt` at the operands the region stopped on;
//! - every function also completes on the device model, each cell its lane routine at a device's shape,
//!   with the same value or the same hand-back.
//!
//! A function that does anything but compute a scalar from scalars stays on the host, and its reason names
//! what it does.

#[path = "accel_common/mod.rs"]
mod accel_common;

use accel_common::{cases, judge_call, region_of};
use logicaffeine_compile::accel::lane::{ExactLanes, LaneShape};
use logicaffeine_compile::accel::region::{discover, Discovered};
use logicaffeine_compile::compile::compile_to_program_optimized_named;

const FUEL: u64 = 10_000_000;

#[test]
fn every_curated_region_is_the_vms_function() {
    let mut bad = Vec::new();
    for case in cases() {
        let region = region_of(&case);
        let mut completed = 0;
        for args in &case.calls {
            let words: Vec<i64> = args.iter().map(|a| a.word()).collect();
            let got = region.eval_checked(&words, FUEL);
            if got.is_ok() {
                completed += 1;
            }
            if let Some(why) = judge_call(case.src, case.func, region.ret, args, &got) {
                bad.push(why);
            }
        }
        if completed == 0 {
            bad.push(format!("{}: no call completed on the region", case.func));
        }
    }
    assert!(bad.is_empty(), "{} region calls differ from the VM:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn every_curated_region_runs_the_same_on_device_lanes() {
    let mut bad = Vec::new();
    for case in cases() {
        let region = region_of(&case);
        for shape in [LaneShape::new(32, 16), LaneShape::new(32, 8), LaneShape::new(8, 4)] {
            for args in &case.calls {
                let words: Vec<i64> = args.iter().map(|a| a.word()).collect();
                let want = region.eval_checked(&words, FUEL);
                let mut lanes = ExactLanes { shape };
                let got = region.eval_on_lanes(&mut lanes, &words, FUEL).map(|(v, _)| v);
                if got != want {
                    bad.push(format!("{shape:?} {}({args:?}): lanes {got:?}, checked {want:?}", case.func));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} device-lane runs differ from the checked region:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn a_function_that_is_not_scalar_computation_stays_on_the_host_by_name() {
    let host = [
        ("## To noisy (x: Int) -> Int:\n    Show x.\n    Return x.\n\n## Main\n    Show noisy(3).\n", "noisy", "Show"),
        ("## To greet (n: Int) -> Text:\n    Return \"hi\".\n\n## Main\n    Show greet(3).\n", "greet", "Text"),
        ("## To first (xs: Seq of Int) -> Int:\n    Return item 1 of xs.\n\n## Main\n    Show first([4, 5]).\n", "first", "parameter 1"),
        (
            "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\n    Show fact(5).\n",
            "fact",
            "Call",
        ),
        ("## To cmpmix (n: Int, f: Float) -> Bool:\n    Return n is less than f.\n\n## Main\n    Show cmpmix(3, 2.5).\n", "cmpmix", "ordering of Int and Float"),
    ];
    for (src, func, needle) in host {
        let (prog, names) = compile_to_program_optimized_named(src).unwrap_or_else(|e| panic!("{func}: {e:?}"));
        let found = discover(&prog, &names);
        let at = names.iter().position(|n| n == func).unwrap_or_else(|| panic!("{func}: not in {names:?}"));
        match &found[at] {
            Discovered::Host(h) => assert!(h.reason.contains(needle), "{func}: the reason names {needle:?}: {}", h.reason),
            Discovered::Region(_) => panic!("{func} became a region"),
        }
    }
}
