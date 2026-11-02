//! Every `## To native` function the standard library declares RUNS on every tier.
//!
//! A native declaration has an empty body — the engine supplies the implementation. The compiled tier
//! always linked `crypto.lg`'s kernels, but the tree-walker and VM ran the empty body and returned
//! `Nothing`, so ML-KEM in the Studio failed with "Cannot index Nothing with Int" while the built binary
//! was correct. The kernels now live in ONE table (`semantics::native_kernels`) that all three read;
//! these locks keep it that way:
//!   • every declared native is implemented on the interpreters (a kernel, or a system native with its
//!     own seam) — a new `## To native` cannot ship half-wired;
//!   • every kernel is declared by the standard library, at the declaration's arity;
//!   • kernel codes (they travel in bytecode) never move;
//!   • a program calling EVERY kernel prints the same digest on the tree-walker, the VM, and the binary.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_all_tiers};
use logicaffeine_compile::semantics::native_kernels::NativeKernel;

/// The system natives, each served by its own seam on the interpreters (not a kernel).
const SYSTEM_NATIVES: &[&str] = &[
    "read",
    "write",
    "get",
    "args",
    "now",
    "sleep",
    "randomInt",
    "randomFloat",
    "tcpConnect",
    "tcpConnectTls",
    "tcpUpgradeTls",
    "tcpSend",
    "tcpRecv",
    "tcpRecvExact",
    "tcpListen",
    "tcpAccept",
    "tcpListenerPort",
    "tcpShutdownWrite",
    "tcpSetTimeout",
    "tcpClose",
];

/// `(name, parameter count)` of every `## To native` declaration in the standard library.
fn declared_natives() -> Vec<(String, usize)> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/assets/std");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the stdlib directory") {
        let path = entry.expect("an entry").path();
        if !matches!(path.extension().and_then(|e| e.to_str()), Some("lg" | "md")) {
            continue;
        }
        for line in std::fs::read_to_string(&path)
            .expect("a stdlib file")
            .lines()
        {
            if let Some(rest) = line.strip_prefix("## To native ") {
                let name = rest
                    .split(|c: char| c == ' ' || c == '(')
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push((name, rest.matches('(').count()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_declared_native_is_implemented_on_the_interpreters() {
    let declared = declared_natives();
    assert!(
        declared.len() >= 40,
        "the scan found only {} natives",
        declared.len()
    );
    let unwired: Vec<String> = declared
        .iter()
        .filter(|(name, arity)| match NativeKernel::from_name(name) {
            Some(k) => k.arity() != *arity,
            None => !SYSTEM_NATIVES.contains(&name.as_str()),
        })
        .map(|(n, a)| format!("{n}/{a}"))
        .collect();
    assert!(
        unwired.is_empty(),
        "stdlib natives with no interpreter implementation (they would return Nothing in the Studio): {unwired:?}"
    );
}

#[test]
fn every_kernel_is_declared_by_the_standard_library() {
    let declared = declared_natives();
    let orphans: Vec<&str> = NativeKernel::ALL
        .iter()
        .filter(|k| {
            !declared
                .iter()
                .any(|(n, a)| n == k.name() && *a == k.arity())
        })
        .map(|k| k.name())
        .collect();
    assert!(
        orphans.is_empty(),
        "kernels no `## To native` declares (at their arity): {orphans:?}"
    );
}

#[test]
fn kernel_codes_never_move() {
    let golden: Vec<&str> = include_str!("goldens/native_kernel_codes.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    let now: Vec<&str> = NativeKernel::ALL.iter().map(|k| k.name()).collect();
    assert!(now.len() >= golden.len(), "a kernel was removed");
    assert_eq!(
        &now[..golden.len()],
        &golden[..],
        "kernel codes moved — append new kernels at the END"
    );
    assert_eq!(
        now.len(),
        golden.len(),
        "a kernel was appended — append its name to the golden too"
    );
}

/// Calls every kernel and prints `name length checksum` for each result.
const EVERY_KERNEL: &str = r#"## To checksum (s: Seq of Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    Repeat for x in s:
        Set total to (total + x * i) % 1000000007.
        Set i to i + 1.
    Return total.

## To checksumW16 (s: Seq of Word16) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    Repeat for x in s:
        Set total to (total + intOfWord16(x) * i) % 1000000007.
        Set i to i + 1.
    Return total.

## To report (label: Text) and (s: Seq of Int):
    Show label + " " + format(length of s) + " " + format(checksum(s)).

## To reportW16 (label: Text) and (s: Seq of Word16):
    Show label + " " + format(length of s) + " " + format(checksumW16(s)).

## To bytesOf (n: Int) and (k: Int) -> Seq of Int:
    Let mutable r be a new Seq of Int.
    Repeat for i from 0 to n - 1:
        Push (i * k + 11) % 256 to r.
    Return r.

## To coeffsOf (k: Int) -> Seq of Int:
    Let mutable r be a new Seq of Int.
    Repeat for i from 0 to 255:
        Push (i * k + 7) % 3329 to r.
    Return r.

## To coeffsW16 (k: Int) -> Seq of Word16:
    Let mutable r be a new Seq of Word16.
    Repeat for i from 0 to 255:
        Push word16((i * k + 7) % 3329) to r.
    Return r.

## Main
Let a be coeffsOf(13).
Let b be coeffsOf(29).
Let aw be coeffsW16(13).
Let bw be coeffsW16(29).
Let seed be bytesOf(32, 5).
report("mlkemNtt", mlkemNtt(a)).
report("mlkemInvNtt", mlkemInvNtt(a)).
report("mlkemBaseMul", mlkemBaseMul(a, b)).
report("cbd2", cbd2(bytesOf(128, 3))).
report("cbd3", cbd3(bytesOf(192, 7))).
report("compress", compress(a, 4)).
report("decompress", decompress(compress(a, 4), 4)).
report("byteEncode", byteEncode(a, 12)).
report("byteDecode", byteDecode(byteEncode(a, 12), 12)).
report("sampleNtt", sampleNtt(bytesOf(840, 73))).
report("sampleA", sampleA(seed, 1, 2)).
report("toMont", toMont(a)).
reportW16("mlkemBaseMulW16", mlkemBaseMulW16(aw, bw)).
reportW16("toMontW16", toMontW16(aw)).
reportW16("cbd2W16", cbd2W16(bytesOf(128, 3))).
reportW16("compressW16", compressW16(aw, 10)).
reportW16("decompressW16", decompressW16(compressW16(aw, 10), 10)).
report("byteEncodeW16", byteEncodeW16(aw, 12)).
reportW16("byteDecodeW16", byteDecodeW16(byteEncodeW16(aw, 12), 12)).
reportW16("sampleAW16", sampleAW16(seed, 2, 1)).
reportW16("sampleMatrixW16", sampleMatrixW16(seed)).
reportW16("mlkemNoiseBatch", mlkemNoiseBatch(seed, 0, 3)).
reportW16("addModQW16", addModQW16(aw, bw)).
reportW16("subModQW16", subModQW16(aw, bw)).
reportW16("zerosW16", zerosW16(5)).
Let kp be mlkemKeypair(seed, bytesOf(32, 9)).
report("mlkemKeypair", kp).
Let ek be mlkemSubseq(kp, 0, 1184).
Let dk be mlkemSubseq(kp, 1184, 2400).
Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).
report("mlkemEncapsKem", sealed).
Let secret be mlkemDecapsKem(dk, mlkemSubseq(sealed, 0, 1088)).
report("mlkemDecapsKem", secret).
Show "shared secrets agree: " + format(secret equals mlkemSubseq(sealed, 1088, 32)).
Let dkp be mldsaKeypair(seed).
report("mldsaKeypair", dkp).
Let pk be mlkemSubseq(dkp, 0, 1952).
Let sk be mlkemSubseq(dkp, 1952, 4032).
Let msg be bytesOf(40, 31).
Let ctx be bytesOf(3, 1).
Let sig be mldsaSign(sk, msg, ctx).
report("mldsaSign", sig).
Show "mldsaVerify " + format(mldsaVerify(pk, msg, ctx, sig)).
Show "mldsaVerify tampered " + format(mldsaVerify(pk, bytesOf(40, 32), ctx, sig)).
"#;

#[test]
fn every_kernel_runs_identically_on_the_tree_walker_and_vm() {
    let r = run_all_tiers(EVERY_KERNEL);
    assert_eq!(r.error, None, "output so far:\n{}", r.output);
    let lines: Vec<&str> = r.output.trim().lines().collect();
    for k in NativeKernel::ALL {
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(&format!("{} ", k.name()))),
            "the digest never exercised {}:\n{}",
            k.name(),
            r.output
        );
    }
    assert!(
        lines.contains(&"shared secrets agree: true"),
        "{}",
        r.output
    );
    assert!(lines.contains(&"mldsaVerify 1"), "{}", r.output);
    assert!(lines.contains(&"mldsaVerify tampered 0"), "{}", r.output);
    assert!(lines.contains(&"zerosW16 5 0"), "{}", r.output);
    assert!(
        lines.iter().any(|l| l.starts_with("mlkemKeypair 3584 ")),
        "{}",
        r.output
    );
}

#[test]
fn every_kernel_runs_identically_when_compiled() {
    let r = run_all_tiers(EVERY_KERNEL);
    assert_eq!(r.error, None);
    assert_compiled_equals_interpreted_eq(EVERY_KERNEL, r.output.trim());
}

#[test]
fn a_programs_own_function_wins_over_a_kernel_name() {
    let src = "## To zerosW16 (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow zerosW16(21).\n";
    common::assert_all_tiers(src, "42");
}

/// A backend that cannot link the standard library's Rust kernels must REFUSE a program that calls
/// one — never lower the call to the declaration's empty body and print `nothing`.
#[test]
fn every_backend_without_the_kernels_refuses_them_loudly() {
    use logicaffeine_compile::compile;
    let src = "## Main\nLet z be zerosW16(3).\nShow length of z.\n";
    let outcomes: Vec<(&str, Result<(), String>)> = vec![
        (
            "C",
            compile::compile_to_c(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "C (from bytecode)",
            compile::compile_to_c_via_bytecode(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "RISC-V",
            compile::compile_to_riscv(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "WASM",
            compile::compile_to_wasm(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "soft core",
            logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
    ];
    let wrong: Vec<String> = outcomes
        .into_iter()
        .filter_map(|(backend, r)| match r {
            Err(e) if e.contains("zerosW16") => None,
            Err(e) => Some(format!("{backend}: refused without naming the kernel: {e}")),
            Ok(()) => Some(format!(
                "{backend}: lowered a native-kernel call it cannot link"
            )),
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
