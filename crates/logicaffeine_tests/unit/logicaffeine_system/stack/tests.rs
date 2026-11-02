use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Recurse until the calls hold `want` bytes of stack below `top`, each call keeping a 1 KiB array the compiler cannot
/// elide; the number of calls. Measured in addresses, so it uses `want` bytes whatever a frame costs in this build.
#[inline(never)]
fn descend(top: usize, want: usize) -> usize {
    let frame = std::hint::black_box([0u8; 1024]);
    if top - frame.as_ptr() as usize >= want {
        return 1;
    }
    1 + descend(top, want) + usize::from(frame[1023])
}

static RAN_ON: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);
static DESCENT: AtomicUsize = AtomicUsize::new(0);
static THREADS_INSIDE: AtomicUsize = AtomicUsize::new(0);

/// The threads this process has now, from `/proc/self/status`.
#[cfg(target_os = "linux")]
fn threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|n| n.trim().parse().ok())
        .expect("a Threads: line")
}

#[test]
fn main_runs_on_the_calling_thread_with_room_for_deep_recursion() {
    fn main() {
        *RAN_ON.lock().unwrap() = Some(std::thread::current().id());
        let top = std::hint::black_box(0u8);
        // 48 MiB of calls: six times a default main stack, and far past a test thread's.
        DESCENT.store(descend(&top as *const u8 as usize, 48 << 20), Ordering::SeqCst);
    }
    run_main(main);
    assert_eq!(*RAN_ON.lock().unwrap(), Some(std::thread::current().id()));
    assert!(DESCENT.load(Ordering::SeqCst) > 1000);
}

#[cfg(target_os = "linux")]
#[test]
fn main_starts_no_thread() {
    fn main() {
        THREADS_INSIDE.store(threads(), Ordering::SeqCst);
    }
    let before = threads();
    run_main(main);
    assert_eq!(
        THREADS_INSIDE.load(Ordering::SeqCst),
        before,
        "a second thread puts every malloc and free on the C library's locked path"
    );
}

#[test]
fn the_stack_is_mapped_between_two_guard_pages() {
    let stack = mapped::Stack::map(MAIN_STACK_BYTES).expect("the system maps the stack");
    let [(low_start, low_end), (high_start, high_end)] = stack.guards();
    assert_eq!(low_end, stack.base() as usize);
    assert_eq!(high_start, stack.base() as usize + stack.size());
    assert!(stack.size() >= MAIN_STACK_BYTES);
    assert_eq!(low_end - low_start, high_end - high_start);
    assert!(low_end > low_start);
}
