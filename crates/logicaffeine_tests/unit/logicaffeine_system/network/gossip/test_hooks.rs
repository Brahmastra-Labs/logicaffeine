use once_cell::sync::Lazy;
use std::sync::Mutex;

pub struct MessageLog {
    pub received: Vec<(String, Vec<u8>)>,
}

static LOG: Lazy<Mutex<MessageLog>> = Lazy::new(|| {
    Mutex::new(MessageLog {
        received: Vec::new(),
    })
});

pub fn log_received(topic: &str, data: &[u8]) {
    if let Ok(mut log) = LOG.lock() {
        log.received.push((topic.to_string(), data.to_vec()));
    }
}

pub fn get_received() -> Vec<(String, Vec<u8>)> {
    LOG.lock().map(|l| l.received.clone()).unwrap_or_default()
}

pub fn clear_log() {
    if let Ok(mut log) = LOG.lock() {
        log.received.clear();
    }
}

pub fn received_count() -> usize {
    LOG.lock().map(|l| l.received.len()).unwrap_or(0)
}
