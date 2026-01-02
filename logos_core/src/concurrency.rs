//! Phase 54: Go-like Concurrency Primitives
//!
//! This module provides green thread primitives for LOGOS:
//! - `TaskHandle<T>`: Wrapper around tokio::task::JoinHandle with abort/completion tracking
//! - `Pipe<T>`: Bounded channel with sender/receiver split (Go-like channels)
//! - `check_preemption()`: Cooperative yielding for long-running computations
//! - `spawn()`: Ergonomic task spawning

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

// Re-export error types for ergonomic API
pub use tokio::sync::mpsc::error::{SendError, TryRecvError, TrySendError};
pub use tokio::task::JoinError;

// =============================================================================
// TaskHandle<T> - Wrapper around JoinHandle with abort/completion tracking
// =============================================================================

/// Handle to a spawned async task.
///
/// Wraps `tokio::task::JoinHandle<T>` with a LOGOS-friendly API.
///
/// # Example
/// ```ignore
/// let handle = spawn(async { expensive_computation() });
/// // Do other work...
/// if handle.is_finished() {
///     let result = handle.await?;
/// }
/// ```
pub struct TaskHandle<T> {
    inner: JoinHandle<T>,
}

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
/// ```ignore
/// let handle = spawn(async {
///     expensive_computation().await
/// });
/// let result = handle.await?;
/// ```
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
/// ```ignore
/// let (tx, rx) = Pipe::<String>::new(16);
///
/// spawn(async move {
///     tx.send("hello".to_string()).await.unwrap();
/// });
///
/// let msg = rx.recv().await;
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
/// ```ignore
/// for i in 0..1_000_000 {
///     heavy_computation(i);
///     check_preemption().await;  // Yield if >10ms elapsed
/// }
/// ```
pub async fn check_preemption() {
    let should_yield = LAST_YIELD.with(|cell| {
        let last = *cell.borrow();
        last.elapsed().as_millis() >= PREEMPTION_THRESHOLD_MS
    });

    if should_yield {
        tokio::task::yield_now().await;
        LAST_YIELD.with(|cell| {
            *cell.borrow_mut() = Instant::now();
        });
    }
}

// =============================================================================
// Tests - TDD: These define the expected behavior
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
