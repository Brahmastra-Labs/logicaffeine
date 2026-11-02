//! TYPE × TIER COVERAGE LOCK — the pit-of-success invariant that EVERY value type works on
//! EVERY execution tier, byte-identically. LOGOS's promise is that a program means the same thing
//! whether it runs on the tree-walker, the register VM, the AOT-native (Rust) backend, the direct
//! WASM backend, or the AOT-C (gcc) backend. A type that silently stops working on one tier is a
//! broken promise; this lock makes that impossible to introduce and un-ignorable to leave.
//!
//! HOW IT WORKS. For each representative program (one per value type) we take the tree-walker's
//! output as the ORACLE (it must always run cleanly), then run the same program through every other
//! tier and demand byte-identical output. A tier that diverges or fails to build is a GAP.
//!
//! THE RATCHET (pit of success). `KNOWN_GAPS` is the exhaustive, documented census of the gaps that
//! still exist. The lock FAILS if either direction drifts:
//!   * a gap appears that is NOT in `KNOWN_GAPS`  → a REGRESSION (a type just broke on a tier);
//!   * a `KNOWN_GAPS` entry no longer gaps        → it was FIXED, so DELETE the entry (ratchet down).
//! So coverage can only ever increase, and `KNOWN_GAPS` can only ever shrink. The goal state is
//! `KNOWN_GAPS == []` — every type on every tier. Until then, every closed gap is permanent.
//!
//! TIERS COVERED HERE: tree-walker (oracle), VM, and the AOT-C (gcc) backend. The WASM and
//! AOT-native tiers join this matrix next; their runners are heavier (wasmi host imports / rustc)
//! and land in a follow-up so this lock stays fast and always-runnable.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{compile_to_c, tw_outcome, vm_outcome};

/// A value type and a tiny program that constructs and `Show`s a representative instance of it.
/// The program must run cleanly on the tree-walker (the oracle); every other tier must agree.
struct TypeProbe {
    /// The `RuntimeValue` family this exercises (matches the census below).
    ty: &'static str,
    /// A self-contained `## Main` program whose output characterizes the type.
    program: &'static str,
}

/// One probe per value type. Add a row when a new `RuntimeValue` variant ships — the lock then
/// demands it work on every tier (or be listed in `KNOWN_GAPS` with a reason).
const PROBES: &[TypeProbe] = &[
    // ---- core scalar/collection tower: must work on EVERY tier (no gaps allowed) ----------
    TypeProbe {
        ty: "int",
        program: "## Main\nShow 6 * 7.\n",
    },
    TypeProbe {
        ty: "float",
        program: "## Main\nShow floor(3.5 * 2.0).\n",
    },
    TypeProbe {
        ty: "bool",
        program: "## Main\nShow 2 is greater than 1.\n",
    },
    TypeProbe {
        ty: "text",
        program: "## Main\nShow upper(\"hi\").\n",
    },
    TypeProbe {
        ty: "char",
        program: "## Main\nShow chr(65).\n",
    },
    TypeProbe {
        ty: "seq",
        program: "## Main\nShow sumSeq([1, 2, 3]).\n",
    },
    TypeProbe {
        ty: "map",
        program: "## Main\nLet m be countInts([1, 2, 2]).\nShow counterGet(m, 2).\n",
    },
    // ---- arbitrary precision: the C backend is int64, so this GAPS on C (see KNOWN_GAPS) ----
    TypeProbe {
        ty: "bigint",
        program: "## Main\nShow pow(2, 100).\n",
    },
    // ---- temporal tower: the C backend has no Moment/Date/Time runtime yet → GAPS on C -----
    TypeProbe {
        ty: "moment",
        program: "## Main\nShow year_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
    },
    TypeProbe {
        ty: "moment_weekday",
        program: "## Main\nShow weekday_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
    },
    TypeProbe {
        ty: "date",
        program: "## Main\nShow date_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
    },
    TypeProbe {
        ty: "time",
        program: "## Main\nShow time_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
    },
    // ---- the exact numeric tower above Int/BigInt (Phase 2): Rational, Decimal, Complex, Modular.
    //      C carries these via the `LRat`/`LDec`/`LComplex`/`LMod` runtimes; `Divide → ExactDivide`
    //      resolution (compile_to_c) keeps a Rational `/` exact instead of flooring. All agree ≡ oracle. ----
    TypeProbe {
        ty: "rational",
        program: "## Main\nLet x: Rational be 7 / 2.\nShow x.\n",
    },
    TypeProbe {
        ty: "decimal",
        program: "## Main\nShow decimal(\"0.1\") + decimal(\"0.2\").\n",
    },
    TypeProbe {
        ty: "complex",
        program: "## Main\nLet i be complex(0, 1).\nShow i * i.\n",
    },
    TypeProbe {
        ty: "modular",
        program: "## Main\nShow modular(10, 7).\n",
    },
];

/// The documented, shrinking census of (type, tier) pairs that do NOT yet agree with the oracle.
/// Each MUST carry a reason. DELETE an entry the moment its gap is closed — the lock forces it.
/// Goal: this list becomes empty.
const KNOWN_GAPS: &[(&str, &str)] = &[
    // GOAL STATE REACHED — every value type now agrees on every tier, so this census is EMPTY.
    //
    // The AOT-C bignum gap (`2^100` wrapped to 0 in raw int64) is CLOSED: integer `**` and `pow(a, b)`
    // now raise through the arbitrary-precision `LInt` runtime (`codegen_c/runtime.rs` `lint_pow` →
    // `lbig_pow` + `lint_norm`), which promotes on overflow and downsizes when a result fits — so C is
    // byte-for-byte the other four tiers (tree-walker/VM/AOT-native/WASM-linked). The exhaustive proof
    // is `c_bigint_power_lock.rs`.
    //
    // The temporal tower (Moment/Date/Time) was closed earlier: `codegen_c/runtime.rs` carries the
    // byte-identical Howard-Hinnant calendar + RFC-3339 parser + Show formatters.
];

fn is_known_gap(ty: &str, tier: &str) -> bool {
    KNOWN_GAPS.contains(&(ty, tier))
}

/// Compile `program` through the AOT-C backend, build it with gcc, and return its stdout — or
/// `None` if any step fails (unsupported construct, gcc error, runtime trap). `None` = a gap.
fn c_backend_output(program: &str) -> Option<String> {
    let c = compile_to_c(program).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_typetier_{}_{:x}",
        std::process::id(),
        c.len()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&src, &c).ok()?;
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .ok()?;
    let out = if compiled.status.success() {
        std::process::Command::new(&bin)
            .output()
            .ok()
            .filter(|r| r.status.success())
            .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string())
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Does gcc exist? The C tier is skipped (not failed) when it does not, so the lock still runs the
/// TW/VM half everywhere.
fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Record one tier's verdict against the ratchet and collect any violation message.
fn judge(ty: &str, tier: &str, agrees: bool, violations: &mut Vec<String>) {
    judge_verdict(ty, tier, agrees, is_known_gap(ty, tier), violations);
}

/// The pure ratchet verdict, decoupled from the LIVE `KNOWN_GAPS` set (the membership is passed in as
/// `is_gap`). This lets the guard test drive all four cases with a synthetic flag, so it stays valid
/// even at the GOAL STATE (`KNOWN_GAPS == []`, reached now that C carries bignum) — where there is no
/// live gap to sample.
fn judge_verdict(ty: &str, tier: &str, agrees: bool, is_gap: bool, violations: &mut Vec<String>) {
    match (agrees, is_gap) {
        (true, false) => {} // works and expected to — the pit of success
        (false, true) => {} // gaps and documented — accounted for, shrinking
        (false, false) => violations.push(format!(
            "REGRESSION: type `{ty}` no longer works on tier `{tier}`. A value type just broke on \
             a backend. Fix the backend — do NOT add it to KNOWN_GAPS to go green."
        )),
        (true, true) => violations.push(format!(
            "FIXED: type `{ty}` now works on tier `{tier}` — DELETE (\"{ty}\", \"{tier}\") from \
             KNOWN_GAPS so the ratchet locks the win in."
        )),
    }
}

#[test]
fn every_value_type_agrees_across_every_tier() {
    let gcc = have_gcc();
    let mut violations = Vec::new();

    for probe in PROBES {
        let oracle = tw_outcome(probe.program);
        assert!(
            oracle.error.is_none(),
            "the tree-walker ORACLE must run type `{}` cleanly, but it errored: {}\nSource:\n{}",
            probe.ty,
            oracle.error.as_deref().unwrap_or(""),
            probe.program
        );
        let want = oracle.output.trim();

        // VM tier.
        let vm = vm_outcome(probe.program);
        let vm_ok = vm.error.is_none() && vm.output.trim() == want;
        judge(probe.ty, "vm", vm_ok, &mut violations);

        // AOT-C tier (only when gcc is present).
        if gcc {
            let c_ok = c_backend_output(probe.program).as_deref() == Some(want);
            judge(probe.ty, "c", c_ok, &mut violations);
        }
    }

    assert!(
        violations.is_empty(),
        "TYPE × TIER COVERAGE broke ({} issue(s)):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// Guard on the guard: the ratchet logic itself must reject both drift directions, so we KNOW the
/// lock fires. Feeds `judge` each of the four (agrees × known-gap) cases and checks its verdict.
#[test]
fn ratchet_rejects_both_regression_and_silent_fix() {
    // Drive the verdict with a SYNTHETIC gap flag rather than the live `KNOWN_GAPS` — the guard must
    // hold even at the goal state (`KNOWN_GAPS == []`), where there is no real gap to sample.
    let mut v = Vec::new();
    judge_verdict("x", "c", true, false, &mut v); // works, not a documented gap → ok
    judge_verdict("x", "c", false, false, &mut v); // gaps, undocumented → REGRESSION
    assert_eq!(v.len(), 1, "an undocumented gap must be flagged");
    assert!(v[0].contains("REGRESSION"));

    let mut v2 = Vec::new();
    judge_verdict("x", "c", false, true, &mut v2); // gaps, documented → ok
    judge_verdict("x", "c", true, true, &mut v2); // works, still listed → FIXED, must delete
    assert_eq!(v2.len(), 1, "a silently-fixed gap must be flagged");
    assert!(v2[0].contains("FIXED"));
}

/// Every `KNOWN_GAPS` entry names a real probe type — no stale rows for types that were renamed
/// or deleted (those would hide a missing probe).
#[test]
fn known_gaps_reference_live_probe_types() {
    for (ty, _tier) in KNOWN_GAPS {
        assert!(
            PROBES.iter().any(|p| p.ty == *ty),
            "KNOWN_GAPS names type `{ty}`, but no probe exercises it — remove the stale row or add \
             the probe."
        );
    }
}
