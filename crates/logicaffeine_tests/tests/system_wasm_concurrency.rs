//! Browser (wasm) cross-target runtime tests (Phase 9a/9b) — the wasm twin of the native
//! `concurrency::tests`. Exercises [`BrowserRuntime`]'s `Yield`/`Timer`/`Spawner` over the
//! real browser event loop (`setTimeout` + `spawn_local`) and the shared tokio-sync `Pipe`
//! channel.
//!
//! Runs under `wasm-pack test --headless` (or node with a polyfill); inert on a normal
//! `cargo test`. The native side is covered by the in-crate `#[tokio::test]`s; this is the
//! same behavioral contract on the browser runtime.
//!
//! Build/run with the `concurrency` feature: `wasm-pack test --headless --chrome
//! crates/logicaffeine_system --features concurrency`.

#![cfg(all(target_arch = "wasm32", feature = "concurrency"))]

use logicaffeine_system::concurrency::{BrowserRuntime, Pipe, Spawner, Timer, Yield};
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
async fn browser_runtime_yield_completes() {
    // The 0-ms macrotask yield must resolve (hand control back to the event loop, not hang).
    BrowserRuntime.yield_now().await;
}

#[wasm_bindgen_test]
async fn browser_runtime_timer_completes() {
    // A short setTimeout-backed sleep resolves. (Wall-clock timing in the browser is fuzzy,
    // so we assert completion, not a precise duration — the native test pins the duration.)
    BrowserRuntime.sleep_ms(5).await;
}

#[wasm_bindgen_test]
async fn browser_runtime_timer_zero_ms_completes() {
    // Edge case: a 0 ms sleep is a degenerate-but-legal request and must still resolve.
    BrowserRuntime.sleep_ms(0).await;
}

#[wasm_bindgen_test]
async fn browser_runtime_spawn_runs_a_task_to_completion() {
    // The spawn + channel round-trip on the browser microtask queue: a spawned task delivers
    // its value over the shared tokio-sync `Pipe`.
    let (tx, mut rx) = Pipe::<i64>::new(1);
    let _task = BrowserRuntime.spawn_task(Box::pin(async move {
        tx.send(42).await.unwrap();
    }));
    assert_eq!(rx.recv().await, Some(42));
}

#[wasm_bindgen_test]
async fn browser_cross_target_traits_are_object_safe() {
    // Edge case / the point: the interpreter holds `&dyn Yield` / `&dyn Timer` on the browser
    // exactly as on native — `async_trait` boxing keeps the async methods object-safe.
    let rt = BrowserRuntime;
    let y: &dyn Yield = &rt;
    let t: &dyn Timer = &rt;
    y.yield_now().await;
    t.sleep_ms(1).await;
}
