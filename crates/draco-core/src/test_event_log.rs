// Test-only event log used to capture corner-table operations (set/map) during
// encoder and decoder CornerTable construction for targeted tests.

use std::sync::{Mutex, OnceLock};

// Test event logger used by both unit and integration tests. Lightweight and
// only used for diagnostics of encoder/decoder ordering. Functions are public
// so integration tests (tests/) can access them.
static LOG: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

pub fn init() {
    LOG.get_or_init(|| Mutex::new(Vec::new()));
}

pub fn clear() {
    if let Some(m) = LOG.get() {
        m.lock().unwrap().clear();
    }
}

pub fn enabled() -> bool {
    LOG.get().is_some()
}

pub fn record_event(s: String) {
    if let Some(m) = LOG.get() {
        m.lock().unwrap().push(s);
    }
}

pub fn take_events() -> Vec<String> {
    if let Some(m) = LOG.get() {
        let mut g = m.lock().unwrap();
        std::mem::take(&mut *g)
    } else {
        Vec::new()
    }
}
