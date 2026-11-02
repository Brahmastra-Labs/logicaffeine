use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};
fn literal(value: &str, with_nat: bool) -> String {
    let mut rows: Vec<Value> = if with_nat {
        include_str!("fixtures/lean_arena/044_natDef.ndjson")
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    } else {
        vec![json!({"meta":{"format":{"version":"3.1.0"}}})]
    };
    // The official tutorial names its natural type N; this synthetic variant
    // gives the same checked inductive the reserved literal name Nat.
    for row in &mut rows {
        if row["str"]["pre"] == 0 && row["str"]["str"] == "N" {
            row["str"]["str"] = json!("Nat");
        }
    }
    let ni = rows
        .iter()
        .filter_map(|r| r["in"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let ei = rows
        .iter()
        .filter_map(|r| r["ie"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let nat = rows
        .iter()
        .find(|r| r["str"]["str"] == "Nat" && r["str"]["pre"] == 0)
        .map(|r| r["in"].as_u64().unwrap())
        .unwrap_or(ni + 1);
    if !with_nat {
        rows.push(json!({"in":nat,"str":{"pre":0,"str":"Nat"}}));
    }
    rows.extend([
        json!({"in":ni,"str":{"pre":0,"str":"literal_test"}}),
        json!({"ie":ei,"const":{"name":nat,"us":[]}}),
        json!({"ie":ei+1,"natVal":value}),
        json!({"def":{"all":[ni],"name":ni,"type":ei,"value":ei+1,"levelParams":[],"safety":"safe","hints":"abbrev"}})
    ]);
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}
#[test]
fn zero_literal_uses_checked_nat() {
    adapter::check_str(&literal("0", true)).unwrap();
}
#[test]
fn successor_literal_uses_checked_nat() {
    adapter::check_str(&literal("2", true)).unwrap();
}
#[test]
fn nat_literal_requires_its_inductive() {
    let e = adapter::check_str(&literal("0", false)).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}
/// Lean's kernel represents a Nat literal in binary (GMP) and accepts one of any size, and so does
/// the reader (see lean_arena_big_naturals): 10^32 is accepted as ONE compact literal, never
/// expanded to unary `Succ` applications. Oversized INPUT is still bounded by the parsing budget
/// (lean_arena_big_naturals::natural_parser_budget_is_about_input_work_not_u64_overflow).
#[test]
fn oversized_literal_is_accepted_without_allocating_its_unary_expansion() {
    let digits = "100000000000000000000000000000000";
    let (ctx, _) =
        adapter::import_audited(&literal(digits, true)).expect("Lean accepts a 10^32 Nat literal");
    let body = ctx
        .get_definition_body("/s:\"literal_test\"")
        .expect("the literal definition");
    let logicaffeine_kernel::Term::Lit(logicaffeine_kernel::Literal::Nat(n)) = body else {
        panic!("the literal stays compact: {body:?}")
    };
    assert_eq!(n.to_string(), digits);
    // Without its checked Nat the same literal is still rejected.
    assert_eq!(
        adapter::check_str(&literal(digits, false))
            .unwrap_err()
            .code(),
        1
    );
}
#[test]
fn malformed_literal_is_not_a_proof_rejection() {
    let e = adapter::check_str(&literal("-1", true)).unwrap_err();
    assert_eq!(e.code(), 2, "{e:?}");
}

#[test]
fn large_nat_literal_uses_the_existing_compact_representation() {
    adapter::check_str(&literal("125330", true)).unwrap();
}

#[test]
fn an_unrelated_inductive_named_nat_does_not_enable_primitive_rules() {
    let input = literal("0", true);
    let mut rows: Vec<Value> = input
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Change the carrier from Type to Prop. This is a legitimate two-constructor
    // inductive, but it is not the kernel's primitive natural datatype.
    let arity = rows
        .iter()
        .find_map(|r| r["inductive"]["types"][0]["type"].as_u64())
        .unwrap();
    let zero_level = json!(0);
    let row = rows
        .iter_mut()
        .find(|r| r["ie"].as_u64() == Some(arity))
        .unwrap();
    row["sort"] = zero_level;
    let input: String = rows.into_iter().map(|r| r.to_string() + "\n").collect();
    assert_eq!(adapter::check_str(&input).unwrap_err().code(), 2);
}
