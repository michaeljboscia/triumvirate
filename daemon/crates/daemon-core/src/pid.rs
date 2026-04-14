//! PID file management for the daemon.
//!
//! Creates ~/.triumvirate/daemon.pid with flock-based locking to prevent
//! multiple daemon instances. On crash, the kernel releases the lock
//! automatically, allowing the next startup to detect and overwrite the
//! stale file.
//!
//! Pantheon reads this file to determine whether the daemon is running
//! and to send signals (SIGTERM for graceful shutdown). Before sending
//! any signal, callers should verify the PID matches the triumvirate
//! binary via libproc to protect against PID recycling.
//!
//! FEAT-015 (REQ-019)

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Acquired PID file — holds an exclusive advisory lock for the daemon's lifetime.
///
/// The lock is released automatically when this struct is dropped (i.e. when
/// the daemon exits cleanly OR crashes — the kernel releases file locks on
/// process termination).
#[derive(Debug)]
pub struct PidFile {
    /// The open file handle — holding it keeps the lock alive.
    _file: File,
    /// Path to the PID file for cleanup/debug purposes.
    path: PathBuf,
}

impl PidFile {
    /// Acquire the daemon PID file at `root/daemon.pid`.
    ///
    /// Creates the file if it doesn't exist, acquires an exclusive non-blocking
    /// advisory lock via `flock()`, truncates the file, and writes the current
    /// process's PID.
    ///
    /// Returns an error if another process already holds the lock (i.e. another
    /// daemon instance is running). If the file exists but the lock is released
    /// (the previous daemon crashed), this succeeds — that's the "stale PID
    /// recovery" path.
    #[allow(clippy::useless_conversion)] // libc constants have different widths on different targets
    pub fn acquire(root: &Path) -> Result<Self> {
        let path = root.join("daemon.pid");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;

        // Acquire exclusive non-blocking advisory lock via libc::flock.
        // LOCK_EX (2) | LOCK_NB (4) = 6. If another process holds the lock,
        // this returns -1 with errno EWOULDBLOCK.
        let fd = file.as_raw_fd();
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // Read the existing PID for a helpful error message
            let existing_pid = Self::read_pid_from_path(&path).unwrap_or(0);
            anyhow::bail!(
                "another daemon instance is running (PID {}, lock held on {}): {}",
                existing_pid,
                path.display(),
                err
            );
        }

        // Lock acquired. Write our PID.
        let mut file = file;
        file.set_len(0)
            .with_context(|| format!("failed to truncate {}", path.display()))?;
        file.seek(SeekFrom::Start(0))?;
        let pid = std::process::id();
        writeln!(file, "{pid}")
            .with_context(|| format!("failed to write pid to {}", path.display()))?;
        file.sync_all()?;

        info!(pid = pid, path = %path.display(), "acquired daemon pid file");
        Ok(Self { _file: file, path })
    }

    /// Read a PID from a pid file without acquiring the lock.
    ///
    /// Used by Pantheon to check if a daemon is "supposedly" running before
    /// trying to connect. Callers should verify the PID is actually alive and
    /// matches the triumvirate binary via `libproc` before sending signals.
    pub fn read_pid_from_path(path: &Path) -> Result<u32> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let pid: u32 = contents
            .trim()
            .parse()
            .with_context(|| format!("invalid pid in {}: {:?}", path.display(), contents))?;
        Ok(pid)
    }

    /// Check if the daemon PID file exists at the canonical path.
    ///
    /// Does NOT verify the PID is alive or that the lock is held. This is
    /// a quick existence check — callers should use `read_pid_from_path`
    /// + kill(pid, 0) to verify liveness.
    pub fn path_at(root: &Path) -> PathBuf {
        root.join("daemon.pid")
    }

    /// Return the path of this active pid file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        // The kernel releases the flock automatically when _file is dropped.
        // We could also delete the file here, but leaving it means Pantheon
        // can read the (now-stale) PID to show "last daemon was PID N" — and
        // the next daemon startup will detect the stale lock and overwrite.
        if let Err(err) = std::fs::remove_file(&self.path) {
            warn!(error = %err, path = %self.path.display(), "failed to remove pid file on drop");
        } else {
            info!(path = %self.path.display(), "released daemon pid file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("triumvirate-pid-test-{nanos}"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn acquire_creates_file_with_current_pid() {
        let root = unique_test_root();
        let pid_file = PidFile::acquire(&root).expect("first acquire should succeed");
        let path = root.join("daemon.pid");

        assert!(path.exists());
        let pid = PidFile::read_pid_from_path(&path).unwrap();
        assert_eq!(pid, std::process::id());

        drop(pid_file);
        // After drop, the file should be removed
        assert!(!path.exists(), "pid file should be removed on drop");
    }

    #[test]
    fn second_acquire_blocked_while_first_held() {
        let root = unique_test_root();
        let first = PidFile::acquire(&root).expect("first acquire should succeed");

        // Second acquire in the same process — flock advisory locks are
        // per-file-descriptor AND per-open-file-description. Since we open
        // a new fd for the second attempt, flock DOES block us.
        let result = PidFile::acquire(&root);
        assert!(
            result.is_err(),
            "second acquire should fail while first is held"
        );
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("another daemon instance"),
            "error should mention another instance: {err_msg}"
        );

        drop(first);

        // After first is released, second acquire should succeed
        let second = PidFile::acquire(&root).expect("should succeed after first released");
        drop(second);
    }

    #[test]
    fn path_at_returns_canonical_location() {
        let root = PathBuf::from("/tmp/fake-root");
        let path = PidFile::path_at(&root);
        assert_eq!(path, PathBuf::from("/tmp/fake-root/daemon.pid"));
    }

    #[test]
    fn read_pid_from_path_handles_trailing_newline() {
        let root = unique_test_root();
        let path = root.join("daemon.pid");
        std::fs::write(&path, "12345\n").unwrap();
        assert_eq!(PidFile::read_pid_from_path(&path).unwrap(), 12345);

        std::fs::write(&path, "67890").unwrap();
        assert_eq!(PidFile::read_pid_from_path(&path).unwrap(), 67890);
    }

    #[test]
    fn read_pid_from_path_rejects_garbage() {
        let root = unique_test_root();
        let path = root.join("daemon.pid");
        std::fs::write(&path, "not-a-number").unwrap();
        assert!(PidFile::read_pid_from_path(&path).is_err());
    }
}
