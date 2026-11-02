//! The crate's examples include the Lean Kernel Arena's checkers (`examples/logos_kernel.rs`,
//! `examples/logos_dual.rs`). On Linux they are linked as position-dependent executables: the loader
//! then patches none of a binary's own tables at start-up, which the Arena counts in every test. The
//! examples alone: the library, the tests and everything else the build links keep the toolchain's
//! default.

//!
//! The published crate ships no examples, and cargo refuses an examples-only link argument for a package
//! without one, so the argument is given only where `examples/` exists.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let examples = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("examples");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") && examples.is_dir() {
        println!("cargo:rustc-link-arg-examples=-no-pie");
    }
}
