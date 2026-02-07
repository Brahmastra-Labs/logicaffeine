//! E2E Tests: Security Policies
//!
//! Tests runtime behavior of policy enforcement: predicates, capabilities,
//! Check statements that should pass or panic, and error messages.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_exact_output, assert_panics};

// =============================================================================
// Predicate Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_predicate_passes() {
    assert_exact_output(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "admin".
Check that the u is admin.
Show "passed"."#,
        "passed",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_predicate_fails() {
    assert_panics(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "guest".
Check that the u is admin."#,
        "Security Check Failed",
    );
}

// =============================================================================
// Capability Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_capability_owner_passes() {
    // Owner can edit their own document
    // Note: We create two separate User objects with same values since the first gets moved into Document
    assert_exact_output(
        r#"## Definition
A User has:
    a name, which is Text.
    a role, which is Text.

A Document has:
    a title, which is Text.
    an owner, which is User.

## Policy
A User is admin if the user's role equals "admin".

A User can edit the Document if:
    The user is admin, OR
    The user equals the document's owner.

## Main
To test_edit (user: User, doc: Document):
    Check that the user can edit the document.
    Show "edit allowed".

Let owner1 be a new User with name "alice" and role "user".
Let owner2 be a new User with name "alice" and role "user".
Let doc be a new Document with title "My Doc" and owner owner1.
test_edit(owner2, doc)."#,
        "edit allowed",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_capability_admin_passes() {
    // Admin can edit any document
    assert_exact_output(
        r#"## Definition
A User has:
    a name, which is Text.
    a role, which is Text.

A Document has:
    a title, which is Text.
    an owner, which is User.

## Policy
A User is admin if the user's role equals "admin".

A User can edit the Document if:
    The user is admin, OR
    The user equals the document's owner.

## Main
To test_edit (user: User, doc: Document):
    Check that the user can edit the document.
    Show "admin edit allowed".

Let owner be a new User with name "bob" and role "user".
Let admin be a new User with name "alice" and role "admin".
Let doc be a new Document with title "Bob's Doc" and owner owner.
test_edit(admin, doc)."#,
        "admin edit allowed",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_capability_non_owner_fails() {
    // Non-admin, non-owner cannot edit
    assert_panics(
        r#"## Definition
A User has:
    a name, which is Text.
    a role, which is Text.

A Document has:
    a title, which is Text.
    an owner, which is User.

## Policy
A User is admin if the user's role equals "admin".

A User can edit the Document if:
    The user is admin, OR
    The user equals the document's owner.

## Main
To test_edit (user: User, doc: Document):
    Check that the user can edit the document.
    Show "should not reach".

Let owner be a new User with name "bob" and role "user".
Let stranger be a new User with name "eve" and role "user".
Let doc be a new Document with title "Bob's Doc" and owner owner.
test_edit(stranger, doc)."#,
        "Security Check Failed",
    );
}

// =============================================================================
// Error Message Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_check_error_message() {
    // Verify panic message includes policy context
    assert_panics(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "guest".
Check that the u is admin."#,
        "admin",
    );
}

// =============================================================================
// Assert vs Check Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_assert_passes() {
    // Assert with true condition doesn't panic
    assert_exact_output(
        r#"## Main
Assert that 1 equals 1.
Show "assert passed"."#,
        "assert passed",
    );
}

// =============================================================================
// Predicate Composition Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_predicate_composition() {
    // Predicate calls another predicate
    assert_exact_output(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".
A User is superuser if the user is admin.

## Main
Let u be a new User with role "admin".
Check that the u is superuser.
Show "super"."#,
        "super",
    );
}

// =============================================================================
// AND/OR Condition Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_and_condition_passes() {
    assert_exact_output(
        r#"## Definition
A User has:
    a role, which is Text.
    a verified, which is Bool.

## Policy
A User is trusted if:
    The user's role equals "admin", AND
    The user's verified equals true.

## Main
Let u be a new User with role "admin" and verified true.
Check that the u is trusted.
Show "trusted"."#,
        "trusted",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_or_condition_passes() {
    // OR condition: either condition can satisfy
    assert_exact_output(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is privileged if:
    The user's role equals "admin", OR
    The user's role equals "moderator".

## Main
Let u be a new User with role "moderator".
Check that the u is privileged.
Show "privileged"."#,
        "privileged",
    );
}
