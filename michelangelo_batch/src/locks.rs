//! Cross-process locks built on the Rust standard library file-lock API
//! (`std::fs::File::try_lock`, stable since 1.89; LockFileEx on Windows).
//!
//! Why file locks and not an in-memory mutex: the ADR requires real
//! cross-process ownership for the heavy worker slot and fencing against
//! late/orphaned attempts. OS file locks are released by the kernel when the
//! owning process dies (crash-safe), work across independent processes, and
//! need no external service or Resource Manager server.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Name of the global heavy-worker admission slot file (limit = 1 in this PoC).
pub const SLOT_LOCK_FILE: &str = "heavy_worker.slot.lock";
/// Name of the per-attempt fencing lock file inside an attempt directory.
pub const ATTEMPT_LOCK_FILE: &str = "attempt.lock";

#[derive(Debug)]
pub enum LockError {
    Busy { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy { path } => write!(f, "lock busy: {}", path.display()),
            LockError::Io { path, source } => {
                write!(f, "lock io error on {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LockError {}

/// An exclusively held file lock. Released on `unlock()`/drop; the OS also
/// releases it if the process dies, which is the crash-safety property we rely
/// on for admission and fencing.
#[derive(Debug)]
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    /// Acquire an exclusive lock immediately, failing with [`LockError::Busy`]
    /// if another process holds it.
    pub fn acquire(path: &Path) -> Result<FileLock, LockError> {
        let file = Self::open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(FileLock {
                file,
                path: path.to_path_buf(),
            }),
            Err(_) => Err(LockError::Busy {
                path: path.to_path_buf(),
            }),
        }
    }

    /// Poll until the lock is acquired or `wait_ms` elapses.
    pub fn acquire_with_wait(path: &Path, wait_ms: u64) -> Result<FileLock, LockError> {
        let start = Instant::now();
        loop {
            match Self::acquire(path) {
                Ok(lock) => return Ok(lock),
                Err(LockError::Busy { .. }) => {
                    if start.elapsed().as_millis() as u64 >= wait_ms {
                        return Err(LockError::Busy {
                            path: path.to_path_buf(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// True if some other process currently holds this lock. Never keeps the
    /// lock: a transiently acquired lock is released immediately.
    pub fn is_locked_elsewhere(path: &Path) -> Result<bool, LockError> {
        let file = Self::open(path)?;
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                Ok(false)
            }
            Err(_) => Ok(true),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(path: &Path) -> Result<File, LockError> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| LockError::Io {
                path: path.to_path_buf(),
                source,
            })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Path of the admission slot file for a runs root.
pub fn slot_lock_path(runs_root: &Path) -> PathBuf {
    runs_root.join(SLOT_LOCK_FILE)
}

/// Path of the fencing lock file for an attempt directory.
pub fn attempt_lock_path(attempt_dir: &Path) -> PathBuf {
    attempt_dir.join(ATTEMPT_LOCK_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_within_process_and_released_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x.lock");
        let l1 = FileLock::acquire(&path).unwrap();
        assert!(matches!(
            FileLock::acquire(&path),
            Err(LockError::Busy { .. })
        ));
        assert!(FileLock::is_locked_elsewhere(&path).unwrap());
        drop(l1);
        assert!(!FileLock::is_locked_elsewhere(&path).unwrap());
        // Re-acquirable after release.
        let _l2 = FileLock::acquire(&path).unwrap();
    }

    #[test]
    fn acquire_with_wait_times_out_when_busy() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("y.lock");
        let _holder = FileLock::acquire(&path).unwrap();
        let started = Instant::now();
        let err = FileLock::acquire_with_wait(&path, 120).unwrap_err();
        assert!(matches!(err, LockError::Busy { .. }));
        assert!(started.elapsed() >= Duration::from_millis(100));
    }
}
