//! A standalone relay host — the native node a browser dials. Used by
//! `scripts/test-wasm-relay.sh` to back the headless-browser relay test, and a
//! minimal example of "a native server being its own relay".
//!
//! Run: `cargo run -p logicaffeine-system --features relay --example relay_host -- 127.0.0.1:9944`

#[cfg(all(not(target_arch = "wasm32"), feature = "relay"))]
#[tokio::main]
async fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:9944".to_string());
    let relay = logicaffeine_system::relay::serve(&addr)
        .await
        .expect("relay binds");
    eprintln!("relay listening on {}", relay.url());
    // Run until killed by the test harness.
    std::future::pending::<()>().await;
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "relay")))]
fn main() {
    eprintln!("relay_host requires the `relay` feature on a native target");
}
