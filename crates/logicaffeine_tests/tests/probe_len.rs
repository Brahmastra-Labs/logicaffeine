mod common;
use common::compile_to_rust;
#[test]
fn probe_len() {
    // Does the oracle know a counted-push-built array's length? `item 50 of arr`
    // where arr is push-built to 100 should be provable ([50,50] ⊆ [1,100]).
    let src = r#"## Main
Let mutable arr be a new Seq of Int.
Let mutable k be 0.
While k is less than 100:
    Push k * 2 to arr.
    Set k to k + 1.
Let x be item 50 of arr.
Show x.
"#;
    let rust = compile_to_rust(src).unwrap();
    eprintln!("HAS_ORACLE_HINT={}", rust.contains("oracle bounds hint"));
    eprintln!("decl: {}", rust.lines().find(|l| l.contains("arr:")).unwrap_or("?"));
}
