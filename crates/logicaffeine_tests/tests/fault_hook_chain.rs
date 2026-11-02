//! The Studio's fault boundary takes a contained panic for itself and passes every other panic on.
//!
//! The boundary learns a panic's message from a panic hook it installs over whatever hook was there
//! before — in a development build, the one that logs the panic and raises dioxus's "App panicked!"
//! toast. A panic the boundary contains is the Studio's to report, so it must not also reach that
//! hook; a panic anywhere else must reach it exactly as before. The panic hook is process-wide,
//! so this lives alone in its own test binary.

#![cfg(feature = "web-tests")]

use logicaffeine_web::ui::fault::contain;
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicUsize, Ordering};

static REACHED: AtomicUsize = AtomicUsize::new(0);

#[test]
fn only_a_panic_outside_every_boundary_reaches_the_hook_installed_before() {
    std::panic::set_hook(Box::new(|_| {
        REACHED.fetch_add(1, Ordering::SeqCst);
    }));

    let contained = catch_unwind(|| contain(|| -> () { panic!("inside the boundary") }));
    assert!(matches!(contained, Ok(Err(_))), "the boundary did not contain the panic");
    assert_eq!(REACHED.load(Ordering::SeqCst), 0, "a contained panic was passed on to the earlier hook");

    let loose = catch_unwind(|| -> () { panic!("outside every boundary") });
    assert!(loose.is_err());
    assert_eq!(REACHED.load(Ordering::SeqCst), 1, "a panic outside every boundary never reached the earlier hook");
}
