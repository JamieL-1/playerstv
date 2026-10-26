//! Locking that survives a panicked thread.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, even if a thread panicked while holding it. Everything
/// playerstv keeps behind a mutex (status snapshots, caches, child process
/// handles) is valid after any single update, so one panicked thread
/// mustn't take every other thread down with it.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_mutex_still_locks() {
        let m = Mutex::new(1);
        let _ = std::panic::catch_unwind(|| {
            let _guard = m.lock().expect("not poisoned yet");
            panic!("poison it");
        });
        assert!(m.is_poisoned());
        *lock(&m) += 1;
        assert_eq!(*lock(&m), 2);
    }
}
