//! The mock clock: all lifetimes are measured on it, never on the wall clock, so that tests
//! can move an hour forward instantly and deterministically.

use std::time::Duration;

/// Time elapsed since the server started. It only moves when `advance` is called.
#[derive(Debug, Default)]
pub(crate) struct Clock {
    elapsed: Duration,
}

impl Clock {
    pub(crate) fn now(&self) -> Duration {
        self.elapsed
    }

    pub(crate) fn advance(&mut self, by: Duration) {
        // Saturating: an absurd advance must not panic a test, it just pins the clock at the end
        // of time.
        self.elapsed = self.elapsed.saturating_add(by);
    }
}
