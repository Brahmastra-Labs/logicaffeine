//! Phase 50: The Fortress (Security Policies)
//!
//! Tests for policy-based security with mandatory runtime enforcement.
//! - `## Policy` block for defining predicates and capabilities
//! - `Check` verb for mandatory runtime guards (never optimized out)
//! - Assert remains debug-only (can be optimized out in release)

mod common;
use common::compile_to_rust;

// =============================================================================
// Policy Block Parsing Tests
// =============================================================================

#[test]
fn test_simple_predicate_definition() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is admin if the user's role equals "admin".

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile policy definitions");

    // Verify predicate method is generated
    assert!(
        rust.contains("fn is_admin(&self) -> bool"),
        "Should generate is_admin predicate method. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("self.role == \"admin\""),
        "Predicate should check role field. Got:\n{}",
        rust
    );
}

#[test]
fn test_capability_with_object_reference() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

A Document has:
    an owner, which is User.

## Policy

A User is admin if the user's role equals "admin".

A User can publish the Document if:
    The user is admin, OR
    The user equals the document's owner.

## Main
    Let u be a new User.
    Let d be a new Document."#;

    let rust = compile_to_rust(source).expect("Should compile capability definitions");

    // Verify capability method takes object parameter
    assert!(
        rust.contains("fn can_publish(&self, document: &Document) -> bool"),
        "Should generate can_publish with Document parameter. Got:\n{}",
        rust
    );
    // Verify capability references predicate
    assert!(
        rust.contains("self.is_admin()"),
        "Capability should call predicate. Got:\n{}",
        rust
    );
    // Verify object field comparison
    assert!(
        rust.contains("document.owner"),
        "Capability should reference object field. Got:\n{}",
        rust
    );
}

// =============================================================================
// Check Statement Tests
// =============================================================================

#[test]
fn test_check_vs_assert_codegen() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is admin if the user's role equals "admin".

## Main

To verify (user: User):
    Assert that 1 equals 1.
    Check that the user is admin."#;

    let rust = compile_to_rust(source).expect("Should compile Check statements");

    // Assert should use debug_assert! (can be optimized out)
    assert!(
        rust.contains("debug_assert!("),
        "Assert should compile to debug_assert!. Got:\n{}",
        rust
    );

    // Check should use mandatory if-guard (never optimized out)
    assert!(
        rust.contains("if !(user.is_admin())"),
        "Check should compile to mandatory if-guard. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("logos_core::panic_with"),
        "Check should use panic_with for security violation. Got:\n{}",
        rust
    );
}

#[test]
fn test_check_includes_source_location() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is admin if the user's role equals "admin".

## Main

To verify (user: User):
    Check that the user is admin."#;

    let rust = compile_to_rust(source).expect("Should compile");

    // Error message should include line number
    assert!(
        rust.contains("Security Check Failed at line"),
        "Error message should include source location. Got:\n{}",
        rust
    );
    // Error message should include policy description
    assert!(
        rust.contains("user is admin"),
        "Error message should include policy text. Got:\n{}",
        rust
    );
}

#[test]
fn test_check_with_capability_and_object() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

A Document has:
    an owner, which is User.

## Policy

A User is admin if the user's role equals "admin".

A User can publish the Document if:
    The user is admin, OR
    The user equals the document's owner.

## Main

To handle_request (user: User, doc: Document):
    Check that the user can publish the document."#;

    let rust = compile_to_rust(source).expect("Should compile capability check");

    // Check should call capability method with object argument
    assert!(
        rust.contains("user.can_publish(&doc)"),
        "Check should call capability with object reference. Got:\n{}",
        rust
    );
}

// =============================================================================
// Policy Composition Tests
// =============================================================================

#[test]
fn test_policy_with_and_condition() {
    let source = r#"## Definition
A User has:
    a role, which is Text.
    a verified, which is Bool.

## Policy

A User is trusted if:
    The user's role equals "admin", AND
    The user's verified equals true.

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile AND conditions");

    assert!(
        rust.contains("fn is_trusted(&self) -> bool"),
        "Should generate is_trusted predicate. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("&&"),
        "Should generate AND condition. Got:\n{}",
        rust
    );
}

#[test]
fn test_policy_with_or_condition() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is privileged if:
    The user's role equals "admin", OR
    The user's role equals "moderator".

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile OR conditions");

    assert!(
        rust.contains("fn is_privileged(&self) -> bool"),
        "Should generate is_privileged predicate. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("||"),
        "Should generate OR condition. Got:\n{}",
        rust
    );
}

#[test]
fn test_policy_references_another_predicate() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is admin if the user's role equals "admin".

A User is superuser if the user is admin.

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile predicate references");

    // superuser should call admin
    assert!(
        rust.contains("fn is_superuser(&self) -> bool"),
        "Should generate is_superuser predicate. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("self.is_admin()"),
        "Predicate should reference other predicate. Got:\n{}",
        rust
    );
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_full_security_flow() {
    let source = r#"## Definition
A User has:
    a username, which is Text.
    a role, which is Text.

A Post has:
    a title, which is Text.
    an author, which is User.

## Policy

A User is admin if the user's role equals "admin".

A User can edit the Post if:
    The user is admin, OR
    The user equals the post's author.

A User can delete the Post if the user is admin.

## Main

To update_post (user: User, post: Post, new_title: Text):
    Check that the user can edit the post.
    Set post's title to new_title.

To remove_post (user: User, post: Post):
    Check that the user can delete the post.
    Show "Post deleted"."#;

    let rust = compile_to_rust(source).expect("Should compile full security flow");

    // Verify all policy methods are generated
    assert!(rust.contains("fn is_admin(&self) -> bool"), "Missing is_admin");
    assert!(rust.contains("fn can_edit(&self, post: &Post) -> bool"), "Missing can_edit");
    assert!(rust.contains("fn can_delete(&self, post: &Post) -> bool"), "Missing can_delete");

    // Verify Check statements are enforced
    assert!(rust.contains("user.can_edit(&post)"), "Missing can_edit check");
    assert!(rust.contains("user.can_delete(&post)"), "Missing can_delete check");

    // Verify panic_with is used (not debug_assert)
    let panic_count = rust.matches("logos_core::panic_with").count();
    assert_eq!(panic_count, 2, "Should have 2 Check enforcement points");
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_predicate_with_boolean_field() {
    let source = r#"## Definition
A User has:
    a verified, which is Bool.

## Policy

A User is verified if the user's verified equals true.

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile boolean field predicate");
    assert!(
        rust.contains("fn is_verified(&self) -> bool"),
        "Should generate is_verified. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("self.verified == true"),
        "Should check boolean field. Got:\n{}",
        rust
    );
}

#[test]
fn test_multiple_predicates_same_type() {
    let source = r#"## Definition
A User has:
    a role, which is Text.
    a active, which is Bool.

## Policy

A User is admin if the user's role equals "admin".
A User is moderator if the user's role equals "moderator".
A User is active if the user's active equals true.

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile multiple predicates");
    assert!(rust.contains("fn is_admin(&self) -> bool"), "Missing is_admin");
    assert!(rust.contains("fn is_moderator(&self) -> bool"), "Missing is_moderator");
    assert!(rust.contains("fn is_active(&self) -> bool"), "Missing is_active");
}

#[test]
fn test_multiple_capabilities_same_type() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

A Document has:
    an owner, which is User.

## Policy

A User is admin if the user's role equals "admin".

A User can view the Document if the user is admin.
A User can edit the Document if the user is admin.
A User can delete the Document if the user is admin.

## Main
    Let u be a new User.
    Let d be a new Document."#;

    let rust = compile_to_rust(source).expect("Should compile multiple capabilities");
    assert!(rust.contains("fn can_view(&self, document: &Document)"), "Missing can_view");
    assert!(rust.contains("fn can_edit(&self, document: &Document)"), "Missing can_edit");
    assert!(rust.contains("fn can_delete(&self, document: &Document)"), "Missing can_delete");
}

#[test]
fn test_check_in_nested_block() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is admin if the user's role equals "admin".

## Main

To process (user: User, flag: Bool):
    If flag:
        Check that the user is admin.
        Show "checked"."#;

    let rust = compile_to_rust(source).expect("Should compile Check in nested block");
    // Check should still be generated with proper indentation
    assert!(
        rust.contains("user.is_admin()"),
        "Should generate Check inside if block. Got:\n{}",
        rust
    );
}

#[test]
fn test_empty_policy_block() {
    // Policy block with no definitions should be valid
    let source = r#"## Definition
A User has:
    a name, which is Text.

## Policy

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile empty policy block");
    assert!(rust.contains("pub struct User"), "Should still generate struct");
}
