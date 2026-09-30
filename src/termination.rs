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
    if let Some(tx) = WINDOWS_TX.get() {
        let _ = tx.send(crate::event::AppEvent::Quit);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !FINISHED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    1
}
