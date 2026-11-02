use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

pub struct MeshTestControl {
    pub pause_publish: AtomicBool,
    pub pause_receive: AtomicBool,
    pub retry_count: AtomicU32,
}

static CONTROL: OnceLock<MeshTestControl> = OnceLock::new();

pub fn get() -> &'static MeshTestControl {
    CONTROL.get_or_init(|| MeshTestControl {
        pause_publish: AtomicBool::new(false),
        pause_receive: AtomicBool::new(false),
        retry_count: AtomicU32::new(0),
    })
}

pub fn pause_network() {
    get().pause_publish.store(true, Ordering::SeqCst);
}

pub fn resume_network() {
    get().pause_publish.store(false, Ordering::SeqCst);
}

pub fn is_paused() -> bool {
    get().pause_publish.load(Ordering::Relaxed)
}

pub fn increment_retry() {
    get().retry_count.fetch_add(1, Ordering::Relaxed);
}

pub fn get_retry_count() -> u32 {
    get().retry_count.load(Ordering::Relaxed)
}

pub fn reset_retry_count() {
    get().retry_count.store(0, Ordering::Relaxed);
}
