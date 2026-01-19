//! Phase: Temporal Operations
//!
//! Tests for temporal arithmetic operations in the kernel prelude.
//! These operations are registered as global functions for duration and date arithmetic.

// =============================================================================
// Test 5.1: Duration arithmetic operations exist in prelude
// =============================================================================

#[test]
fn duration_add_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // add_duration : Duration -> Duration -> Duration
    assert!(ctx.get_global("add_duration").is_some(), "add_duration should be in prelude");
}

#[test]
fn duration_sub_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // sub_duration : Duration -> Duration -> Duration
    assert!(ctx.get_global("sub_duration").is_some(), "sub_duration should be in prelude");
}

#[test]
fn duration_mul_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // mul_duration : Duration -> Int -> Duration
    assert!(ctx.get_global("mul_duration").is_some(), "mul_duration should be in prelude");
}

#[test]
fn duration_div_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // div_duration : Duration -> Int -> Duration
    assert!(ctx.get_global("div_duration").is_some(), "div_duration should be in prelude");
}

// =============================================================================
// Test 5.2: Date/Moment operations exist in prelude
// =============================================================================

#[test]
fn date_add_days_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // date_add_days : Date -> Int -> Date (simple date + days offset)
    assert!(ctx.get_global("date_add_days").is_some(), "date_add_days should be in prelude");
}

#[test]
fn date_sub_date_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // date_sub_date : Date -> Date -> Int (difference in days)
    assert!(ctx.get_global("date_sub_date").is_some(), "date_sub_date should be in prelude");
}

#[test]
fn moment_add_duration_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // moment_add_duration : Moment -> Duration -> Moment
    assert!(ctx.get_global("moment_add_duration").is_some(), "moment_add_duration should be in prelude");
}

#[test]
fn moment_sub_moment_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // moment_sub_moment : Moment -> Moment -> Duration
    assert!(ctx.get_global("moment_sub_moment").is_some(), "moment_sub_moment should be in prelude");
}

// =============================================================================
// Test 5.3: Comparison operations exist
// =============================================================================

#[test]
fn date_compare_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // date_lt : Date -> Date -> Bool (date ordering)
    assert!(ctx.get_global("date_lt").is_some(), "date_lt should be in prelude");
}

#[test]
fn moment_compare_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // moment_lt : Moment -> Moment -> Bool (moment ordering)
    assert!(ctx.get_global("moment_lt").is_some(), "moment_lt should be in prelude");
}

#[test]
fn duration_compare_operation_exists() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // duration_lt : Duration -> Duration -> Bool (duration ordering)
    assert!(ctx.get_global("duration_lt").is_some(), "duration_lt should be in prelude");
}
