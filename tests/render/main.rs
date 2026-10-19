//! Headless render tests: every screen is drawn at many sizes without panicking
//! and text dumps are written under `target/audit/` (for visual review).

mod agents;
mod animation;
mod common;
mod crash_hunt;
mod hints;
mod home;
mod overlays;
mod palette;
mod panes;
mod power;
mod pty;
mod search;
mod session;
mod settings;
/// Shell integration edge cases for bash, zsh and fish (`term/integration.rs`): unusual
/// directory names, user startup options, the user's own prompt hooks and `ZDOTDIR`.
/// Every shell starts through a small wrapper named after it (so NOBLE treats it as that
/// shell) that points HOME, XDG_CONFIG_HOME and XDG_DATA_HOME at a temporary directory: the
/// tests write their own "user config" there and never read the real one.
/// Unix only: the wrapper is a POSIX script. On Windows the same scripts run under Git Bash,
/// whose cwd tracking `cwd_is_tracked_after_cd` covers; pwsh and cmd report with OSC 9;9.
#[cfg(unix)]
mod shells;
mod system;
mod tabs;
mod themes;
mod welcome;
