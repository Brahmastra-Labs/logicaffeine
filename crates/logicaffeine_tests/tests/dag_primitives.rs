//! Primitive validation: a native fast path exists only for a definition proven to meet its
//! equations, a forged primitive is refused, and a validated one really computes natively.

use logicaffeine_kernel::dag::env::Safety;
use logicaffeine_kernel::dag::export::{check_export, Reader, Verdict};
use logicaffeine_kernel::dag::expr::Node;
use logicaffeine_kernel::dag::level::LevelsId;
use logicaffeine_kernel::dag::nat::Nat;
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;

fn prelude_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus/good/init-prelude.ndjson")
}

fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn the_prelude_validates_exactly_its_primitives() {
    let stats = big_stack(|| {
        let (v, stats) = check_export(BufReader::new(std::fs::File::open(prelude_path()).unwrap()));
        assert!(matches!(v, Verdict::Accept));
        stats
    });
    assert_eq!(
        stats.validated_primitives,
        [
            "Char.ofNat",
            "Nat.add",
            "Nat.beq",
            "Nat.ble",
            "Nat.div",
            "Nat.mod",
            "Nat.mul",
            "Nat.pow",
            "Nat.pred",
            "Nat.sub",
            "String.ofList"
        ]
    );
}

/// The Prelude with `Nat.pow`'s body replaced by `Nat.mul`'s: same type, wrong equations.
fn prelude_with_forged_pow() -> String {
    let text = std::fs::read_to_string(prelude_path()).unwrap();
    let mut names: std::collections::HashMap<u64, String> = Default::default();
    let mut defs: std::collections::HashMap<String, u64> = Default::default();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(i) = v.get("in").and_then(|i| i.as_u64()) {
            let (k, body) = if v.get("str").is_some() {
                ("str", &v["str"])
            } else {
                ("num", &v["num"])
            };
            let pre = body["pre"].as_u64().unwrap();
            let part = if k == "str" {
                body["str"].as_str().unwrap().to_string()
            } else {
                body["i"].to_string()
            };
            let full = if pre == 0 {
                part
            } else {
                format!("{}.{}", names[&pre], part)
            };
            names.insert(i, full);
        }
        if let Some(d) = v.get("def") {
            defs.insert(
                names[&d["name"].as_u64().unwrap()].clone(),
                d["value"].as_u64().unwrap(),
            );
        }
    }
    let mul_value = defs["Nat.mul"];
    text.lines()
        .map(|line| {
            let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
            if let Some(d) = v.get_mut("def") {
                if names[&d["name"].as_u64().unwrap()] == "Nat.pow" {
                    d["value"] = serde_json::json!(mul_value);
                }
            }
            v.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_forged_primitive_is_rejected() {
    let forged = prelude_with_forged_pow();
    let verdict = big_stack(move || check_export(BufReader::new(forged.as_bytes())).0);
    match verdict {
        Verdict::Reject(e) => assert!(e.to_string().contains("Nat.pow"), "{e}"),
        Verdict::Accept => panic!("a Nat.pow with multiplication's body was admitted"),
        Verdict::Decline(e) => panic!("declined instead of rejecting: {e}"),
    }
}

#[test]
fn a_validated_primitive_computes_natively() {
    big_stack(|| {
        let mut k = Kernel::new();
        Reader::new(&mut k)
            .run(BufReader::new(std::fs::File::open(prelude_path()).unwrap()))
            .unwrap();
        k.begin_decl(std::rc::Rc::from(Vec::new()), Safety::Safe);
        let pow = k.s.names.dotted("Nat.pow");
        let pow = k.s.cnst(pow, LevelsId::EMPTY);
        let two = k.s.nat_lit(Nat::from_u64(2));
        let exp = k.s.nat_lit(Nat::from_u64(100_000));
        let e = k.s.mk_app_n(pow, &[two, exp]);
        let t0 = std::time::Instant::now();
        let r = k.whnf(e).unwrap();
        let Node::NatLit(n) = k.s.node(r) else {
            panic!("2^100000 did not reduce to a literal")
        };
        assert_eq!(k.s.nat(n).bit_len(), 100_001);
        assert!(
            t0.elapsed().as_secs_f64() < 2.0,
            "2^100000 took {:?}",
            t0.elapsed()
        );
        k.end_decl();
    });
}
