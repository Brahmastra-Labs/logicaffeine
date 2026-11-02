use super::*;

fn make_interner() -> Interner {
    Interner::new()
}

#[test]
fn test_verification_pass_creation() {
    let interner = make_interner();
    let pass = VerificationPass::new(&interner);
    // Just verify it constructs without panic
    drop(pass);
}
