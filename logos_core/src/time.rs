//! Time module for LOGOS standard library.

use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::thread;

/// Get current time as milliseconds since Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Sleep for the given number of milliseconds.
pub fn sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}
