//! Own the entire server tree before it can launch descendants.
use std::{
    mem::{size_of, zeroed},
    os::windows::io::AsRawHandle,
    process::Child,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

#[link(name = "ntdll")]
extern "system" {
    fn NtResumeProcess(process_handle: HANDLE) -> i32;
}

pub(super) struct Job(HANDLE);

impl Job {
    pub fn attach_and_resume(child: &mut Child) -> Result<Self, String> {
        let result = Self::attach(child).and_then(|job| {
            if unsafe { NtResumeProcess(child.as_raw_handle() as HANDLE) } < 0 {
                return Err("could not resume MCP server".into());
            }
            Ok(job)
        });
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    fn attach(child: &Child) -> Result<Self, String> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err("could not create MCP process job".into());
        }
        let job = Self(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0
            || unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0
        {
            return Err("could not contain MCP server process tree".into());
        }
        Ok(job)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
