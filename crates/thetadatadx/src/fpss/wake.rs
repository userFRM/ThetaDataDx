//! Self-pipe wake-up FD primitive.
//!
//! Lets a producer signal an event-loop reader (`epoll` / `kqueue`)
//! when new work is available, coalesced via a `signaled`
//! `AtomicBool`: at most one byte is in flight in the pipe at any
//! time regardless of how many `signal()` calls preceded it. The
//! reader clears the bool with [`WakeFd::rearm`] before draining
//! the pipe; on the producer side [`WakeFd::signal`] no-ops if a
//! byte is already in flight. The write-end FD is closed by
//! [`WakeFd::drop`]; the read-end is owned by the caller.
//!
//! Crate-internal primitive. No public surface depends on it today;
//! a future async-friendly streaming surface can layer on top
//! without re-implementing the coalesce + close semantics.

use std::sync::atomic::{AtomicBool, Ordering};

// `RawFd` is `i32` everywhere POSIX. On non-Unix targets we stub the
// type at `i32` so the struct definition stays portable while the
// `signal()` / `rearm()` paths bail out early via the `#[cfg(unix)]`
// implementations below.
#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(not(unix))]
type RawFd = i32;

/// Coalesced wake-up signal backed by a self-pipe write FD.
///
/// Cheap to clone (it isn't `Clone`; consumers share via `Arc<WakeFd>`),
/// holds a single `AtomicBool` and a `RawFd`. Constructing a `WakeFd`
/// requires a write-end file descriptor the caller already owns; the
/// [`Drop`] impl closes it.
#[derive(Debug)]
pub struct WakeFd {
    /// Write-end of the self-pipe. Closed on `Drop`. `-1` on platforms
    /// that do not support the wake mechanism (e.g. Windows) — every
    /// `signal()` short-circuits and the SDK falls back to the timeout
    /// poll path on the iterator side.
    write_fd: RawFd,
    /// `true` while a wake byte is in the pipe waiting to be consumed.
    /// Set to `true` by [`Self::signal`] before the byte is written and
    /// cleared to `false` by [`Self::rearm`] before the reader drains
    /// the pipe.
    signaled: AtomicBool,
}

#[cfg(unix)]
impl WakeFd {
    /// Wrap a previously-allocated write-end FD.
    ///
    /// The caller retains responsibility for the matching read-end FD
    /// (set it on the event loop with `loop.add_reader(read_fd, ...)`).
    /// `write_fd` should be opened with `O_NONBLOCK` so a backed-up
    /// pipe never blocks the I/O thread that signals it.
    #[must_use]
    pub fn from_raw_write_fd(write_fd: RawFd) -> Self {
        Self {
            write_fd,
            signaled: AtomicBool::new(false),
        }
    }

    /// Signal the reader that the iterator queue has at least one fresh
    /// event ready. Idempotent under load — at most one wake byte is in
    /// the pipe at any time.
    ///
    /// Called from the event-dispatch consumer thread on every successful
    /// `queue.push`. The first call after an empty pipe writes one
    /// byte; subsequent calls see `signaled == true` and short-circuit
    /// without touching the pipe FD until [`Self::rearm`] clears the
    /// flag.
    ///
    /// `write(2)` on `O_NONBLOCK` may return `EAGAIN` when the pipe is
    /// full (consumer hung). That case is logged via `tracing::warn!`
    /// and counted on the [`super::FpssClient::dropped_count`] axis is
    /// NOT incremented — the missed wake is recoverable on the next
    /// push, and an over-counted drop would mislead operators reading
    /// the metric. The reader's `next_timeout` poll covers the wedged
    /// case as a long-stop.
    pub fn signal(&self) {
        // Coalesce: first writer to observe `false` wins the write.
        // Cross-thread visibility of the published event ride on the
        // kernel's `write(2)` / `read(2)` pipe lock, not on the atomic;
        // the atomic is just the userspace gate that suppresses
        // redundant syscalls. Coalesce contract pinned by
        // `tests::signal_writes_a_single_byte_until_rearm`.
        if self
            .signaled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // SAFETY: `self.write_fd` is either `-1` (handled below) or a
        // valid open write-end FD the caller passed to `from_raw_write_fd`
        // and that hasn't been dropped yet (this method holds `&self`,
        // which holds the FD alive via `Drop`). The 1-byte buffer points
        // into stack memory and is valid for `len = 1`.
        if self.write_fd < 0 {
            return;
        }
        let byte: u8 = 1;
        // SAFETY: `self.write_fd` is owned by this `WakeFd` (held alive
        // by `&self`); it was either `-1` (filtered by the early return
        // above) or a valid open writable FD passed in via
        // `from_raw_write_fd`. The buffer is a single stack byte
        // (`byte`) valid for the `len = 1` write, and one-byte writes
        // on a pipe are atomic per POSIX (`pipe(7)`, `write(2)`
        // PIPE_BUF). No aliasing — `byte` is a local on this stack
        // frame.
        let res = unsafe {
            libc::write(
                self.write_fd,
                std::ptr::addr_of!(byte).cast::<libc::c_void>(),
                1,
            )
        };
        if res < 0 {
            // `Error::last_os_error()` reads `errno` portably across
            // every libc layout (glibc, musl, macOS, BSD) without
            // dispatching on `__errno_location` / `__error`. We only
            // need the numeric code for the EAGAIN / EWOULDBLOCK / EPIPE
            // filter — no allocation, no formatting.
            let err = std::io::Error::last_os_error();
            let errno = err.raw_os_error().unwrap_or(0);
            // EAGAIN / EWOULDBLOCK on a full pipe is the only expected
            // failure under load. EPIPE means the reader closed the
            // read-end — also benign; the wake_fd is about to be torn
            // down. Anything else is unexpected and worth a warn.
            if errno != libc::EAGAIN && errno != libc::EWOULDBLOCK && errno != libc::EPIPE {
                tracing::warn!(
                    target: "thetadatadx::fpss::wake",
                    errno,
                    "wake-fd write failed; async reader may miss a wake-up until the next push",
                );
            }
        }
    }

    /// Clear the wake-pending flag so the next [`Self::signal`] call
    /// writes a fresh byte to the pipe. Must be called by the reader
    /// BEFORE it drains the pipe with `read(2)`, so a producer push
    /// observed between the clear and the drain still writes a wake
    /// byte the reader will consume on its next `epoll` wait.
    pub fn rearm(&self) {
        // `Release` pairs with the `AcqRel` on the producer's
        // `compare_exchange` in `signal` — every wake byte the producer
        // wrote before the reader's clear is guaranteed to be visible
        // (in the pipe) by the time the reader gets here, and any
        // producer push after the clear will write a fresh byte.
        self.signaled.store(false, Ordering::Release);
    }

    /// Snapshot of the current wake-pending state. Diagnostic only.
    #[must_use]
    pub fn is_signaled(&self) -> bool {
        self.signaled.load(Ordering::Acquire)
    }

    /// Underlying write-end FD. Diagnostic only — callers MUST NOT
    /// `close(2)` this FD; the [`Drop`] impl owns it.
    #[must_use]
    pub fn write_fd(&self) -> RawFd {
        self.write_fd
    }
}

#[cfg(unix)]
impl Drop for WakeFd {
    fn drop(&mut self) {
        if self.write_fd >= 0 {
            // SAFETY: `write_fd` was passed in via `from_raw_write_fd`
            // and has not been closed by anything else in the SDK —
            // [`signal`] only writes to it. Closing here is the
            // canonical ownership transfer.
            unsafe {
                libc::close(self.write_fd);
            }
        }
    }
}

// Stub on non-Unix platforms. The pyclass surfaces a clear "asyncio
// streaming is POSIX-only" error at construction time; the core SDK
// keeps the WakeFd type so the cross-platform signatures compile.
#[cfg(not(unix))]
impl WakeFd {
    /// Stub constructor on non-Unix platforms. The caller's FD is
    /// stashed verbatim so [`Self::write_fd`] returns it, but
    /// [`Self::signal`] never touches it — there is no portable
    /// `write(2)` we can call without dragging a Windows-specific
    /// HANDLE/IOCP abstraction into the core SDK.
    ///
    /// Callers of the async event-loop bindings on non-Unix raise a
    /// clear runtime error at the binding entry; this stub exists so
    /// the Rust signatures remain cross-platform.
    #[must_use]
    pub fn from_raw_write_fd(write_fd: i32) -> Self {
        Self {
            write_fd,
            signaled: AtomicBool::new(false),
        }
    }

    /// No-op on non-Unix.
    pub fn signal(&self) {
        let _ = self.signaled.swap(true, Ordering::AcqRel);
    }

    /// No-op on non-Unix.
    pub fn rearm(&self) {
        self.signaled.store(false, Ordering::Release);
    }

    /// Always `false` on non-Unix.
    #[must_use]
    pub fn is_signaled(&self) -> bool {
        false
    }

    /// Returns the FD value stashed at construction time. On non-Unix
    /// the SDK never reads or writes through it — the value is opaque
    /// — but echoing it back keeps the cross-platform getter contract
    /// symmetric with the Unix impl.
    #[must_use]
    pub fn write_fd(&self) -> i32 {
        self.write_fd
    }
}

// `pipe2(2)` + `__errno_location` used by the test helper are
// Linux-only libc symbols (macOS uses `pipe(2)` + `__error`). The
// wake-coalesce logic the tests exercise is platform-agnostic, so
// gating coverage to Linux is sufficient.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::io::FromRawFd;
    use std::sync::Arc;

    /// Allocate a non-blocking self-pipe for the unit tests.
    ///
    /// Returns `(read_fd, write_fd)`. Both are `O_NONBLOCK | O_CLOEXEC`.
    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0_i32; 2];
        // SAFETY: `pipe2` writes two file descriptors into `fds`.
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        // SAFETY: `libc::__errno_location` returns a per-thread non-null
        // pointer guaranteed by glibc / musl; the deref reads the current
        // thread's errno slot and is sound on any platform with a POSIX C
        // runtime.
        assert_eq!(rc, 0, "pipe2 failed: errno={}", unsafe {
            *libc::__errno_location()
        });
        (fds[0], fds[1])
    }

    #[test]
    fn signal_writes_a_single_byte_until_rearm() {
        let (read_fd, write_fd) = make_pipe();
        let wake = Arc::new(WakeFd::from_raw_write_fd(write_fd));

        wake.signal();
        wake.signal();
        wake.signal();

        // SAFETY: `read_fd` is a valid open pipe-read FD we just allocated.
        let mut reader = unsafe { std::fs::File::from_raw_fd(read_fd) };
        let mut buf = [0_u8; 8];
        let n = reader.read(&mut buf).expect("read should not block");
        assert_eq!(
            n, 1,
            "exactly one wake byte is in the pipe under coalescing"
        );
        assert_eq!(buf[0], 1);

        // No rearm yet — additional `signal()` calls must stay short-circuited.
        wake.signal();
        let mut buf2 = [0_u8; 8];
        let err = reader
            .read(&mut buf2)
            .expect_err("read should EAGAIN — wake is still pending");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);

        // After rearm, a fresh signal makes another byte available.
        wake.rearm();
        wake.signal();
        let n = reader.read(&mut buf2).expect("read should not block");
        assert_eq!(n, 1);
    }

    #[test]
    fn drop_closes_write_fd() {
        let (read_fd, write_fd) = make_pipe();
        {
            let _wake = WakeFd::from_raw_write_fd(write_fd);
        }
        // Reading from the pipe should return EOF (0) now that the
        // write-end is closed.
        // SAFETY: the raw fd was just produced (pipe2 / dup) and is exclusively owned by this scope.
        let mut reader = unsafe { std::fs::File::from_raw_fd(read_fd) };
        let mut buf = [0_u8; 4];
        let n = reader.read(&mut buf).expect("read should not error");
        assert_eq!(n, 0, "write-end closed should produce EOF on the read-end");
    }
}
