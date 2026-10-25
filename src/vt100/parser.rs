/// A parser for terminal output which produces an in-memory representation of
/// the terminal contents.
pub struct Parser<CB: crate::vt100::Callbacks = ()> {
    parser: vte::Parser,
    screen: crate::vt100::screen::WrappedScreen<CB>,
}

impl Parser {
    /// The default number of rows used when creating a parser with
    /// [`Parser::default`].
    ///
    /// The value of this constant is 24.
    // `unwrap` is not `const` in this crate's MSRV.
    pub const DEFAULT_ROWS: core::num::NonZeroU16 = match core::num::NonZeroU16::new(24) {
        Some(n) => n,
        None => unreachable!(),
    };

    /// The default number of columns used when creating a parser with
    /// [`Parser::default`].
    ///
    /// The value of this constant is 80.
    pub const DEFAULT_COLS: core::num::NonZeroU16 = match core::num::NonZeroU16::new(80) {
        Some(n) => n,
        None => unreachable!(),
    };

    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback.
    #[must_use]
    pub fn new(rows: core::num::NonZeroU16, cols: core::num::NonZeroU16, scrollback_capacity: usize) -> Self {
        Self::new_with_callbacks(rows, cols, scrollback_capacity, ())
    }
}

impl<CB: crate::vt100::Callbacks> Parser<CB> {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback. Terminal events will be reported via method
    /// calls on the provided [`Callbacks`](crate::vt100::Callbacks) implementation.
    pub fn new_with_callbacks(
        rows: core::num::NonZeroU16,
        cols: core::num::NonZeroU16,
        scrollback_capacity: usize,
        callbacks: CB,
    ) -> Self {
        let screen = crate::vt100::screen::Screen::new(crate::vt100::grid::Size { rows, cols }, scrollback_capacity);
        Self { parser: vte::Parser::new(), screen: crate::vt100::screen::WrappedScreen { screen, callbacks } }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    /// Resizes the terminal.
    ///
    /// When the screen height is decreased, this function does the following:
    ///
    /// * If the cursor is not at the bottom of the screen, rows from the
    ///   bottom are discarded until the cursor is at the bottom (or until the
    ///   desired height is reached, whichever is earlier).
    /// * If the screen is still too tall after that, rows from the top of the
    ///   screen are pushed into scrollback. If this would cause scrollback to
    ///   exceed the maximum size, rows from the top of scrollback are
    ///   discarded, and the [`on_scroll`] callback is called for each of them.
    ///
    /// When the screen height is increased, this function does the following:
    ///
    /// * Rows from the bottom of scrollback are pushed to the top of the
    ///   screen and removed from scrollback, until the desired height is
    ///   reached.[^1]
    /// * If, after pushing all scrollback to the top, the screen is still too
    ///   short, blank rows are added to the bottom of the screen until the
    ///   desired height is reached.
    ///
    /// When the screen width is changed, each row is naively truncated or
    /// extended with blank cells as necessary. This does not match the
    /// behavior of most terminals and may be changed in a future
    /// SemVer-incompatible version.
    ///
    /// [^1]: The parser tracks how many rows of scrollback there *would* be if
    /// the scrollback capacity were unbounded. For the purposes of determining
    /// how many rows should be pushed to the top vs. the bottom, this
    /// *theoretical scrollback length* is used rather than the actual length
    /// of the scrollback buffer, which is capped at `scrollback_capacity`.
    /// Assuming a sufficiently large change in height, once the real
    /// scrollback buffer has been emptied, blank rows will continue to be
    /// pushed to the top of the screen until the theoretical scrollback length
    /// is reached, and only then will blank rows be added to the bottom until
    /// the desired height is reached. This behavior is intended to more
    /// closely match the behavior of a real terminal emulator that this parser
    /// might be mirroring.
    ///
    /// [`on_scroll`]: crate::vt100::Callbacks::on_scroll
    pub fn set_size(&mut self, rows: core::num::NonZeroU16, cols: core::num::NonZeroU16) {
        self.screen.set_size_with_cb(rows, cols);
    }

    /// Returns a reference to a [`Screen`](crate::vt100::Screen) object containing
    /// the terminal state.
    #[must_use]
    pub fn screen(&self) -> &crate::vt100::Screen {
        &self.screen.screen
    }

    /// Returns a mutable reference to a [`Screen`](crate::vt100::Screen) object
    /// containing the terminal state.
    #[must_use]
    pub fn screen_mut(&mut self) -> &mut crate::vt100::Screen {
        &mut self.screen.screen
    }

    /// Returns a reference to the [`Callbacks`](crate::vt100::Callbacks) state object
    /// passed into the constructor.
    pub fn callbacks(&self) -> &CB {
        &self.screen.callbacks
    }

    /// Returns a mutable reference to the [`Callbacks`](crate::vt100::Callbacks)
    /// state object passed into the constructor.
    pub fn callbacks_mut(&mut self) -> &mut CB {
        &mut self.screen.callbacks
    }

    /// Returns mutable references to both the [screen](Self::screen_mut) and
    /// [callbacks](Self::callbacks_mut).
    pub fn screen_and_callbacks_mut(&mut self) -> (&mut crate::vt100::Screen, &mut CB) {
        (&mut self.screen.screen, &mut self.screen.callbacks)
    }
}

impl Default for Parser {
    /// Returns a parser with dimensions 80x24 and no scrollback.
    fn default() -> Self {
        Self::new(Self::DEFAULT_ROWS, Self::DEFAULT_COLS, 0)
    }
}

impl std::io::Write for Parser {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
