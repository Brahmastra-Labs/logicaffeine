//! Go-like Concurrency Primitives
//!
//! Provides green thread primitives for LOGOS with ergonomic async APIs:
//!
//! - [`TaskHandle<T>`]: Spawned task handle with abort/completion tracking
//! - [`Pipe<T>`]: Bounded channel with sender/receiver split (Go-like channels)
//! - [`spawn`]: Ergonomic task spawning returning a `TaskHandle`
//! - [`check_preemption`]: Cooperative yielding for long-running computations
//!
//! # Cooperative Preemption
//!
//! The [`check_preemption`] function implements cooperative multitasking with a
//! 10ms threshold. This value balances responsiveness (shorter = more responsive
//! UI/network) against overhead (longer = less context-switch cost). For CPU-bound
//! loops, insert `check_preemption().await` to prevent starving other tasks.
//!
//! # Features
//!
//! Requires the `concurrency` feature.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::concurrency::{spawn, Pipe, check_preemption};
//!
//! # fn main() {}
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # async fn expensive_computation() -> i32 { 42 }
//! # fn heavy_work(_: i32) {}
//! // Spawn a task
//! let handle = spawn(async { expensive_computation().await });
//!
//! // Channel communication
//! let (tx, mut rx) = Pipe::<String>::new(16);
//! tx.send("hello".to_string()).await?;
//! let msg = rx.recv().await;
//!
//! // Cooperative yielding in long loops
//! for i in 0..1_000_000 {
//!     heavy_work(i);
//!     check_preemption().await;
//! }
//! # Ok(())
//! # }
//! ```

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
// Only the native `TaskHandle`'s `Future` impl uses these.
#[cfg(not(target_arch = "wasm32"))]
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::sync::mpsc;
// `JoinHandle`/`JoinError` are runtime-tied (tokio "rt") → native only. The cross-target
// channel (`mpsc`) is from tokio's runtime-agnostic "sync" feature, shared by both targets.
#[cfg(not(target_arch = "wasm32"))]
use tokio::task::JoinHandle;

// Re-export error types for ergonomic API
pub use tokio::sync::mpsc::error::{SendError, TryRecvError, TrySendError};
#[cfg(not(target_arch = "wasm32"))]
pub use tokio::task::JoinError;

// =============================================================================
// Mode-B deterministic replay — the seeded choice function
// =============================================================================
//
// The interpreter/VM resolve every scheduling nondeterminism through one seeded
// SplitMix64 chooser (`logicaffeine_runtime::seed`). Invariant I6 forbids linking
// that crate into AOT binaries, so Mode-B (`largo build --deterministic` /
// `LOGOS_SEED=…`) shares the *choice function* — the exact same SplitMix64
// algorithm + `below(n)` rule — re-implemented here. Identical algorithm + seed +
// draw order ⇒ identical winner, so a compiled `Select` reproduces the
// interpreter's choice. (Production / Mode-A never touches any of this.)

/// SplitMix64 — byte-for-byte the interpreter's `SeededRng`. Tiny, allocation-
/// free, identical on every target.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `[0, n)`. Returns 0 when `n <= 1` *without drawing* —
    /// the exact rule the interpreter uses, so single-ready decisions never
    /// advance the RNG and the draw sequences stay aligned.
    fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

static SEEDED_CHOOSER: std::sync::OnceLock<std::sync::Mutex<SplitMix64>> =
    std::sync::OnceLock::new();

/// Deterministic seeded pick among `n` ready options, sharing the interpreter's
/// SplitMix64 choice function (seeded by `LOGOS_SEED`, default 0). A Mode-B
/// compiled `Select` over multiple simultaneously-ready arms calls this with the
/// ready-arm count to reproduce the interpreter's winner. Returns 0 for `n <= 1`.
///
/// The chooser is process-global and seeded once on first use, so successive
/// selects draw in program order — the same order the interpreter draws.
pub fn seeded_pick(n: usize) -> usize {
    let chooser = SEEDED_CHOOSER.get_or_init(|| {
        let seed = std::env::var("LOGOS_SEED")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        std::sync::Mutex::new(SplitMix64 { state: seed })
    });
    chooser.lock().unwrap().below(n)
}

/// Whether Mode-B deterministic replay is active for this process — i.e. the
/// program was built `--deterministic` AND `LOGOS_SEED` is set. The emitted
/// `Select` consults this to choose the seeded path vs raw `tokio::select!`.
pub fn deterministic_replay_enabled() -> bool {
    std::env::var("LOGOS_SEED").is_ok()
}

// =============================================================================
// TaskHandle<T> - Wrapper around JoinHandle with abort/completion tracking
// =============================================================================

/// Handle to a spawned async task.
///
/// Wraps `tokio::task::JoinHandle<T>` with a LOGOS-friendly API.
///
/// # Example
/// ```no_run
/// use logicaffeine_system::concurrency::spawn;
///
/// # fn main() {}
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let handle = spawn(async { 42 });
/// // Do other work...
/// if handle.is_finished() {
///     let result = handle.await?;
/// }
/// # Ok(())
/// # }
/// ```
#[cfg(not(target_arch = "wasm32"))]
pub struct TaskHandle<T> {
    inner: JoinHandle<T>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> TaskHandle<T> {
    /// Create a new TaskHandle wrapping a JoinHandle.
    pub(crate) fn new(handle: JoinHandle<T>) -> Self {
        Self { inner: handle }
    }

    /// Check if the task has completed.
    ///
    /// Returns `true` if the task has finished (successfully or with error),
    /// `false` if still running.
    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    /// Abort the task.
    ///
    /// The task will be cancelled at the next await point.
    /// If the task has already completed, this has no effect.
    pub fn abort(&self) {
        self.inner.abort();
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Future for TaskHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx)
    }
}

// =============================================================================
// spawn() - Ergonomic task spawning
// =============================================================================

/// Spawn an async task and return a handle to it.
///
/// This is a thin wrapper around `tokio::spawn` that returns
/// a `TaskHandle<T>` for LOGOS codegen.
///
/// # Example
/// ```no_run
/// use logicaffeine_system::concurrency::spawn;
///
/// # fn main() {}
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let handle = spawn(async {
///     42
/// });
/// let result = handle.await?;
/// # Ok(())
/// # }
/// ```
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F, T>(future: F) -> TaskHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    TaskHandle::new(tokio::spawn(future))
}

// =============================================================================
// Pipe<T> - Bounded channel with sender/receiver split
// =============================================================================

/// A bounded channel for communication between tasks.
///
/// `Pipe<T>` provides Go-like channel semantics with a capacity limit.
/// Unlike Go, sender and receiver are split for Rust's ownership model.
///
/// # Example
/// ```no_run
/// use logicaffeine_system::concurrency::{spawn, Pipe};
///
/// # fn main() {}
/// # async fn example() {
/// let (tx, mut rx) = Pipe::<String>::new(16);
///
/// spawn(async move {
///     tx.send("hello".to_string()).await.unwrap();
/// });
///
/// let msg = rx.recv().await;
/// # }
/// ```
pub struct Pipe<T>(std::marker::PhantomData<T>);

impl<T> Pipe<T> {
    /// Create a new bounded channel with the specified capacity.
    ///
    /// Returns a (Sender, Receiver) pair.
    pub fn new(capacity: usize) -> (PipeSender<T>, PipeReceiver<T>) {
        let (tx, rx) = mpsc::channel(capacity);
        (PipeSender { inner: tx }, PipeReceiver { inner: rx })
    }
}

/// Sender half of a Pipe.
///
/// Can be cloned to create multiple senders.
#[derive(Clone)]
pub struct PipeSender<T> {
    inner: mpsc::Sender<T>,
}

impl<T> PipeSender<T> {
    /// Send a value asynchronously.
    ///
    /// Waits if the channel is full. Returns error if all receivers dropped.
    pub async fn send(&self, val: T) -> Result<(), SendError<T>> {
        self.inner.send(val).await
    }

    /// Try to send a value without blocking.
    ///
    /// Returns immediately with an error if the channel is full or closed.
    pub fn try_send(&self, val: T) -> Result<(), TrySendError<T>> {
        self.inner.try_send(val)
    }

    /// Check if the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Get the current capacity of the channel.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

/// Receiver half of a Pipe.
///
/// Cannot be cloned - only one receiver per channel.
pub struct PipeReceiver<T> {
    inner: mpsc::Receiver<T>,
}

impl<T> PipeReceiver<T> {
    /// Receive a value asynchronously.
    ///
    /// Returns `None` if all senders have been dropped and the channel is empty.
    pub async fn recv(&mut self) -> Option<T> {
        self.inner.recv().await
    }

    /// Try to receive a value without blocking.
    ///
    /// Returns immediately with an error if the channel is empty or closed.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }

    /// The number of messages currently buffered. A `Select` (`logos_select!`)
    /// uses this to test a receive arm's readiness *without consuming* its value,
    /// so the seeded winner-pick can read every arm's readiness before committing
    /// to exactly one.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the channel currently has no buffered messages.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Close the receiver.
    ///
    /// Prevents further values from being sent. Existing values can still be received.
    pub fn close(&mut self) {
        self.inner.close()
    }
}

// =============================================================================
// check_preemption() - The "Nanny" function for cooperative scheduling
// =============================================================================

/// Preemption threshold: yield if more than 10ms since last yield
const PREEMPTION_THRESHOLD_MS: u128 = 10;

thread_local! {
    static LAST_YIELD: RefCell<Instant> = RefCell::new(Instant::now());
}

/// Reset the preemption timer (useful for tests).
pub fn reset_preemption_timer() {
    LAST_YIELD.with(|cell| {
        *cell.borrow_mut() = Instant::now();
    });
}

/// Check if we should yield to other tasks.
///
/// This is the "Nanny" function for cooperative multitasking.
/// If more than 10ms have elapsed since the last yield point,
/// yields control via `tokio::task::yield_now()` and resets the timer.
///
/// # Usage
///
/// Insert calls to `check_preemption().await` in long-running loops
/// to ensure fair scheduling with other async tasks.
///
/// ```no_run
/// use logicaffeine_system::concurrency::check_preemption;
///
/// # fn main() {}
/// # async fn example() {
/// # fn heavy_computation(_: i32) {}
/// for i in 0..1_000_000 {
///     heavy_computation(i);
///     check_preemption().await;  // Yield if >10ms elapsed
/// }
/// # }
/// ```
pub async fn check_preemption() {
    let should_yield = LAST_YIELD.with(|cell| {
        let last = *cell.borrow();
        last.elapsed().as_millis() >= PREEMPTION_THRESHOLD_MS
    });

    if should_yield {
        // Hand control back to the scheduler: native yields the tokio runtime; the browser
        // awaits a 0-ms `setTimeout` macrotask (the only portable wasm yield).
        #[cfg(not(target_arch = "wasm32"))]
        tokio::task::yield_now().await;
        #[cfg(target_arch = "wasm32")]
        browser_timeout_ms(0).await;
        LAST_YIELD.with(|cell| {
            *cell.borrow_mut() = Instant::now();
        });
    }
}

// =============================================================================
// Cross-target runtime traits (Phase 9a / T16)
// =============================================================================
//
// The async runtime services concurrency needs — cooperative yield and a timer —
// abstracted behind traits so the SAME interpreter/driver code runs on native (tokio)
// and in the browser (wasm). Native impls are `Send` (multi-thread); the wasm twins are
// `?Send` (single-threaded browser), mirroring the `Vfs` split. `async_trait` boxes the
// returned futures, so `&dyn Yield` / `&dyn Timer` are object-safe — the interpreter holds
// them generically without knowing the target. (The generic `Spawner`/`Channel` traits fold
// in next, reusing the existing `spawn`/`Pipe` primitives.)

/// Cooperative yield. Native yields the tokio scheduler; the browser awaits a 0-ms macrotask
/// so the UI can repaint between ticks.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
pub trait Yield: Send + Sync {
    /// Yield control so other tasks (or the UI) can make progress.
    async fn yield_now(&self);
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
pub trait Yield {
    /// Yield control so other tasks (or the UI) can make progress.
    async fn yield_now(&self);
}

/// Millisecond async sleep. Native = `tokio::time::sleep`; browser = a gloo timeout.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
pub trait Timer: Send + Sync {
    /// Sleep for at least `ms` milliseconds.
    async fn sleep_ms(&self, ms: u64);
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
pub trait Timer {
    /// Sleep for at least `ms` milliseconds.
    async fn sleep_ms(&self, ms: u64);
}

/// The native cross-target runtime — tokio-backed. (The browser twin, `BrowserRuntime`,
/// lands with Phase 9b's drive loop.)
#[derive(Clone, Copy, Default, Debug)]
pub struct NativeRuntime;

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl Yield for NativeRuntime {
    async fn yield_now(&self) {
        tokio::task::yield_now().await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl Timer for NativeRuntime {
    async fn sleep_ms(&self, ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

/// Abort handle for a task spawned via [`Spawner`]. Cross-target: wraps the native tokio
/// abort handle (the wasm twin lands with `BrowserRuntime` in Phase 9b — wasm tasks are
/// detached, aborted via a cooperative cancel flag). The handle is intentionally minimal —
/// spawned tasks return `()` and communicate results back over channels ([`Pipe`]).
pub struct SpawnedTask {
    #[cfg(not(target_arch = "wasm32"))]
    inner: JoinHandle<()>,
}

impl SpawnedTask {
    /// Cancel the task at its next await point. A no-op if it already finished.
    pub fn abort(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.inner.abort();
    }

    /// Whether the task has finished (successfully or aborted).
    pub fn is_finished(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            return self.inner.is_finished();
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
}

/// Spawn a detached concurrent task. The genuine native↔browser divergence: native uses
/// `tokio::spawn` (multi-thread, the future is `Send`); the browser uses `spawn_local`
/// (single-thread, the future is `?Send`). The future is type-erased to a `BoxFuture`, so
/// the method is object-safe — the interpreter holds `&dyn Spawner` without knowing the
/// target. ([`Pipe<T>`] is already the cross-target *channel*: `tokio::sync::mpsc` is
/// runtime-agnostic and compiles on wasm, so no separate `Channel` trait is needed.)
#[cfg(not(target_arch = "wasm32"))]
pub trait Spawner: Send + Sync {
    /// Spawn `fut` as a detached task and return its abort handle.
    fn spawn_task(&self, fut: Pin<Box<dyn Future<Output = ()> + Send>>) -> SpawnedTask;
}

#[cfg(target_arch = "wasm32")]
pub trait Spawner {
    /// Spawn `fut` as a detached task and return its abort handle.
    fn spawn_task(&self, fut: Pin<Box<dyn Future<Output = ()>>>) -> SpawnedTask;
}

#[cfg(not(target_arch = "wasm32"))]
impl Spawner for NativeRuntime {
    fn spawn_task(&self, fut: Pin<Box<dyn Future<Output = ()> + Send>>) -> SpawnedTask {
        SpawnedTask { inner: tokio::spawn(fut) }
    }
}

// -----------------------------------------------------------------------------
// Browser (wasm) runtime — single-threaded, `?Send`. The channel (`Pipe`) is shared
// (tokio's runtime-agnostic `sync::mpsc`); only the runtime-tied services are wasm-specific:
// `setTimeout` for Timer/Yield (no extra dep) and `spawn_local` for Spawner. Compile-verified
// for `wasm32`; runtime behavior is exercised by `wasm-pack test --headless` (Phase 9b).
// -----------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    // The GLOBAL `setTimeout` — present in BOTH the browser and node (the wasm test host),
    // unlike `window.setTimeout` which is browser-only. Binding the global keeps the timer
    // portable across every wasm host and lets the runtime tests run under node.
    #[wasm_bindgen(js_name = setTimeout)]
    fn global_set_timeout(handler: &js_sys::Function, timeout_ms: i32) -> f64;
}

/// Await a `setTimeout(ms)` macrotask — the portable async sleep, and (with `ms = 0`) the
/// cooperative-yield macrotask that lets the UI repaint between ticks.
#[cfg(target_arch = "wasm32")]
async fn browser_timeout_ms(ms: u64) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        global_set_timeout(&resolve, ms as i32);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The browser cross-target runtime — single-threaded, backed by the JS event loop.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Default, Debug)]
pub struct BrowserRuntime;

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl Yield for BrowserRuntime {
    async fn yield_now(&self) {
        browser_timeout_ms(0).await;
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl Timer for BrowserRuntime {
    async fn sleep_ms(&self, ms: u64) {
        browser_timeout_ms(ms).await;
    }
}

#[cfg(target_arch = "wasm32")]
impl Spawner for BrowserRuntime {
    fn spawn_task(&self, fut: Pin<Box<dyn Future<Output = ()>>>) -> SpawnedTask {
        wasm_bindgen_futures::spawn_local(fut);
        SpawnedTask {}
    }
}

// =============================================================================
// Tests - TDD: These define the expected behavior
// =============================================================================

// Native-only: these `#[tokio::test]`s need tokio's runtime (`rt`/`macros`), which the wasm
// build (sync-only tokio) doesn't have. The browser runtime is covered by
// `tests/wasm_concurrency.rs` under `wasm-pack`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::time::Duration;

    // ---- Cross-target runtime traits (T16) ----

    #[tokio::test]
    async fn native_runtime_yield_completes() {
        // Yielding must return (not hang) and let the runtime keep going.
        NativeRuntime.yield_now().await;
    }

    #[tokio::test]
    async fn native_runtime_timer_sleeps_at_least_requested() {
        let start = std::time::Instant::now();
        NativeRuntime.sleep_ms(5).await;
        assert!(
            start.elapsed() >= Duration::from_millis(4),
            "timer must sleep ~the requested ms, slept {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn native_runtime_timer_zero_ms_returns_promptly() {
        // Edge case: a 0 ms sleep must complete (a degenerate but legal request).
        let start = std::time::Instant::now();
        NativeRuntime.sleep_ms(0).await;
        assert!(start.elapsed() < Duration::from_millis(50), "0 ms sleep must return promptly");
    }

    #[tokio::test]
    async fn cross_target_traits_are_object_safe() {
        // Edge case / the whole point: the interpreter must hold the runtime as `&dyn Yield`
        // / `&dyn Timer` without knowing the target. `async_trait` boxing makes the async
        // methods object-safe — if this stops compiling, the abstraction has been broken.
        let rt = NativeRuntime;
        let y: &dyn Yield = &rt;
        let t: &dyn Timer = &rt;
        y.yield_now().await;
        t.sleep_ms(1).await;
    }

    #[tokio::test]
    async fn native_runtime_is_send_sync_for_multithread_spawn() {
        // Edge case: the native runtime must be `Send + Sync` so a multi-thread executor can
        // share it across worker threads (the work-stealing driver needs this).
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&NativeRuntime);
    }

    #[tokio::test]
    async fn native_runtime_spawn_runs_a_task_to_completion() {
        // The spawned task runs concurrently and delivers its result over a channel — the
        // canonical "spawn + channel" round-trip the abstraction exists to support.
        let (tx, mut rx) = Pipe::<i64>::new(1);
        let task = NativeRuntime.spawn_task(Box::pin(async move {
            tx.send(42).await.unwrap();
        }));
        assert_eq!(rx.recv().await, Some(42));
        let _ = task;
    }

    #[tokio::test]
    async fn spawner_is_object_safe() {
        // Edge case / the point: the interpreter holds `&dyn Spawner` without the concrete type.
        let rt = NativeRuntime;
        let spawner: &dyn Spawner = &rt;
        let (tx, mut rx) = Pipe::<i64>::new(1);
        spawner.spawn_task(Box::pin(async move {
            tx.send(7).await.unwrap();
        }));
        assert_eq!(rx.recv().await, Some(7));
    }

    #[tokio::test]
    async fn native_runtime_spawn_abort_prevents_delivery() {
        // Edge case: aborting cancels the task at its next await point — before it can send,
        // so its sender drops and the channel closes (recv yields None, never the value).
        let (tx, mut rx) = Pipe::<i64>::new(1);
        let task = NativeRuntime.spawn_task(Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            let _ = tx.send(1).await;
        }));
        task.abort();
        assert_eq!(rx.recv().await, None, "an aborted task must not deliver its value");
    }


    // -------------------------------------------------------------------------
    // TaskHandle tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_task_handle_creation_and_completion() {
        let handle = spawn(async { 42 });

        // Task should complete quickly
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(handle.is_finished());
    }

    #[tokio::test]
    async fn test_task_handle_await_result() {
        let handle = spawn(async { 42 });
        let result = handle.await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_task_handle_is_finished_initially_false() {
        let handle = spawn(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            42
        });

        // Should not be finished immediately
        assert!(!handle.is_finished());

        // Cleanup
        handle.abort();
    }

    #[tokio::test]
    async fn test_task_handle_abort() {
        let handle = spawn(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            42
        });

        handle.abort();

        // Wait a bit for abort to take effect
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(handle.is_finished());

        // Awaiting should return JoinError
        let result = handle.await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_returns_task_handle() {
        let handle: TaskHandle<i32> = spawn(async { 1 + 1 });
        let result = handle.await.unwrap();
        assert_eq!(result, 2);
    }

    #[tokio::test]
    async fn test_spawn_with_captured_values() {
        let x = 10;
        let y = 20;
        let handle = spawn(async move { x + y });
        let result = handle.await.unwrap();
        assert_eq!(result, 30);
    }

    #[tokio::test]
    async fn test_spawn_with_complex_return_type() {
        let handle = spawn(async { vec![1, 2, 3] });
        let result = handle.await.unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    // -------------------------------------------------------------------------
    // Pipe tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_pipe_send_recv() {
        let (tx, mut rx) = Pipe::<i32>::new(16);

        tx.send(42).await.unwrap();
        let received = rx.recv().await;

        assert_eq!(received, Some(42));
    }

    #[tokio::test]
    async fn test_pipe_recv_none_when_closed() {
        let (tx, mut rx) = Pipe::<i32>::new(16);

        drop(tx);

        let received = rx.recv().await;
        assert_eq!(received, None);
    }

    #[tokio::test]
    async fn test_pipe_try_send_success() {
        let (tx, mut rx) = Pipe::<i32>::new(16);

        assert!(tx.try_send(42).is_ok());
        assert_eq!(rx.recv().await, Some(42));
    }

    #[tokio::test]
    async fn test_pipe_try_send_full() {
        let (tx, _rx) = Pipe::<i32>::new(1);

        assert!(tx.try_send(1).is_ok());
        // Channel is now full
        assert!(matches!(tx.try_send(2), Err(TrySendError::Full(_))));
    }

    #[tokio::test]
    async fn test_pipe_try_recv_empty() {
        let (_tx, mut rx) = Pipe::<i32>::new(16);

        // Channel is empty
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn test_pipe_sender_clone() {
        let (tx, mut rx) = Pipe::<i32>::new(16);
        let tx2 = tx.clone();

        tx.send(1).await.unwrap();
        tx2.send(2).await.unwrap();

        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
    }

    #[tokio::test]
    async fn test_pipe_is_closed() {
        let (tx, rx) = Pipe::<i32>::new(16);

        assert!(!tx.is_closed());
        drop(rx);
        assert!(tx.is_closed());
    }

    #[tokio::test]
    async fn test_pipe_receiver_close() {
        let (tx, mut rx) = Pipe::<i32>::new(16);

        rx.close();

        // Sender should now fail
        assert!(tx.send(42).await.is_err());
    }

    // -------------------------------------------------------------------------
    // check_preemption tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_check_preemption_no_yield_initially() {
        // Reset timer
        reset_preemption_timer();

        // Should not yield if called immediately
        let start = Instant::now();
        check_preemption().await;
        let elapsed = start.elapsed();

        // Should be nearly instant (no actual yield)
        assert!(elapsed.as_millis() < 5);
    }

    #[tokio::test]
    async fn test_check_preemption_yields_after_threshold() {
        // Reset timer
        reset_preemption_timer();

        // Simulate 15ms of computation
        std::thread::sleep(Duration::from_millis(15));

        // This should yield
        check_preemption().await;

        // Timer should be reset - next call should not yield
        let start = Instant::now();
        check_preemption().await;
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 5);
    }

    // -------------------------------------------------------------------------
    // Integration tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_spawn_with_pipe_communication() {
        let (tx, mut rx) = Pipe::<String>::new(16);

        let producer = spawn(async move {
            for i in 0..5 {
                tx.send(format!("message {}", i)).await.unwrap();
                check_preemption().await;
            }
        });

        let mut received = Vec::new();
        while let Some(msg) = rx.recv().await {
            received.push(msg);
        }

        producer.await.unwrap();
        assert_eq!(received.len(), 5);
    }

    #[tokio::test]
    async fn test_multiple_producers_single_consumer() {
        let (tx, mut rx) = Pipe::<i32>::new(32);

        let tx1 = tx.clone();
        let tx2 = tx.clone();
        drop(tx); // Drop original

        let p1 = spawn(async move {
            for i in 0..10 {
                tx1.send(i).await.unwrap();
            }
        });

        let p2 = spawn(async move {
            for i in 10..20 {
                tx2.send(i).await.unwrap();
            }
        });

        // Wait for producers
        p1.await.unwrap();
        p2.await.unwrap();

        // Collect all messages
        let mut values = Vec::new();
        while let Some(v) = rx.recv().await {
            values.push(v);
        }

        values.sort();
        assert_eq!(values, (0..20).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn test_task_abort_with_pipe() {
        let (tx, mut rx) = Pipe::<i32>::new(16);

        let producer = spawn(async move {
            for i in 0.. {
                if tx.send(i).await.is_err() {
                    break;
                }
                check_preemption().await;
            }
        });

        // Receive a few messages
        for _ in 0..5 {
            rx.recv().await;
        }

        // Abort the producer
        producer.abort();

        // Close receiver - this will cause sender to fail
        rx.close();

        // Ensure task was aborted
        let result = producer.await;
        assert!(result.is_err());
    }
}
