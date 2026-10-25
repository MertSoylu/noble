//! Vendored copy of the `vt100` crate, version 0.18.1 (upstream commit fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad,
//! <https://github.com/doy/vt100-rust>), (c) 2016 Jesse Luehrs, MIT license (see `LICENSE-MIT` in this folder).
//! It is part of NOBLE so the emulator can be changed here (reflow on resize). Apart from the module paths
//! (`crate::` became `crate::vt100::`), dropping the `std`/`embedded-io` features and the crate-level lint
//! attributes, it is the upstream code.
//!
//! This crate parses a terminal byte stream and provides an in-memory
//! representation of the rendered contents.
//!
//! # Overview
//!
//! This is essentially the terminal parser component of a graphical terminal
//! emulator pulled out into a separate crate. Although you can use this crate
//! to build a graphical terminal emulator, it also contains functionality
//! necessary for implementing terminal applications that want to run other
//! terminal applications - programs like `screen` or `tmux` for example.
//!
//! # Synopsis
//!
//! ```
//! let mut parser = noble::vt100::Parser::default();
//!
//! let screen = parser.screen().clone();
//! parser.process(b"this text is \x1b[31mRED\x1b[m");
//! assert_eq!(
//!     parser.screen().cell(0, 13).unwrap().fgcolor(),
//!     noble::vt100::Color::Idx(1),
//! );
//!
//! let screen = parser.screen().clone();
//! parser.process(b"\x1b[3D\x1b[32mGREEN");
//! assert_eq!(
//!     parser.screen().contents_formatted(),
//!     "\x1b[?25h\x1b[m\x1b[H\x1b[Jthis text is \x1b[32mGREEN",
//! );
//! assert_eq!(
//!     parser.screen().contents_diff(&screen),
//!     "\x1b[1;14H\x1b[32mGREEN",
//! );
//! ```

// Upstream lints with pedantic/nursery on; NOBLE runs the default set, minus what the unchanged code trips.
#![allow(missing_docs, clippy::collapsible_if, clippy::too_many_arguments)]

mod attrs;
mod callbacks;
pub mod capture;
mod cell;
mod grid;
mod parser;
mod perform;
mod reflow;
mod row;
mod screen;
mod term;

#[cfg(test)]
mod reflow_tests;

pub use attrs::Color;
pub use callbacks::Callbacks;
pub use cell::Cell;
pub use parser::Parser;
pub use screen::{CursorStyle, MouseProtocolEncoding, MouseProtocolMode, Screen};
