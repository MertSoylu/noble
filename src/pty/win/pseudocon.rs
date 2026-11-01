use super::WinChild;
use crate::pty::cmdbuilder::CommandBuilder;
use crate::pty::filedescriptor::{FileDescriptor, OwnedHandle};
use crate::pty::win::procthreadattr::ProcThreadAttributeList;
use anyhow::{Error, bail, ensure};
use std::ffi::{CStr, OsStr, OsString};
use std::io::Error as IoError;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::{LazyLock, Mutex};
use std::{mem, ptr};
use windows_sys::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE, INVALID_HANDLE_VALUE, S_OK};
use windows_sys::Win32::System::Console::COORD;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_FLAGS, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    LoadLibraryExW,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows_sys::core::HRESULT;

// The Win32 name of the pseudo console handle (`HPCON` in the SDK); a lint about its capitals is not worth a rename.
#[allow(clippy::upper_case_acronyms)]
pub type HPCON = HANDLE;

pub const PSEUDOCONSOLE_INHERIT_CURSOR: u32 = 0x1;
pub const PSEUDOCONSOLE_RESIZE_QUIRK: u32 = 0x2;
pub const PSEUDOCONSOLE_WIN32_INPUT_MODE: u32 = 0x4;
#[allow(dead_code)]
pub const PSEUDOCONSOLE_PASSTHROUGH_MODE: u32 = 0x8;

type CreatePseudoConsoleFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizePseudoConsoleFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type ClosePseudoConsoleFn = unsafe extern "system" fn(HPCON);
/// The untyped address `GetProcAddress` returns.
type ExportFn = unsafe extern "system" fn() -> isize;

/// The three ConPTY functions, from `kernel32.dll` or a sideloaded `conpty.dll`.
struct ConPtyFuncs {
    create: CreatePseudoConsoleFn,
    resize: ResizePseudoConsoleFn,
    close: ClosePseudoConsoleFn,
}

impl ConPtyFuncs {
    /// Loads the module `file` and looks up the three functions; `None` when it cannot be loaded or does not export
    /// all of them. `flags` limit where a bare file name is searched. The default search (`LoadLibraryW`, which
    /// `shared_library` used) also looks in the current folder and along PATH, so a `conpty.dll` placed there would be
    /// loaded. The callers pass the flags that search only the folder of the executable and System32.
    fn open(file: &str, flags: LOAD_LIBRARY_FLAGS) -> Option<Self> {
        let wide: Vec<u16> = OsStr::new(file).encode_wide().chain(std::iter::once(0)).collect();
        let module: HMODULE = unsafe { LoadLibraryExW(wide.as_ptr(), ptr::null_mut(), flags) };
        if module.is_null() {
            return None;
        }
        let (Some(create), Some(resize), Some(close)) = (
            export(module, c"CreatePseudoConsole"),
            export(module, c"ResizePseudoConsole"),
            export(module, c"ClosePseudoConsole"),
        ) else {
            // A module without the three functions is not used: release it, as `shared_library` did.
            unsafe { FreeLibrary(module) };
            return None;
        };
        // SAFETY: each pointer is the export of that name, and the types above are its documented signatures. Every
        // function pointer has the same size, so the transmutes only change the type.
        unsafe {
            Some(Self {
                create: mem::transmute::<ExportFn, CreatePseudoConsoleFn>(create),
                resize: mem::transmute::<ExportFn, ResizePseudoConsoleFn>(resize),
                close: mem::transmute::<ExportFn, ClosePseudoConsoleFn>(close),
            })
        }
    }
}

/// The address of the export `name` of `module`, or `None` when it does not have one.
fn export(module: HMODULE, name: &CStr) -> Option<ExportFn> {
    unsafe { GetProcAddress(module, name.as_ptr().cast()) }
}

/// Where a bare `conpty.dll` name is searched: the folder of `noble.exe`, then System32. The current folder and PATH
/// are never searched, so a planted copy there is not loaded into `noble.exe`.
const CONPTY_LOAD_FLAGS: LOAD_LIBRARY_FLAGS = LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;

fn load_conpty() -> ConPtyFuncs {
    // If the kernel doesn't export these functions then their system is
    // too old and we cannot run. System32 only: the kernel is never looked up anywhere else.
    let kernel = ConPtyFuncs::open("kernel32.dll", LOAD_LIBRARY_SEARCH_SYSTEM32)
        .expect("this system does not support conpty.  Windows 10 October 2018 or newer is required");

    // We prefer to use a sideloaded conpty.dll and openconsole.exe host deployed
    // alongside the application.  We check for this after checking for kernel
    // support so that we don't try to proceed and do something crazy. The search is the
    // folder of noble.exe, then System32 (where a system copy would be); the current folder and PATH are not searched.
    ConPtyFuncs::open("conpty.dll", CONPTY_LOAD_FLAGS).unwrap_or(kernel)
}

static CONPTY: LazyLock<ConPtyFuncs> = LazyLock::new(load_conpty);

pub struct PseudoCon {
    con: HPCON,
}

unsafe impl Send for PseudoCon {}
unsafe impl Sync for PseudoCon {}

impl Drop for PseudoCon {
    fn drop(&mut self) {
        unsafe { (CONPTY.close)(self.con) };
    }
}

impl PseudoCon {
    pub fn new(size: COORD, input: FileDescriptor, output: FileDescriptor) -> Result<Self, Error> {
        let mut con: HPCON = INVALID_HANDLE_VALUE;
        let result = unsafe {
            (CONPTY.create)(
                size,
                input.as_raw_handle(),
                output.as_raw_handle(),
                PSEUDOCONSOLE_INHERIT_CURSOR | PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE,
                &mut con,
            )
        };
        ensure!(result == S_OK, "failed to create pseudo console: HRESULT {}", result);
        Ok(Self { con })
    }

    pub fn resize(&self, size: COORD) -> Result<(), Error> {
        let result = unsafe { (CONPTY.resize)(self.con, size) };
        ensure!(result == S_OK, "failed to resize console to {}x{}: HRESULT: {}", size.X, size.Y, result);
        Ok(())
    }

    pub fn spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<WinChild> {
        let mut si: STARTUPINFOEXW = unsafe { mem::zeroed() };
        si.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        // Explicitly set the stdio handles as invalid handles otherwise
        // we can end up with a weird state where the spawned process can
        // inherit the explicitly redirected output handles from its parent.
        // For example, when daemonizing wezterm-mux-server, the stdio handles
        // are redirected to a log file and the spawned process would end up
        // writing its output there instead of to the pty we just created.
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;

        let mut attrs = ProcThreadAttributeList::with_capacity(1)?;
        attrs.set_pty(self.con)?;
        si.lpAttributeList = attrs.as_mut_ptr();

        let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let (mut exe, mut cmdline) = cmd.cmdline()?;
        let cmd_os = OsString::from_wide(&cmdline);

        let cwd = cmd.current_directory();

        let res = unsafe {
            CreateProcessW(
                exe.as_mut_slice().as_mut_ptr(),
                cmdline.as_mut_slice().as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                cmd.environment_block().as_mut_slice().as_mut_ptr() as *mut _,
                cwd.as_ref().map(|c| c.as_slice().as_ptr()).unwrap_or(ptr::null()),
                &si.StartupInfo,
                &mut pi,
            )
        };
        if res == 0 {
            let err = IoError::last_os_error();
            let msg = format!(
                "CreateProcessW `{:?}` in cwd `{:?}` failed: {}",
                cmd_os,
                cwd.as_ref().map(|c| OsString::from_wide(c)),
                err
            );
            log::error!("{}", msg);
            bail!("{}", msg);
        }

        // Make sure we close out the thread handle so we don't leak it;
        // we do this simply by making it owned
        let _main_thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as _) };
        let proc = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as _) };

        Ok(WinChild { proc: Mutex::new(proc) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

    /// Set for the child run of `probe_child`: the parent sets it, and `probe_child` does nothing without it.
    const PROBE_ENV: &str = "NOBLE_CONPTY_PROBE";

    /// The file that `conpty.dll` resolves to with `flags`, as `load_conpty` searches for it (`None` when nothing is
    /// found). The module is released again.
    fn load_conpty_file(flags: LOAD_LIBRARY_FLAGS) -> Option<PathBuf> {
        let name: Vec<u16> = OsStr::new("conpty.dll").encode_wide().chain(std::iter::once(0)).collect();
        let module: HMODULE = unsafe { LoadLibraryExW(name.as_ptr(), ptr::null_mut(), flags) };
        if module.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        unsafe { FreeLibrary(module) };
        (len > 0 && len < buf.len()).then(|| canonical(&PathBuf::from(OsString::from_wide(&buf[..len]))))
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// A `conpty.dll` in the current folder is not loaded by the search `load_conpty` uses. The check runs in a child
    /// test process whose current folder holds a copy of a system DLL named `conpty.dll`: the current folder is
    /// process-wide, so this test process keeps its own. The default search, which the old code used, does find that
    /// copy, so the check would catch a change back to it.
    #[test]
    fn a_conpty_dll_in_the_current_folder_is_not_loaded() {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let folder = std::env::temp_dir().join(format!("noble-conpty-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        // version.dll is a small system DLL that loads on its own; the copy only has to load.
        std::fs::copy(system.join("System32").join("version.dll"), folder.join("conpty.dll")).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .current_dir(&folder)
            .env(PROBE_ENV, "1")
            .args(["--exact", "pty::win::pseudocon::tests::probe_child", "--nocapture"])
            .status();
        let _ = std::fs::remove_dir_all(&folder);
        assert!(status.is_ok_and(|s| s.success()), "the probe run failed");
    }

    /// The child run of `a_conpty_dll_in_the_current_folder_is_not_loaded`; it does nothing in a normal test run.
    #[test]
    fn probe_child() {
        if std::env::var_os(PROBE_ENV).is_none() {
            return;
        }
        let planted = canonical(&std::env::current_dir().unwrap().join("conpty.dll"));
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system_copy = system.join("System32").join("conpty.dll");
        // The search `load_conpty` uses does not pick the copy in the current folder (System32's own copy may be picked).
        assert_ne!(
            load_conpty_file(CONPTY_LOAD_FLAGS).as_ref(),
            Some(&planted),
            "the current folder's conpty.dll loaded"
        );
        // The default search does reach the current folder, so the check above is not empty. A System32 copy wins that
        // search, so this control runs only where System32 has no conpty.dll.
        if !system_copy.exists() {
            assert_eq!(load_conpty_file(0), Some(planted), "the default search finds the copy in the current folder");
        }
    }
}
