use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(usize);

impl WindowId {
    /// Generates a globally unique window identifier.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(1);
        WindowId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
    
    /// Returns the inner usize value.
    pub fn as_usize(&self) -> usize {
        self.0
    }
}
