#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use logos::compile::compile_to_rust;

    #[test]
    fn compile_assert_greater_than() {
        // Phase 5 Gate: "Assert that x is greater than 0." → debug_assert!(x > 0)
        let source = "## Main\nLet x be 5.\nAssert that x is greater than 0.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust_code = result.unwrap();
        assert!(rust_code.contains("debug_assert!"), "Should have debug_assert: {}", rust_code);
    }

    #[test]
    fn compile_let_and_return() {
        let source = "## Main\nLet x be 42.\nReturn x.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust_code = result.unwrap();
        assert!(rust_code.contains("fn main()"));
        assert!(rust_code.contains("let x = 42;"));
        assert!(rust_code.contains("return x;"));
    }

    #[test]
    fn compile_just_return() {
        let source = "## Main\nReturn 42.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust_code = result.unwrap();
        assert!(rust_code.contains("fn main()"));
        assert!(rust_code.contains("return 42;"));
    }

    #[test]
    fn compile_let_statement() {
        let source = "## Main\nLet x be 5.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust_code = result.unwrap();
        assert!(rust_code.contains("let x = 5;"));
    }

    #[test]
    fn compile_set_statement() {
        // Grand Challenge: x is reassigned, so must be `mut` for valid Rust
        let source = "## Main\nLet x be 5.\nSet x to 10.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust_code = result.unwrap();
        assert!(rust_code.contains("let mut x = 5;"), "Variable reassigned by Set must be mut: {}", rust_code);
        assert!(rust_code.contains("x = 10;"));
    }
}
