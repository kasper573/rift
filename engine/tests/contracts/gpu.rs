use std::sync::{Mutex, MutexGuard, PoisonError};

/// The GPU to one contract at a time, held for the whole contract. The Vulkan loader crashes
/// naming an object on one thread while a device is made or dropped on another, as contracts
/// running side by side with renderers of their own would.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static GPU: Mutex<()> = Mutex::new(());
    GPU.lock().unwrap_or_else(PoisonError::into_inner)
}
