use super::*;
use std::mem::size_of;

#[test]
fn test_ast_node_sizes() {
    println!("LogicExpr size: {} bytes", size_of::<LogicExpr>());
    println!("Term size: {} bytes", size_of::<Term>());
    println!("NounPhrase size: {} bytes", size_of::<NounPhrase>());

    assert!(
        size_of::<LogicExpr>() <= 48,
        "LogicExpr is {} bytes - consider boxing large variants",
        size_of::<LogicExpr>()
    );
    assert!(
        size_of::<Term>() <= 32,
        "Term is {} bytes",
        size_of::<Term>()
    );
}
