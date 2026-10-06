//! The tree is a job object that kills its processes when the last handle to it
//! closes, which also happens when the backend dies.

#![allow(unsafe_code)] // Win32 FFI: no safe wrapper for job objects exists.

use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;

use tokio::process::{Child, Command};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};

pub fn prepare(_: &mut Command) {}

pub struct Tree {
    job: OwnedHandle,
}

impl Tree {
    /// The process runs briefly outside the job, so a child it starts in that
    /// window escapes it.
    pub fn attach(child: &Child) -> io::Result<Tree> {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the process already exited"))?;
        // SAFETY: null attributes and name are allowed; the result is checked below.
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a job handle that nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: an all-zero limit record is valid; only the flag is set.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` outlives the call and the size passed is its own.
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both handles are open; `process` is owned by `child`, which the caller holds.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Tree { job })
    }

    /// There is no polite way to end a tree of console programs, so this kills.
    pub fn terminate(&self) {
        self.kill();
    }

    pub fn kill(&self) {
        // SAFETY: the job handle is open for as long as `self` lives.
        unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) };
    }
}
