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
    assert!(
        start.elapsed() < Duration::from_millis(50),
        "0 ms sleep must return promptly"
    );
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
    assert_eq!(
        rx.recv().await,
        None,
        "an aborted task must not deliver its value"
    );
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
