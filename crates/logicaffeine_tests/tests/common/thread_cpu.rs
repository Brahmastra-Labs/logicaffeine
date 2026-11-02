//! The calling thread's CPU time: the time it ran on a core, not the time it waited for one. A timing
//! lock whose claim must not depend on how busy the machine is (the fast gate keeps every core busy) reads
//! this instead of the wall clock. It measures only work done on the calling thread.

use std::time::Duration;

/// CPU time the calling thread has consumed so far.
#[cfg(unix)]
pub fn thread_cpu_time() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID) failed");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Without a per-thread CPU clock, elapsed time since the first call: the wall clock these platforms'
/// runs (never the parallel gate) are measured by.
#[cfg(not(unix))]
pub fn thread_cpu_time() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}
