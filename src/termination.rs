//! The OS asking NOBLE to end: the terminal window closing (SIGHUP), `kill` (SIGTERM), or on
//! Windows the console closing, a logoff or a shutdown. Each turns into `AppEvent::Quit`, so the
//! normal shutdown path runs and saves the session (see `App::shutdown`).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::event::Tx;

/// Set by `finished` once the shutdown path is done.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// Starts listening. Call once, before the main loop. A failure to register is ignored: the
/// periodic session save (`App::autosave_session`) still keeps the session recent.
pub fn install(tx: Tx) {
    #[cfg(unix)]
    {
        // Windows has no SIGHUP/SIGTERM; its console control events are handled in the other branch.
        use signal_hook::consts::{SIGHUP, SIGTERM};
        let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGHUP, SIGTERM]) else { return };
        let _ = std::thread::Builder::new().name("signals".into()).spawn(move || {
            for _ in signals.forever() {
                let _ = tx.send(crate::event::AppEvent::Quit);
            }
        });
    }
    #[cfg(windows)]
    {
        // Unix has no console control events; it uses signals in the other branch.
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        let _ = WINDOWS_TX.set(tx);
        // SAFETY: `on_console_event` is a plain function that lives for the whole process.
        unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) };
        // A process that loaded user32.dll (the clipboard does) never gets CTRL_LOGOFF_EVENT or
        // CTRL_SHUTDOWN_EVENT: a logoff or shutdown is announced to its windows instead.
        let _ = std::thread::Builder::new().name("session-end".into()).spawn(session_end_window);
    }
}

/// The shutdown path is done: on Windows this lets the console handler return, which ends the process.
pub fn finished() {
    FINISHED.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
static WINDOWS_TX: std::sync::OnceLock<Tx> = std::sync::OnceLock::new();

/// Windows gives a closing console about 5 seconds: the handler asks the main loop to quit and
/// waits (up to 4 s) until it has saved, because the process ends as soon as the handler returns.
#[cfg(windows)]
unsafe extern "system" fn on_console_event(kind: u32) -> i32 {
    use windows_sys::Win32::System::Console::{CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT};
    if !matches!(kind, CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT) {
        // Ctrl+C / Ctrl+Break: the default handling (raw mode already delivers Ctrl+C as a key).
        return 0;
    }
    quit_and_wait();
    1
}

/// Asks the main loop to quit and waits (up to 4 s) until the shutdown path has saved.
#[cfg(windows)]
fn quit_and_wait() {
    if let Some(tx) = WINDOWS_TX.get() {
        let _ = tx.send(crate::event::AppEvent::Quit);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !FINISHED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// A hidden top-level window whose only job is to hear `WM_ENDSESSION` (logoff, shutdown, restart) and
/// run the shutdown path before Windows ends the process. Runs its own message loop on this thread.
#[cfg(windows)]
fn session_end_window() {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage, WNDCLASSW,
    };
    let class: Vec<u16> = "NobleSessionEnd\0".encode_utf16().collect();
    // SAFETY: plain Win32 calls; `class` outlives the window (this function only returns when the message
    // loop ends), the structs are zero-initialized C structs and every handle is checked before use.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(on_window_message);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        if RegisterClassW(&wc) == 0 {
            return;
        }
        // Never shown (no WS_VISIBLE). Not a message-only window: those do not get session messages.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn on_window_message(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_ENDSESSION, WM_QUERYENDSESSION};
    match msg {
        // NOBLE never blocks a logoff.
        WM_QUERYENDSESSION => 1,
        // `wparam` is nonzero when the session really ends (not when another app cancelled it).
        WM_ENDSESSION => {
            if wparam != 0 {
                quit_and_wait();
            }
            0
        }
        // SAFETY: the default handling for every other message, with the arguments Windows passed in.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
