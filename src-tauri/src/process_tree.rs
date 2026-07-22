//! Cross-platform ownership of a short-lived CLI process tree.
//!
//! Unix callers create a dedicated process group before spawning and retain
//! its numeric group ID locally. Windows has no equivalent process group, so
//! a Job Object keeps a handle-based identity that remains valid even after
//! the direct child has exited.

use std::io;
use std::process::Child;

#[cfg(windows)]
pub struct ProcessTreeHandle {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(not(windows))]
pub struct ProcessTreeHandle;

#[cfg(windows)]
pub fn attach_process_tree(child: &Child) -> io::Result<ProcessTreeHandle> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // SAFETY: null security attributes/name request an unnamed job owned by
    // this process. The returned handle is closed by `ProcessTreeHandle`.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }

    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is initialized and lives through this synchronous API
    // call; the structure size and information class match exactly.
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: this is the still-owned Job Object handle.
        unsafe {
            let _ = CloseHandle(job);
        }
        return Err(error);
    }

    // SAFETY: `Child` owns a valid process handle for this operation. The Job
    // Object keeps the process tree associated until its own handle closes.
    let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as _) };
    if assigned == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: this is the still-owned Job Object handle.
        unsafe {
            let _ = CloseHandle(job);
        }
        return Err(error);
    }

    Ok(ProcessTreeHandle { job })
}

#[cfg(not(windows))]
pub fn attach_process_tree(_child: &Child) -> io::Result<ProcessTreeHandle> {
    Ok(ProcessTreeHandle)
}

#[cfg(windows)]
pub fn terminate_process_tree(process_tree: &ProcessTreeHandle) {
    use windows_sys::Win32::System::JobObjects::TerminateJobObject;

    // SAFETY: the Job Object is owned by this handle and contains only the
    // child process tree assigned above.
    unsafe {
        let _ = TerminateJobObject(process_tree.job, 1);
    }
}

#[cfg(not(windows))]
pub fn terminate_process_tree(_process_tree: &ProcessTreeHandle) {}

#[cfg(windows)]
impl Drop for ProcessTreeHandle {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // Closing a configured Job Object kills any descendant that retained
        // a pipe after the direct CLI process exited.
        unsafe {
            let _ = CloseHandle(self.job);
        }
    }
}
