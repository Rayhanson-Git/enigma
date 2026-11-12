//! The caretaker lock (03 §10).
use crate::error::{Error, Kind, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// Held while alive; dropping closes the file, which releases the lock.
pub struct CaretakerLock(#[allow(dead_code)] File);

pub fn acquire(state_dir: &Path) -> Result<CaretakerLock> {
    let path = state_dir.join("caretaker.lock");
    let f = OpenOptions::new().read(true).append(true).create(true).open(&path).map_err(|e| Error::io(&e, &path))?;
    match f.try_lock() {
        Ok(()) => Ok(CaretakerLock(f)),
        Err(TryLockError::WouldBlock) => Err(Error::new(
            Kind::LockBusy,
            format!(
                "this vault is being written by another maintenance run (Librarian, indexer, organizer or consolidation); retry once it has finished ({})",
                std::path::absolute(&path).unwrap_or(path).display()
            ),
        )),
        Err(TryLockError::Error(e)) => Err(Error::io(&e, &path)),
    }
}
