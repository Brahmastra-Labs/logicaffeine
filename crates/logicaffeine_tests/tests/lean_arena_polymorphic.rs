use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn imports_040_prodtype() {
    adapter::check_str(include_str!("fixtures/lean_arena/040_prodType.ndjson")).unwrap();
}

#[test]
fn imports_041_pprodtype() {
    adapter::check_str(include_str!("fixtures/lean_arena/041_pprodType.ndjson")).unwrap();
}

#[test]
fn imports_042_punittype() {
    adapter::check_str(include_str!("fixtures/lean_arena/042_pUnitType.ndjson")).unwrap();
}

#[test]
fn imports_045_rbtreedef() {
    adapter::check_str(include_str!("fixtures/lean_arena/045_rbTreeDef.ndjson")).unwrap();
}

#[test]
fn imports_060_typewithtypefieldpoly() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/060_typeWithTypeFieldPoly.ndjson"
    ))
    .unwrap();
}

#[test]
fn imports_066_prodrec() {
    adapter::check_str(include_str!("fixtures/lean_arena/066_prodRec.ndjson")).unwrap();
}

#[test]
fn imports_067_pprodrec() {
    adapter::check_str(include_str!("fixtures/lean_arena/067_pprodRec.ndjson")).unwrap();
}

#[test]
fn imports_068_punitrec() {
    adapter::check_str(include_str!("fixtures/lean_arena/068_punitRec.ndjson")).unwrap();
}

#[test]
fn imports_071_rbtreeref() {
    adapter::check_str(include_str!("fixtures/lean_arena/071_rbTreeRef.ndjson")).unwrap();
}
