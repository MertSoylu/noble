use crate::pty::{Child, ChildKiller, ExitStatus};
use anyhow::Context as _;
use std::io::{Error as IoError, Result as IoResult};
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use windows_sys::Win32::Foundation::STILL_ACTIVE;
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, INFINITE, TerminateProcess, WaitForSingleObject,
};

pub mod conpty;
mod procthreadattr;
mod pseudocon;

use crate::pty::filedescriptor::OwnedHandle;

#[derive(Debug)]
pub struct WinChild {
    proc: Mutex<OwnedHandle>,
}

impl WinChild {
    fn is_complete(&mut self) -> IoResult<Option<ExitStatus>> {
        let mut status: u32 = 0;
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle(), &mut status) };
        if res != 0 {
            if status == STILL_ACTIVE as u32 { Ok(None) } else { Ok(Some(ExitStatus::with_exit_code(status))) }
        } else {
            Ok(None)
        }
    }

    fn do_kill(&mut self) -> IoResult<()> {
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        let res = unsafe { TerminateProcess(proc.as_raw_handle(), 1) };
        let err = IoError::last_os_error();
        if res == 0 { Err(err) } else { Ok(()) }
    }
}

impl ChildKiller for WinChild {
    fn kill(&mut self) -> IoResult<()> {
        self.do_kill().ok();
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

#[derive(Debug)]
pub struct WinChildKiller {
    proc: OwnedHandle,
}

impl ChildKiller for WinChildKiller {
    fn kill(&mut self) -> IoResult<()> {
        let res = unsafe { TerminateProcess(self.proc.as_raw_handle(), 1) };
        let err = IoError::last_os_error();
        if res == 0 { Err(err) } else { Ok(()) }
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

impl Child for WinChild {
    fn try_wait(&mut self) -> IoResult<Option<ExitStatus>> {
        self.is_complete()
    }

    fn wait(&mut self) -> IoResult<ExitStatus> {
        if let Ok(Some(status)) = self.try_wait() {
            return Ok(status);
        }
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        unsafe {
            WaitForSingleObject(proc.as_raw_handle(), INFINITE);
        }
        let mut status: u32 = 0;
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle(), &mut status) };
        if res != 0 { Ok(ExitStatus::with_exit_code(status)) } else { Err(IoError::last_os_error()) }
    }

    fn process_id(&self) -> Option<u32> {
        let res = unsafe { GetProcessId(self.proc.lock().unwrap().as_raw_handle()) };
        if res == 0 { None } else { Some(res) }
    }

    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        let proc = self.proc.lock().unwrap();
        Some(proc.as_raw_handle())
    }
}

impl std::future::Future for WinChild {
    type Output = anyhow::Result<ExitStatus>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<anyhow::Result<ExitStatus>> {
        match self.is_complete() {
            Ok(Some(status)) => Poll::Ready(Ok(status)),
            Err(err) => Poll::Ready(Err(err).context("Failed to retrieve process exit status")),
            Ok(None) => {
                struct PassRawHandleToWaiterThread(pub RawHandle);
                unsafe impl Send for PassRawHandleToWaiterThread {}
                impl PassRawHandleToWaiterThread {
                    // A method rather than `.0` in the closure: edition 2021 closures capture the field, which is
                    // not `Send`, while this captures the whole wrapper.
                    fn into_raw(self) -> RawHandle {
                        self.0
                    }
                }

                let proc = self.proc.lock().unwrap().try_clone()?;
                let handle = PassRawHandleToWaiterThread(proc.as_raw_handle());

                let waker = cx.waker().clone();
                std::thread::spawn(move || {
                    unsafe {
                        WaitForSingleObject(handle.into_raw(), INFINITE);
                    }
                    waker.wake();
                });
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::{CommandBuilder, PtySize, native_pty_system};
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};

    /// `kill` reports what `TerminateProcess` did: it ends a running process and returns `Ok`, and a handle that is not a
    /// process makes it return `Err` (every NOBLE caller discards that result, so only this test sees it). This module is
    /// Windows only, so the test is too.
    #[test]
    fn kill_reports_whether_the_process_was_ended() {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .expect("a pseudo console");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/C", "pause"]);
        let mut child = pair.slave.spawn_command(cmd).expect("cmd.exe starts on the pseudo console");
        let mut killer = child.clone_killer();
        killer.kill().expect("TerminateProcess ends the running process");
        child.wait().expect("the killed process can be waited for");
        // A file handle is not a process: TerminateProcess fails, and kill says so.
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let mut not_a_process =
            WinChildKiller { proc: unsafe { OwnedHandle::from_raw_handle(file.into_raw_handle()) } };
        assert!(not_a_process.kill().is_err());
    }
}
