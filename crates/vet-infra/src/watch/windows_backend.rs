//! Recursive watch on `ReadDirectoryChangesW`, driven directly. `notify` ignores the
//! byte count of a completion and unwatches silently on handle errors; this module
//! does neither:
//! - a zero-byte completion (or `ERROR_NOTIFY_ENUM_DIR`) means the buffer overflowed:
//!   `MustRescan` at once;
//! - a handle error (watched folder deleted or renamed, network drop) re-opens the
//!   watch with retries and reports `MustRescan` once it is back, and also when the
//!   retries are exhausted (it then keeps trying more slowly).
//!
//! One thread per watch issues overlapped reads and waits on the read's event and a
//! stop event, so dropping the watcher ends the thread promptly.

#![allow(unsafe_code)] // Win32 FFI: no safe wrapper for ReadDirectoryChangesW exists.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use vet_app::ports::WatchEvent;
use windows_sys::Win32::Foundation::{
    ERROR_NOTIFY_ENUM_DIR, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES,
    FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};

use super::dirty_event;

const BUFFER_BYTES: usize = 64 * 1024;
const REOPEN_ATTEMPTS: u32 = 5;
const REOPEN_DELAY: Duration = Duration::from_millis(200);
const REOPEN_SLOW_DELAY: Duration = Duration::from_secs(1);
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_ATTRIBUTES
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION;
/// `FILE_NOTIFY_INFORMATION` header: next offset, action, name length in bytes.
const HEADER: usize = 12;

/// Stops and joins the watch thread when dropped.
pub struct DirWatcher {
    stop: OwnedHandle,
    thread: Option<JoinHandle<()>>,
}

impl DirWatcher {
    pub fn open(root: &Path, tx: UnboundedSender<WatchEvent>) -> io::Result<Self> {
        Self::open_with_buffer(root, BUFFER_BYTES, tx)
    }

    fn open_with_buffer(root: &Path, buffer_bytes: usize, tx: UnboundedSender<WatchEvent>) -> io::Result<Self> {
        let stop = create_event()?;
        // The thread is joined before `stop` is closed, so the raw value stays valid.
        let stop_raw = stop.as_raw_handle() as usize;
        let root = root.to_owned();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("vet-watch".into())
            .spawn(move || run(&root, buffer_bytes, stop_raw as HANDLE, &tx, &ready_tx))?;
        let watcher = Self {
            stop,
            thread: Some(thread),
        };
        // A failed first open is the caller's error; the thread has exited by then.
        ready_rx.recv().map_err(io::Error::other)??;
        Ok(watcher)
    }
}

impl Drop for DirWatcher {
    fn drop(&mut self) {
        // SAFETY: `stop` is a live event handle owned by `self`.
        unsafe { SetEvent(self.stop.as_raw_handle()) };
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

fn create_event() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name are allowed; the result is checked below.
    let handle = unsafe { CreateEventW(null(), 1, 0, null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a valid event handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
}

/// True when `stop` is signalled within `timeout`.
fn stopped(stop: HANDLE, timeout: Duration) -> bool {
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE - 1);
    // SAFETY: `stop` stays valid for the whole thread lifetime (see `open_with_buffer`).
    unsafe { WaitForSingleObject(stop, millis) == WAIT_OBJECT_0 }
}

enum Completion {
    Stopped,
    Overflow,
    Changes(Vec<OsString>),
}

/// One open directory handle with at most one pending overlapped read.
struct Watch {
    dir: OwnedHandle,
    event: OwnedHandle,
    overlapped: Box<OVERLAPPED>,
    /// `u32` elements: the kernel requires a DWORD-aligned buffer.
    buffer: Box<[u32]>,
    pending: bool,
}

impl Watch {
    fn open(root: &Path, buffer_bytes: usize) -> io::Result<Self> {
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; the result is checked below.
        let dir = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if dir == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `dir` is a valid handle that nothing else owns.
        let dir = unsafe { OwnedHandle::from_raw_handle(dir as RawHandle) };
        Ok(Self {
            dir,
            event: create_event()?,
            overlapped: Box::new(overlapped_for(null_mut())),
            buffer: vec![0; buffer_bytes.div_ceil(4).max(1)].into_boxed_slice(),
            pending: false,
        })
    }

    fn arm(&mut self) -> io::Result<()> {
        *self.overlapped = overlapped_for(self.event.as_raw_handle() as HANDLE);
        // SAFETY: `event` is a live event handle owned by `self`.
        unsafe { ResetEvent(self.event.as_raw_handle()) };
        // SAFETY: `dir` is an overlapped directory handle. `buffer` and `overlapped` are boxed,
        // so their addresses are stable, and `Drop` cancels the read before they are freed.
        let ok = unsafe {
            ReadDirectoryChangesW(
                self.dir.as_raw_handle(),
                self.buffer.as_mut_ptr().cast(),
                u32::try_from(self.buffer.len() * 4).unwrap_or(u32::MAX),
                1,
                FILTER,
                null_mut(),
                &raw mut *self.overlapped,
                None,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        self.pending = true;
        Ok(())
    }

    fn wait(&mut self, stop: HANDLE) -> io::Result<Completion> {
        let handles = [self.event.as_raw_handle(), stop];
        // SAFETY: both handles are valid and `handles` holds the two counted entries.
        let woken = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        if woken == WAIT_OBJECT_0 + 1 {
            return Ok(Completion::Stopped);
        }
        if woken != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = 0u32;
        // SAFETY: the read is complete (its event is signalled); `overlapped` is the one it used.
        let ok = unsafe { GetOverlappedResult(self.dir.as_raw_handle(), &*self.overlapped, &mut bytes, 0) };
        self.pending = false;
        if ok == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == i32::try_from(ERROR_NOTIFY_ENUM_DIR).ok() {
                Ok(Completion::Overflow)
            } else {
                Err(error)
            };
        }
        if bytes == 0 {
            return Ok(Completion::Overflow);
        }
        let len = (bytes as usize).min(self.buffer.len() * 4);
        // SAFETY: the buffer owns `buffer.len() * 4` initialised bytes and `len` does not exceed that.
        let filled = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), len) };
        Ok(parse(filled).map_or(Completion::Overflow, Completion::Changes))
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.pending {
            let mut bytes = 0u32;
            // SAFETY: cancels the read issued on `dir`, then waits until the kernel is done with
            // `buffer` and `overlapped`, which are freed after this returns.
            unsafe {
                CancelIoEx(self.dir.as_raw_handle(), &*self.overlapped);
                GetOverlappedResult(self.dir.as_raw_handle(), &*self.overlapped, &mut bytes, 1);
            }
        }
    }
}

fn overlapped_for(event: HANDLE) -> OVERLAPPED {
    // SAFETY: OVERLAPPED is plain data for which all-zero is the documented initial state.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event;
    overlapped
}

/// Relative names of a `FILE_NOTIFY_INFORMATION` chain; `None` when it is malformed.
fn parse(bytes: &[u8]) -> Option<Vec<OsString>> {
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize);
    let mut names = Vec::new();
    let mut at = 0;
    loop {
        let next = word(at)?;
        let name_len = word(at + 8)?;
        let name = bytes.get(at + HEADER..at + HEADER + name_len)?;
        let wide: Vec<u16> = name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        names.push(OsString::from_wide(&wide));
        if next == 0 {
            return Some(names);
        }
        at += next;
    }
}

fn run(
    root: &Path,
    buffer_bytes: usize,
    stop: HANDLE,
    tx: &UnboundedSender<WatchEvent>,
    ready: &mpsc::Sender<io::Result<()>>,
) {
    let mut watch = match Watch::open(root, buffer_bytes) {
        Ok(watch) => {
            drop(ready.send(Ok(())));
            watch
        }
        Err(e) => {
            drop(ready.send(Err(e)));
            return;
        }
    };
    loop {
        let event = match watch.arm().and_then(|()| watch.wait(stop)) {
            Ok(Completion::Stopped) => return,
            Ok(Completion::Overflow) => WatchEvent::MustRescan,
            Ok(Completion::Changes(names)) => {
                match dirty_event(root, names.iter().map(|name| PathBuf::from(root).join(name))) {
                    Some(event) => event,
                    None => continue,
                }
            }
            Err(_) => {
                let Some(reopened) = reopen(root, buffer_bytes, stop, tx) else {
                    return;
                };
                watch = reopened;
                WatchEvent::MustRescan
            }
        };
        if tx.send(event).is_err() {
            return;
        }
    }
}

/// Retries until the folder can be watched again. `None` when stopped or the receiver is gone.
fn reopen(root: &Path, buffer_bytes: usize, stop: HANDLE, tx: &UnboundedSender<WatchEvent>) -> Option<Watch> {
    for attempt in 1.. {
        let delay = if attempt <= REOPEN_ATTEMPTS {
            REOPEN_DELAY
        } else {
            REOPEN_SLOW_DELAY
        };
        if stopped(stop, delay) {
            return None;
        }
        if let Ok(watch) = Watch::open(root, buffer_bytes) {
            return Some(watch);
        }
        if attempt == REOPEN_ATTEMPTS && tx.send(WatchEvent::MustRescan).is_err() {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use vet_core::content::RepoPath;

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(20);

    async fn until(rx: &mut UnboundedReceiver<WatchEvent>, wanted: impl Fn(&WatchEvent) -> bool) -> bool {
        tokio::time::timeout(TIMEOUT, async {
            while let Some(event) = rx.recv().await {
                if wanted(&event) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }

    #[tokio::test]
    async fn writes_become_dirty_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let _watcher = DirWatcher::open(dir.path(), tx).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("a.rs"), b"x").unwrap();
        let wanted = RepoPath(b"src/a.rs".to_vec());
        assert!(
            until(
                &mut rx,
                |event| matches!(event, WatchEvent::Dirty(paths) if paths.contains(&wanted))
            )
            .await,
            "no Dirty event for src/a.rs"
        );
    }

    #[tokio::test]
    async fn a_burst_into_a_tiny_buffer_rescans() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let _watcher = DirWatcher::open_with_buffer(dir.path(), 64, tx).unwrap();
        for i in 0..2000 {
            std::fs::write(dir.path().join(format!("file-with-a-long-name-{i}.txt")), b"x").unwrap();
        }
        assert!(
            until(&mut rx, |event| *event == WatchEvent::MustRescan).await,
            "no MustRescan after the burst"
        );
    }

    #[tokio::test]
    async fn deleting_the_watched_folder_rescans_and_drop_does_not_hang() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let watcher = DirWatcher::open(&root, tx).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            until(&mut rx, |event| *event == WatchEvent::MustRescan).await,
            "no MustRescan after the folder was deleted"
        );
        let dropped = tokio::task::spawn_blocking(move || drop(watcher));
        assert!(tokio::time::timeout(TIMEOUT, dropped).await.is_ok(), "drop hung");
    }
}
