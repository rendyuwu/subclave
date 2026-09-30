//! Poison-safe lock acquisition for the vault's shared, unsupervised locks.
//!
//! A `Mutex` is *poisoned* when a thread panics while holding the guard. The
//! default `.lock().unwrap()` then propagates that panic to every later
//! acquirer, which in Subclave would end the background tick thread or take
//! the app's working state down with it.
//!
//! [`LockExt::lock_or_recover`] recovers the inner guard instead
//! (`unwrap_or_else(|e| e.into_inner())`). That is the right call where the
//! protected value is replaced wholesale or simply appended to, so a
//! half-finished mutation from a panicking thread is at worst a few stale
//! bytes rather than a dead process: the process-global clipboard handle and
//! the vault state mutexes (payload, pending seal, save lock, the auto-lock
//! flag) are all that shape.
//!
//! Use this on those long-lived locks. Where a poisoned lock is a genuine
//! "this invariant is broken, fail fast" signal, keep the plain `.lock()?` /
//! `.unwrap()` instead.
//!
//! Only a `Mutex` flavour exists today because those locks are all `Mutex`;
//! add an `RwLock` equivalent here if that ever changes.

use std::sync::{Mutex, MutexGuard};

pub trait LockExt<T> {
    /// Acquire the mutex, recovering the guard if the lock was poisoned by a
    /// panicking thread rather than re-panicking.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}
