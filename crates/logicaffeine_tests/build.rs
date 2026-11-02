//! A wasm32 test binary exports its shadow-stack pointer, as the web app's build does: the Studio's
//! fault boundary (`logicaffeine_web::ui::fault`) puts the pointer back from JavaScript after a
//! trap, and `wasm_fault_boundary` runs that boundary in real wasm. Native builds are untouched.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-tests=--export=__stack_pointer");
    }
}
