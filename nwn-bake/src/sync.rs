// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Locks that outlive a panic, and panics turned into errors.
//!
//! A program that keeps nwn-bake loaded (`serve`, a server that links the library) runs many requests in one process, so one request
//! that panics while it holds a cache's lock must not fail every request after it. The caches hold whole entries (a value is put in or
//! not, never half), so what a poisoned lock guards is still sound, and [`Recover`] uses it as it is. A batch runs each area's work
//! through [`caught`], so a panic fails that area and not the batch (`docs/area-iso.md`, "Batches and the cache").

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// A [`Mutex`] used whether or not a thread panicked while holding it.
pub(crate) trait Recover<T> {
    /// The lock, poisoned or not.
    fn locked(&self) -> MutexGuard<'_, T>;
    /// The value, poisoned or not.
    fn take(self) -> T;
    /// The value through `&mut`, poisoned or not.
    fn inner_mut(&mut self) -> &mut T;
}

impl<T> Recover<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(self) -> T {
        self.into_inner().unwrap_or_else(PoisonError::into_inner)
    }

    fn inner_mut(&mut self) -> &mut T {
        self.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `f()`, or an error saying it panicked and with what message.
pub(crate) fn caught<R>(f: impl FnOnce() -> R) -> anyhow::Result<R> {
    // (AssertUnwindSafe: what `f` shares is behind locks that recover, or is dropped with the failed area.)
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| anyhow::anyhow!("panicked: {}", panic_message(p.as_ref())))
}

/// The text of a panic's payload (`panic!` gives a `&str` or a `String`).
fn panic_message(p: &(dyn Any + Send)) -> String {
    p.downcast_ref::<&str>().map(ToString::to_string).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "no message".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_is_still_used() {
        let m = Mutex::new(1);
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _held = m.lock().unwrap();
                panic!("while holding it");
            })
            .join()
        });
        assert!(m.is_poisoned());
        *m.locked() += 1;
        assert_eq!(m.take(), 2);
    }

    #[test]
    fn a_panic_is_an_error_with_its_message() {
        assert_eq!(caught(|| 3).unwrap(), 3);
        let e = caught(|| -> u8 { panic!("area {} is broken", "x") }).unwrap_err();
        assert_eq!(e.to_string(), "panicked: area x is broken");
    }
}
