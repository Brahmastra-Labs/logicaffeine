//! Logicaffeine Web Application Entry Point
//!
//! This is the standalone WASM binary for the web IDE.

use logicaffeine_web::App;

fn main() {
    dioxus::launch(App);
}
