//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! DATE DECOMPOSITION ON THE FABRIC — the pit-of-success ratchet for the FULL Gregorian calendar
//! (`year`/`month`/`day`/`quarter`/ISO-`week`/`weekday` + time-of-day `hour`/`minute`/`second`) lowering to
//! the combinational loop fabric, on EVERY loop shape.
//!
//! A date-component extractor reads a clock value — a `Moment` (nanos since epoch, from `now`) or a `Date`
//! (whole days, from `today`) — tracked by `REG_TIMEKIND`. Calendar fields lower to the full Howard-Hinnant
//! `civil_from_days` / `days_from_civil` tile chains (truncating `DivS` + `ModS` + `Select` era-adjusts);
//! time-of-day and weekday are pure div/mod. Each MUST lower on counted / while / nested loops, carry a
//! calendar `Func` (DivS/ModS) tile, and evaluate ≡ VM across a battery of dates that exercises leap years,
//! a year boundary (an ISO-week edge), a leap day, and the epoch. COVERAGE RATCHET: the (component × shape)
//! grid must be complete. Extend it; never delete a cell.
//!
//! A combinational fabric has NO wall clock, so a deterministic differential injects a FIXED clock
//! (`temporal::set_fixed_clock`) that BOTH the VM and the fabric read — the fabric bakes the instant into its
//! cells at build, so evaluating after the clock is restored still uses that instant.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_base::temporal::{days_from_civil, NANOS_PER_DAY, NANOS_PER_SECOND};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, eval_nested_fabric, eval_while_fabric, loop_fabric_from_source,
    nested_fabric_from_source, while_fabric_from_source, FuncUnit, LoopCell,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::semantics::temporal::{clear_fixed_clock, set_fixed_clock};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    Counted,
    While,
    Nested,
}
impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Counted => "counted",
            Shape::While => "while",
            Shape::Nested => "nested",
        }
    }
}

/// A date-component kernel: `key`, the clock source (`today` = Date / `now` = Moment), and the `the … of`
/// extractor phrase.
struct Kernel {
    key: &'static str,
    clock: &'static str,
    comp: &'static str,
}

const KERNELS: &[Kernel] = &[
    // Calendar fields on a Date — the full civil-from-days chain.
    Kernel {
        key: "year",
        clock: "today",
        comp: "the year of",
    },
    Kernel {
        key: "month",
        clock: "today",
        comp: "the month of",
    },
    Kernel {
        key: "day",
        clock: "today",
        comp: "the day of",
    },
    Kernel {
        key: "quarter",
        clock: "today",
        comp: "the quarter of",
    },
    Kernel {
        key: "week",
        clock: "today",
        comp: "the week of",
    },
    Kernel {
        key: "weekday_d",
        clock: "today",
        comp: "the weekday of",
    },
    // Time-of-day + weekday on a Moment.
    Kernel {
        key: "hour",
        clock: "now",
        comp: "the hour of",
    },
    Kernel {
        key: "minute",
        clock: "now",
        comp: "the minute of",
    },
    Kernel {
        key: "second",
        clock: "now",
        comp: "the second of",
    },
    Kernel {
        key: "weekday_m",
        clock: "now",
        comp: "the weekday of",
    },
];

const SHAPES: &[Shape] = &[Shape::Counted, Shape::While, Shape::Nested];

fn kernel_src(shape: Shape, k: &Kernel) -> (String, String) {
    let tag = match shape {
        Shape::Counted => "c",
        Shape::While => "w",
        Shape::Nested => "n",
    };
    let fname = format!("{}{}", k.key, tag);
    let base = if matches!(shape, Shape::Nested) {
        "            "
    } else {
        "        "
    };
    // `Let t be <clock>. Let c be <the … of> t. Set s to s + c.`
    let body = format!(
        "Let t be {}.\nLet c be {} t.\nSet s to s + c.",
        k.clock, k.comp
    );
    let ibody: String = body
        .lines()
        .map(|l| format!("{base}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let src = match shape {
        Shape::Counted => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to n:\n{ibody}\n    Return s.\n"
        ),
        Shape::While => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 0.\n    While i is less than n:\n{ibody}\n        Set i to i + 1.\n    Return s.\n"
        ),
        Shape::Nested => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to 2:\n        Repeat for j from 1 to n:\n{ibody}\n    Return s.\n"
        ),
    };
    (fname, src)
}

/// Build the loop fabric AND run the VM UNDER the injected clock, then compare. The fabric bakes the instant
/// at build, so evaluating after the (already-restored) clock is fine.
fn build_and_check(
    shape: Shape,
    fname: &str,
    src: &str,
    inputs: &[i64],
) -> Result<Vec<LoopCell>, String> {
    let (body, results): (Vec<LoopCell>, Vec<i64>) = match shape {
        Shape::Counted => {
            let lf = loop_fabric_from_source(src, fname);
            (
                lf.body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_loop_fabric(&lf, &[x]))
                    .collect(),
            )
        }
        Shape::While => {
            let wf = while_fabric_from_source(src, fname);
            (
                wf.body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_while_fabric(&wf, &[x]))
                    .collect(),
            )
        }
        Shape::Nested => {
            let nf = nested_fabric_from_source(src, fname);
            (
                nf.inner_body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_nested_fabric(&nf, &[x]))
                    .collect(),
            )
        }
    };
    for (&x, &got) in inputs.iter().zip(results.iter()) {
        let full = format!("{src}## Main\n    Show {fname}({x}).\n");
        let out = vm_outcome(&full);
        if let Some(e) = &out.error {
            return Err(format!("VM error {e:?} for {fname}({x})"));
        }
        let vm = out.output.trim().to_string();
        if got.to_string() != vm {
            return Err(format!("{fname}({x}): fabric {got} != VM {vm}"));
        }
    }
    Ok(body)
}

/// A date component lowers through the Gregorian calendar arithmetic — the body MUST carry a `DivS` or `ModS`
/// functional-unit tile (the civil-from-days divisions / the weekday·time mods).
fn body_has_calendar_func(body: &[LoopCell]) -> bool {
    body.iter().any(|c| {
        matches!(
            c,
            LoopCell::Func {
                unit: FuncUnit::DivS | FuncUnit::ModS,
                ..
            }
        )
    })
}

#[test]
fn date_decomposition_lowers_on_every_loop_shape_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {}));

    // A battery of instants: a leap year (Q1), a year boundary (an ISO-week edge → week 52/1), a leap DAY, the
    // epoch (a Thursday), and a mid-year date (Q3). Time-of-day 13:45:30 exercises hour/minute/second.
    let dates = [
        (2024, 3, 10),
        (2023, 12, 31),
        (2020, 2, 29),
        (1970, 1, 1),
        (2025, 7, 4),
    ];
    let tod = (13 * 3600 + 45 * 60 + 30) * NANOS_PER_SECOND; // 13:45:30

    let declared: BTreeSet<(&str, &str)> = SHAPES
        .iter()
        .flat_map(|s| KERNELS.iter().map(move |k| (s.name(), k.key)))
        .collect();
    let inputs: &[i64] = &[1, 2, 4];
    let mut realized: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

    for (y, m, d) in dates {
        let days = days_from_civil(y, m, d) as i32;
        let nanos = days as i64 * NANOS_PER_DAY + tod;
        for &shape in SHAPES {
            for k in KERNELS {
                let (fname, src) = kernel_src(shape, k);
                set_fixed_clock(days, nanos);
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    build_and_check(shape, &fname, &src, inputs)
                }));
                clear_fixed_clock();
                match built {
                    Ok(Ok(body)) => {
                        if !body_has_calendar_func(&body) {
                            failures.push(format!(
                                "{fname} [{}] @ {y}-{m}-{d}: no DivS/ModS calendar tile",
                                shape.name()
                            ));
                        } else {
                            realized.insert((shape.name(), k.key));
                        }
                    }
                    Ok(Err(e)) => failures.push(format!("{} @ {y}-{m}-{d}: {e}", shape.name())),
                    Err(e) => {
                        let msg = e
                            .downcast_ref::<String>()
                            .map(|s| s.as_str())
                            .or_else(|| e.downcast_ref::<&str>().copied())
                            .unwrap_or("panic");
                        failures.push(format!(
                            "{fname} [{}] @ {y}-{m}-{d}: did not lower: {}",
                            shape.name(),
                            &msg[..msg.len().min(80)]
                        ));
                    }
                }
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("=== date-fabric failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "date decomposition: {} cell(s) failed (see stderr)",
        failures.len()
    );
    assert_eq!(
        realized,
        declared,
        "date coverage gap: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "date-fabric: {} cells ({} components × {} loop shapes) over {} dates — the Gregorian calendar lowers to the fabric ≡ VM",
        declared.len(),
        KERNELS.len(),
        SHAPES.len(),
        dates.len()
    );
}
