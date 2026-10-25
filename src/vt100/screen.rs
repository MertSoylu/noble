use crate::vt100::term::BufWrite as _;
use alloc::{string::String, string::ToString as _};
use unicode_width::UnicodeWidthChar as _;

const MODE_APPLICATION_KEYPAD: u8 = 0b0000_0001;
const MODE_APPLICATION_CURSOR: u8 = 0b0000_0010;
const MODE_HIDE_CURSOR: u8 = 0b0000_0100;
const MODE_ALTERNATE_SCREEN: u8 = 0b0000_1000;
const MODE_BRACKETED_PASTE: u8 = 0b0001_0000;

/// The xterm mouse handling mode currently in use.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum MouseProtocolMode {
    /// Mouse handling is disabled.
    #[default]
    None,

    /// Mouse button events should be reported on button press. Also known as
    /// X10 mouse mode.
    Press,

    /// Mouse button events should be reported on button press and release.
    /// Also known as VT200 mouse mode.
    PressRelease,

    // Highlight,
    /// Mouse button events should be reported on button press and release, as
    /// well as when the mouse moves between cells while a button is held
    /// down.
    ButtonMotion,

    /// Mouse button events should be reported on button press and release,
    /// and mouse motion events should be reported when the mouse moves
    /// between cells regardless of whether a button is held down or not.
    AnyMotion,
    // DecLocator,
}

/// The encoding to use for the enabled [`MouseProtocolMode`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum MouseProtocolEncoding {
    /// Default single-printable-byte encoding.
    #[default]
    Default,

    /// UTF-8-based encoding.
    Utf8,

    /// SGR-like encoding.
    Sgr,
    // Urxvt,
}

/// The cursor's visual style.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum CursorStyle {
    /// The cursor should match the terminal's default.
    #[default]
    Default,

    /// The cursor should be a blinking block.
    BlinkBlock,

    /// The cursor should be a steady block.
    SteadyBlock,

    /// The cursor should be a blinking underline.
    BlinkUnderline,

    /// The cursor should be a steady underline.
    SteadyUnderline,

    /// The cursor should be a blinking vertical bar.
    BlinkVerticalBar,

    /// The cursor should be a steady vertical bar.
    SteadyVerticalBar,
}

/// Represents the overall terminal state.
#[derive(Clone, Debug)]
pub struct Screen {
    grid: crate::vt100::grid::Grid,
    alternate_grid: crate::vt100::grid::Grid,

    attrs: crate::vt100::attrs::Attrs,
    saved_attrs: crate::vt100::attrs::Attrs,

    modes: u8,
    mouse_protocol_mode: MouseProtocolMode,
    mouse_protocol_encoding: MouseProtocolEncoding,
    cursor_style: CursorStyle,

    last_char: Option<char>,
}

impl Screen {
    pub(crate) fn new(size: crate::vt100::grid::Size, scrollback_capacity: usize) -> Self {
        let mut grid = crate::vt100::grid::Grid::new(
            size,
            crate::vt100::grid::Scrollback::Enabled { capacity: scrollback_capacity },
        );
        grid.allocate_rows();
        grid.set_reflow(true);
        Self {
            grid,
            alternate_grid: crate::vt100::grid::Grid::new(size, crate::vt100::grid::Scrollback::Disabled),

            attrs: crate::vt100::attrs::Attrs::default(),
            saved_attrs: crate::vt100::attrs::Attrs::default(),

            modes: 0,
            mouse_protocol_mode: MouseProtocolMode::default(),
            mouse_protocol_encoding: MouseProtocolEncoding::default(),
            cursor_style: CursorStyle::default(),

            last_char: None,
        }
    }

    /// Resizes the screen **without calling** the [`on_scroll`] callback.
    ///
    /// This method behaves the same as [`Parser::set_size`] but does not call
    /// the [`on_scroll`] callback. If you want that, use [`Parser::set_size`].
    ///
    /// [`on_scroll`]: crate::vt100::Callbacks::on_scroll
    /// [`Parser::set_size`]: crate::vt100::Parser::set_size
    pub fn set_size(&mut self, rows: core::num::NonZeroU16, cols: core::num::NonZeroU16) {
        self.grid.set_size(crate::vt100::grid::Size { rows, cols }, |_| {});
        self.alternate_grid.set_size(crate::vt100::grid::Size { rows, cols }, |_| {});
    }

    /// NOBLE: turns the reflow of the main screen on a resize on or off (on by default; the alternate screen never
    /// reflows). Off, a width change cuts and pads every row.
    pub fn set_reflow(&mut self, reflow: bool) {
        self.grid.set_reflow(reflow);
    }

    /// NOBLE: whether a taller main screen pulls the newest history rows down (on by default, as xterm-like
    /// terminals with a bottom-anchored screen do). Off, the screen keeps its top row and gets blank rows at the
    /// bottom, which is how a ConPTY viewport grows.
    pub fn set_pull_history(&mut self, pull: bool) {
        self.grid.set_pull_history(pull);
    }

    /// NOBLE: whether a taller main screen pulls history down.
    #[must_use]
    pub fn pull_history(&self) -> bool {
        self.grid.pull_history()
    }

    /// NOBLE: whether the main screen reflows on a resize.
    #[must_use]
    pub fn reflow(&self) -> bool {
        self.grid.reflow()
    }

    /// Returns the current size of the terminal.
    ///
    /// The return value will be (rows, cols).
    #[must_use]
    pub fn size(&self) -> (core::num::NonZeroU16, core::num::NonZeroU16) {
        let size = self.grid().size();
        (size.rows, size.cols)
    }

    /// Returns the number of lines in the scrollback.
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.grid().scrollback_len()
    }

    /// NOBLE: the number of lines above the oldest scrollback line: scrolled out of a full scrollback, erased
    /// (`CSI 3 J`) or cut by a reflow. `history_dropped() + n` numbers the scrollback line `n` (0 = oldest) and
    /// `history_dropped() + scrollback_len() + row` a screen row, the same for as long as `reflow_epoch` stays the
    /// same, however much output follows or the screen grows (rows pulled back from the scrollback keep their
    /// number). `0` on the alternate screen.
    #[must_use]
    pub fn history_dropped(&self) -> usize {
        self.grid().history_dropped()
    }

    /// NOBLE: changes when a reflow or a reset renumbered the lines of the main screen, so a number taken from
    /// `history_dropped` before no longer points at the same text. A width change reflows; a height change and
    /// scrolling do not.
    #[must_use]
    pub fn reflow_epoch(&self) -> u64 {
        self.grid.epoch()
    }

    /// Returns the maximum capacity of the scrollback.
    #[must_use]
    pub fn scrollback_capacity(&self) -> usize {
        self.grid().scrollback_capacity()
    }

    /// Scrolls to the given position in the scrollback.
    ///
    /// This position indicates the offset from the top of the screen, and
    /// should be `0` to put the normal screen in view.
    ///
    /// This affects the return values of methods called on the screen: for
    /// instance, `screen.cell(0, 0)` will return the top left corner of the
    /// screen after taking the scrollback offset into account.
    ///
    /// The value given will be clamped to the actual size of the scrollback.
    pub fn set_scrollback(&mut self, rows: usize) {
        self.grid_mut().set_scrollback(rows);
    }

    /// Clears the scrollback history for the active screen and returns the
    /// scrollback position to the live viewport.
    ///
    /// This does not erase the visible screen contents, reset terminal
    /// modes, or call the `on_row` callback. To model a terminal action that
    /// clears both the visible display and scrollback history, process both
    /// `CSI 2 J` and `CSI 3 J`.
    pub fn clear_scrollback(&mut self) {
        self.grid_mut().erase_all_scrollback(|_| {});
    }

    /// Returns the current position in the scrollback.
    ///
    /// This position indicates the offset from the top of the screen, and is
    /// `0` when the normal screen is in view.
    #[must_use]
    pub fn scrollback(&self) -> usize {
        self.grid().scrollback()
    }

    /// Returns the text contents of the terminal.
    ///
    /// This will not include any formatting information, and will be in plain
    /// text format.
    #[must_use]
    pub fn contents(&self) -> String {
        let mut contents = String::new();
        self.write_contents(&mut contents);
        contents
    }

    fn write_contents(&self, contents: &mut String) {
        self.grid().write_contents(contents);
    }

    /// Returns the text contents of the terminal by row, restricted to the
    /// given subset of columns.
    ///
    /// This will not include any formatting information, and will be in plain
    /// text format.
    ///
    /// Newlines will not be included.
    pub fn rows(&self, start: u16, width: u16) -> impl Iterator<Item = String> + '_ {
        self.grid().visible_rows().map(move |row| {
            let mut contents = String::new();
            row.write_contents(&mut contents, start, width, false);
            contents
        })
    }

    /// Returns the text contents of the terminal logically between two cells.
    /// This will include the remainder of the starting row after `start_col`,
    /// followed by the entire contents of the rows between `start_row` and
    /// `end_row`, followed by the beginning of the `end_row` up until
    /// `end_col`. This is useful for things like determining the contents of
    /// a clipboard selection.
    #[must_use]
    pub fn contents_between(&self, start_row: u16, start_col: u16, end_row: u16, end_col: u16) -> String {
        match start_row.cmp(&end_row) {
            core::cmp::Ordering::Less => {
                let (_, cols) = self.size();
                let cols = cols.get();
                let mut contents = String::new();
                for (i, row) in self
                    .grid()
                    .visible_rows()
                    .enumerate()
                    .skip(usize::from(start_row))
                    .take(usize::from(end_row) - usize::from(start_row) + 1)
                {
                    if i == usize::from(start_row) {
                        row.write_contents(&mut contents, start_col, cols - start_col, false);
                        if !row.wrapped() {
                            contents.push('\n');
                        }
                    } else if i == usize::from(end_row) {
                        row.write_contents(&mut contents, 0, end_col, false);
                    } else {
                        row.write_contents(&mut contents, 0, cols, false);
                        if !row.wrapped() {
                            contents.push('\n');
                        }
                    }
                }
                contents
            }
            core::cmp::Ordering::Equal => {
                if start_col < end_col {
                    self.rows(start_col, end_col - start_col).nth(usize::from(start_row)).unwrap_or_default()
                } else {
                    String::new()
                }
            }
            core::cmp::Ordering::Greater => String::new(),
        }
    }

    /// Return escape codes sufficient to reproduce the entire contents of the
    /// current terminal state. This is a convenience wrapper around
    /// [`contents_formatted`](Self::contents_formatted) and
    /// [`input_mode_formatted`](Self::input_mode_formatted).
    #[must_use]
    pub fn state_formatted(&self) -> String {
        let mut contents = String::new();
        self.write_contents_formatted(&mut contents);
        self.write_input_mode_formatted(&mut contents);
        contents
    }

    /// Return escape codes sufficient to turn the terminal state of the
    /// screen `prev` into the current terminal state. This is a convenience
    /// wrapper around [`contents_diff`](Self::contents_diff) and
    /// [`input_mode_diff`](Self::input_mode_diff).
    #[must_use]
    pub fn state_diff(&self, prev: &Self) -> String {
        let mut contents = String::new();
        self.write_contents_diff(&mut contents, prev);
        self.write_input_mode_diff(&mut contents, prev);
        contents
    }

    /// Returns the formatted visible contents of the terminal.
    ///
    /// Formatting information will be included inline as terminal escape
    /// codes. The result will be suitable for feeding directly to a raw
    /// terminal parser, and will result in the same visual output.
    #[must_use]
    pub fn contents_formatted(&self) -> String {
        let mut contents = String::new();
        self.write_contents_formatted(&mut contents);
        contents
    }

    fn write_contents_formatted(&self, contents: &mut String) {
        crate::vt100::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        let prev_attrs = self.grid().write_contents_formatted(contents);
        self.attrs.write_escape_code_diff(contents, &prev_attrs);
    }

    /// Returns the formatted visible contents of the terminal in a "basic"
    /// format.
    ///
    /// The contents will contain no escape sequences except SGR sequences,
    /// and no control characters except `'\n'`.
    ///
    /// A newline will be inserted after each row that does not wrap onto the
    /// next line, except the last. Trailing whitespace will not be trimmed
    /// from the string; a blank terminal *n* rows tall will result in a string
    /// with *n* - 1 newlines.
    ///
    /// Terminal attributes should be reset before and after displaying the
    /// returned string; it will not begin or end with a reset sequence.
    #[must_use]
    pub fn contents_formatted_basic(&self) -> String {
        let mut contents = String::new();
        // Writing to a `String` cannot fail.
        #[allow(clippy::missing_panics_doc)]
        self.write_contents_formatted_basic(&mut contents, crate::vt100::capture::BasicFormattedCaptureRange::Visible)
            .unwrap();
        contents
    }

    /// Like [`Self::contents_formatted_basic`] but writes into the provided
    /// writer.
    ///
    /// For the `range` parameter, if you used the callback to write terminal
    /// data into this writer in a streaming fashion, pass
    /// [`BasicFormattedCaptureRange::Full`][full] with your existing
    /// [`BasicFormattedCaptureState`][state]. Otherwise, if you want the same
    /// result as [`Self::contents_formatted_basic`], pass
    /// [`BasicFormattedCaptureRange::Visible`][visible].
    ///
    /// [`on_scroll`]: crate::vt100::Callbacks::on_scroll
    /// [full]: crate::vt100::capture::BasicFormattedCaptureRange::Full
    /// [state]: crate::vt100::capture::BasicFormattedCaptureState
    /// [visible]: crate::vt100::capture::BasicFormattedCaptureRange::Visible
    ///
    /// # Errors
    ///
    /// If the writer returns an error, this method will forward that error.
    /// Otherwise, this method will not return any errors of its own.
    pub fn write_contents_formatted_basic(
        &self,
        writer: &mut impl alloc::fmt::Write,
        range: crate::vt100::capture::BasicFormattedCaptureRange<'_>,
    ) -> alloc::fmt::Result {
        self.grid().write_contents_formatted_basic(writer, range)
    }

    /// Returns the formatted visible contents of the terminal by row,
    /// restricted to the given subset of columns.
    ///
    /// Formatting information will be included inline as terminal escape
    /// codes. The result will be suitable for feeding directly to a raw
    /// terminal parser, and will result in the same visual output.
    ///
    /// You are responsible for positioning the cursor before printing each
    /// row, and the final cursor position after displaying each row is
    /// unspecified.
    // the unwraps in this method shouldn't be reachable
    #[allow(clippy::missing_panics_doc)]
    pub fn rows_formatted(&self, start: u16, width: u16) -> impl Iterator<Item = String> + '_ {
        let mut wrapping = false;
        self.grid().visible_rows().enumerate().map(move |(i, row)| {
            // number of rows in a grid is stored in a u16 (see Size), so
            // visible_rows can never return enough rows to overflow here
            let i = i.try_into().unwrap();
            let mut contents = String::new();
            row.write_contents_formatted(&mut contents, start, width, i, wrapping, None, None);
            if start == 0 && width == self.grid.size().cols() {
                wrapping = row.wrapped();
            }
            contents
        })
    }

    /// Returns a string containing escape sequences sufficient to turn the
    /// visible contents of the screen described by `prev` into the visible
    /// contents of the screen described by `self`.
    ///
    /// The result of rendering `prev.contents_formatted()` followed by
    /// `self.contents_diff(prev)` should be equivalent to the result of
    /// rendering `self.contents_formatted()`. This is primarily useful when
    /// you already have a terminal parser whose state is described by `prev`,
    /// since the diff will likely require less memory and cause less
    /// flickering than redrawing the entire screen contents.
    #[must_use]
    pub fn contents_diff(&self, prev: &Self) -> String {
        let mut contents = String::new();
        self.write_contents_diff(&mut contents, prev);
        contents
    }

    fn write_contents_diff(&self, contents: &mut String, prev: &Self) {
        if self.hide_cursor() != prev.hide_cursor() {
            crate::vt100::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        }
        let prev_attrs = self.grid().write_contents_diff(contents, prev.grid(), prev.attrs);
        self.attrs.write_escape_code_diff(contents, &prev_attrs);
    }

    /// Returns a sequence of strings containing escape sequences sufficient to
    /// turn the visible contents of the subset of each row from `prev` (as
    /// described by `start` and `width`) into the visible contents of the
    /// corresponding row subset in `self`.
    ///
    /// You are responsible for positioning the cursor before printing each
    /// row, and the final cursor position after displaying each row is
    /// unspecified.
    // the unwraps in this method shouldn't be reachable
    #[allow(clippy::missing_panics_doc)]
    pub fn rows_diff<'a>(&'a self, prev: &'a Self, start: u16, width: u16) -> impl Iterator<Item = String> + 'a {
        self.grid().visible_rows().zip(prev.grid().visible_rows()).enumerate().map(move |(i, (row, prev_row))| {
            // number of rows in a grid is stored in a u16 (see Size), so
            // visible_rows can never return enough rows to overflow here
            let i = i.try_into().unwrap();
            let mut contents = String::new();
            row.write_contents_diff(
                &mut contents,
                prev_row,
                start,
                width,
                i,
                false,
                false,
                crate::vt100::grid::Pos { row: i, col: start },
                crate::vt100::attrs::Attrs::default(),
            );
            contents
        })
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// terminal's input modes.
    ///
    /// Supported modes are:
    /// * application keypad
    /// * application cursor
    /// * bracketed paste
    /// * xterm mouse support
    #[must_use]
    pub fn input_mode_formatted(&self) -> String {
        let mut contents = String::new();
        self.write_input_mode_formatted(&mut contents);
        contents
    }

    fn write_input_mode_formatted(&self, contents: &mut String) {
        crate::vt100::term::ApplicationKeypad::new(self.mode(MODE_APPLICATION_KEYPAD)).write_buf(contents);
        crate::vt100::term::ApplicationCursor::new(self.mode(MODE_APPLICATION_CURSOR)).write_buf(contents);
        crate::vt100::term::BracketedPaste::new(self.mode(MODE_BRACKETED_PASTE)).write_buf(contents);
        crate::vt100::term::MouseProtocolMode::new(self.mouse_protocol_mode, MouseProtocolMode::None)
            .write_buf(contents);
        crate::vt100::term::MouseProtocolEncoding::new(self.mouse_protocol_encoding, MouseProtocolEncoding::Default)
            .write_buf(contents);
        crate::vt100::term::CursorStyle::new(self.cursor_style, CursorStyle::Default).write_buf(contents);
    }

    /// Returns terminal escape sequences sufficient to change the previous
    /// terminal's input modes to the input modes enabled in the current
    /// terminal.
    #[must_use]
    pub fn input_mode_diff(&self, prev: &Self) -> String {
        let mut contents = String::new();
        self.write_input_mode_diff(&mut contents, prev);
        contents
    }

    fn write_input_mode_diff(&self, contents: &mut String, prev: &Self) {
        if self.mode(MODE_APPLICATION_KEYPAD) != prev.mode(MODE_APPLICATION_KEYPAD) {
            crate::vt100::term::ApplicationKeypad::new(self.mode(MODE_APPLICATION_KEYPAD)).write_buf(contents);
        }
        if self.mode(MODE_APPLICATION_CURSOR) != prev.mode(MODE_APPLICATION_CURSOR) {
            crate::vt100::term::ApplicationCursor::new(self.mode(MODE_APPLICATION_CURSOR)).write_buf(contents);
        }
        if self.mode(MODE_BRACKETED_PASTE) != prev.mode(MODE_BRACKETED_PASTE) {
            crate::vt100::term::BracketedPaste::new(self.mode(MODE_BRACKETED_PASTE)).write_buf(contents);
        }
        crate::vt100::term::MouseProtocolMode::new(self.mouse_protocol_mode, prev.mouse_protocol_mode)
            .write_buf(contents);
        crate::vt100::term::MouseProtocolEncoding::new(self.mouse_protocol_encoding, prev.mouse_protocol_encoding)
            .write_buf(contents);
        crate::vt100::term::CursorStyle::new(self.cursor_style, prev.cursor_style).write_buf(contents);
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// terminal's drawing attributes.
    ///
    /// Supported drawing attributes are:
    /// * fgcolor
    /// * bgcolor
    /// * bold
    /// * dim
    /// * italic
    /// * underline
    /// * inverse
    ///
    /// This is not typically necessary, since
    /// [`contents_formatted`](Self::contents_formatted) will leave
    /// the current active drawing attributes in the correct state, but this
    /// can be useful in the case of drawing additional things on top of a
    /// terminal output, since you will need to restore the terminal state
    /// without the terminal contents necessarily being the same.
    #[must_use]
    pub fn attributes_formatted(&self) -> String {
        let mut contents = String::new();
        self.write_attributes_formatted(&mut contents);
        contents
    }

    fn write_attributes_formatted(&self, contents: &mut String) {
        crate::vt100::term::ClearAttrs.write_buf(contents);
        self.attrs.write_escape_code_diff(contents, &crate::vt100::attrs::Attrs::default());
    }

    /// Returns the current cursor position of the terminal.
    ///
    /// The return value will be (row, col).
    #[must_use]
    pub fn cursor_position(&self) -> (u16, u16) {
        let pos = self.grid().pos();
        (pos.row, pos.col)
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// cursor state of the terminal.
    ///
    /// This is not typically necessary, since
    /// [`contents_formatted`](Self::contents_formatted) will leave
    /// the cursor in the correct state, but this can be useful in the case of
    /// drawing additional things on top of a terminal output, since you will
    /// need to restore the terminal state without the terminal contents
    /// necessarily being the same.
    ///
    /// Note that the string returned by this function may alter the active
    /// drawing attributes, because it may require redrawing existing cells in
    /// order to position the cursor correctly (for instance, in the case
    /// where the cursor is past the end of a row). Therefore, you should
    /// ensure to reset the active drawing attributes if necessary after
    /// processing this data, for instance by using
    /// [`attributes_formatted`](Self::attributes_formatted).
    #[must_use]
    pub fn cursor_state_formatted(&self) -> String {
        let mut contents = String::new();
        self.write_cursor_state_formatted(&mut contents);
        contents
    }

    fn write_cursor_state_formatted(&self, contents: &mut String) {
        crate::vt100::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        self.grid().write_cursor_position_formatted(contents, None, None);

        // we don't just call write_attributes_formatted here, because that
        // would still be confusing - consider the case where the user sets
        // their own unrelated drawing attributes (on a different parser
        // instance) and then calls cursor_state_formatted. just documenting
        // it and letting the user handle it on their own is more
        // straightforward.
    }

    /// Returns the [`Cell`](crate::vt100::Cell) object at the given location in the
    /// terminal, if it exists.
    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&crate::vt100::Cell> {
        self.grid().visible_cell(crate::vt100::grid::Pos { row, col })
    }

    /// Returns whether the text in row `row` should wrap to the next line.
    #[must_use]
    pub fn row_wrapped(&self, row: u16) -> bool {
        self.grid().visible_row(row).is_some_and(crate::vt100::row::Row::wrapped)
    }

    /// Returns whether the alternate screen is currently in use.
    #[must_use]
    pub fn alternate_screen(&self) -> bool {
        self.mode(MODE_ALTERNATE_SCREEN)
    }

    /// Returns whether the terminal should be in application keypad mode.
    #[must_use]
    pub fn application_keypad(&self) -> bool {
        self.mode(MODE_APPLICATION_KEYPAD)
    }

    /// Returns whether the terminal should be in application cursor mode.
    #[must_use]
    pub fn application_cursor(&self) -> bool {
        self.mode(MODE_APPLICATION_CURSOR)
    }

    /// Returns whether the terminal should be in hide cursor mode.
    #[must_use]
    pub fn hide_cursor(&self) -> bool {
        self.mode(MODE_HIDE_CURSOR)
    }

    /// Returns whether the terminal should be in bracketed paste mode.
    #[must_use]
    pub fn bracketed_paste(&self) -> bool {
        self.mode(MODE_BRACKETED_PASTE)
    }

    /// Returns the currently active [`MouseProtocolMode`].
    #[must_use]
    pub fn mouse_protocol_mode(&self) -> MouseProtocolMode {
        self.mouse_protocol_mode
    }

    /// Returns the currently active [`MouseProtocolEncoding`].
    #[must_use]
    pub fn mouse_protocol_encoding(&self) -> MouseProtocolEncoding {
        self.mouse_protocol_encoding
    }

    /// Returns the currently active [`CursorStyle`].
    #[must_use]
    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    /// Returns the currently active foreground color.
    #[must_use]
    pub fn fgcolor(&self) -> crate::vt100::Color {
        self.attrs.fgcolor
    }

    /// Returns the currently active background color.
    #[must_use]
    pub fn bgcolor(&self) -> crate::vt100::Color {
        self.attrs.bgcolor
    }

    /// Returns whether newly drawn text should be rendered with the bold text
    /// attribute.
    #[must_use]
    pub fn bold(&self) -> bool {
        self.attrs.bold()
    }

    /// Returns whether newly drawn text should be rendered with the dim text
    /// attribute.
    #[must_use]
    pub fn dim(&self) -> bool {
        self.attrs.dim()
    }

    /// Returns whether newly drawn text should be rendered with the italic
    /// text attribute.
    #[must_use]
    pub fn italic(&self) -> bool {
        self.attrs.italic()
    }

    /// Returns whether newly drawn text should be rendered with the
    /// underlined text attribute.
    #[must_use]
    pub fn underline(&self) -> bool {
        self.attrs.underline()
    }

    /// Returns whether newly drawn text should be rendered with the inverse
    /// text attribute.
    #[must_use]
    pub fn inverse(&self) -> bool {
        self.attrs.inverse()
    }

    pub(crate) fn grid(&self) -> &crate::vt100::grid::Grid {
        if self.mode(MODE_ALTERNATE_SCREEN) { &self.alternate_grid } else { &self.grid }
    }

    fn grid_mut(&mut self) -> &mut crate::vt100::grid::Grid {
        if self.mode(MODE_ALTERNATE_SCREEN) { &mut self.alternate_grid } else { &mut self.grid }
    }

    fn enter_alternate_grid(&mut self) {
        self.grid_mut().set_scrollback(0);
        self.set_mode(MODE_ALTERNATE_SCREEN);
        self.alternate_grid.allocate_rows();
    }

    fn exit_alternate_grid(&mut self) {
        self.clear_mode(MODE_ALTERNATE_SCREEN);
    }

    fn save_cursor(&mut self) {
        self.grid_mut().save_cursor();
        self.saved_attrs = self.attrs;
    }

    fn restore_cursor(&mut self) {
        self.grid_mut().restore_cursor();
        self.attrs = self.saved_attrs;
    }

    fn set_mode(&mut self, mode: u8) {
        self.modes |= mode;
    }

    fn clear_mode(&mut self, mode: u8) {
        self.modes &= !mode;
    }

    fn mode(&self, mode: u8) -> bool {
        self.modes & mode != 0
    }

    fn set_mouse_mode(&mut self, mode: MouseProtocolMode) {
        self.mouse_protocol_mode = mode;
    }

    fn clear_mouse_mode(&mut self, mode: MouseProtocolMode) {
        if self.mouse_protocol_mode == mode {
            self.mouse_protocol_mode = MouseProtocolMode::default();
        }
    }

    fn set_mouse_encoding(&mut self, encoding: MouseProtocolEncoding) {
        self.mouse_protocol_encoding = encoding;
    }

    fn clear_mouse_encoding(&mut self, encoding: MouseProtocolEncoding) {
        if self.mouse_protocol_encoding == encoding {
            self.mouse_protocol_encoding = MouseProtocolEncoding::default();
        }
    }

    fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    // determine the width of the character we're about to draw, and whether
    // it makes sense to even attempt to draw a character of that width.
    fn calculate_width(&self, c: char) -> Option<u16> {
        let pos = self.grid().pos();
        let size = self.grid().size();

        let width = c.width();
        if width.is_none() && (u32::from(c)) < 256 {
            // don't even try to draw control characters
            return None;
        }
        let width = u16::try_from(width.unwrap_or(1))
            // char::width() can only return 0, 1, 2, or 3
            .unwrap();
        // TODO: this is invalid for U+17D8 which has a width of 3 for some
        // reason, but adding triple width characters is going to be a
        // massive pain just for this one character
        let width = width.min(2);

        // if the character is wider than the screen, we can't draw it, so
        // just ignore it
        if width > size.cols() {
            return None;
        }

        // a continuation character without a base character before it is
        // meaningless
        if width == 0 && pos.col == 0 {
            return None;
        }

        Some(width)
    }

    // if this is the initial codepoint in a new cell, we need to determine
    // whether wrapping that character to the next row should set the row's
    // wrapping flag
    fn new_character_should_wrap(&self, width: u16) -> bool {
        let pos = self.grid().pos();
        let size = self.grid().size();

        // it doesn't make any sense to wrap if the last column in a row
        // didn't already have contents. don't try to handle the case
        // where a character wraps because there was only one column left
        // in the previous row - literally everything handles this case
        // differently, and this is tmux behavior (and also the
        // simplest). i'm open to reconsidering this behavior, but only
        // with a really good reason (xterm handles this by introducing
        // the concept of triple width cells, which i really don't want
        // to do).
        let mut wrap = false;
        if pos.col + width > size.cols() {
            let last_cell = self
                .grid()
                .drawing_cell(crate::vt100::grid::Pos { row: pos.row, col: size.cols() - 1 })
                // pos.row is valid, since it comes directly from
                // self.grid().pos() which we assume to always have a
                // valid row value. size.cols - 1 is also always a valid
                // column.
                .unwrap();
            if last_cell.has_contents() || last_cell.is_wide_continuation() {
                wrap = true;
            } else if width > 1 && pos.col + 1 == size.cols() {
                // NOBLE: a wide character that does not fit in the one cell left moves to the next row. The
                // row still continues there (`Grid::col_wrap` marks the empty cell as a leading spacer).
                wrap = true;
            }
        }

        wrap
    }

    // if we already have a character in a cell, but adding a continuation
    // character widened the cell off the end of the row, we need to move the
    // base character to the beginning of the next row
    fn wrap_widened_cell(&mut self, old_pos: crate::vt100::grid::Pos, new_pos: crate::vt100::grid::Pos) {
        if new_pos.col == 0 {
            // `col_wrap` may have scrolled the grid, so the row the cell was on is the one above the new one (a
            // one-row grid has none and keeps the position it had)
            let old_pos = if new_pos.row > 0 {
                crate::vt100::grid::Pos { row: new_pos.row - 1, col: old_pos.col }
            } else {
                old_pos
            };
            let attrs = self.attrs;
            let contents = self.grid().drawing_cell(old_pos).unwrap().contents().to_string();
            self.grid_mut().drawing_cell_mut(new_pos).unwrap().set_contents(&contents, attrs);
            let old_cell = self.grid_mut().drawing_cell_mut(old_pos).unwrap();
            old_cell.clear(attrs);
            if new_pos.row > 0 {
                // NOBLE: the emptied cell is the spacer of a row that continues on the next one
                old_cell.set_leading_spacer(true);
                self.grid_mut().drawing_row_mut(old_pos.row).unwrap().wrap(true);
            }
            self.grid_mut().col_inc(2);
        } else {
            self.grid_mut().col_inc(1);
        }
    }

    // once we have drawn a character, clean up the wide character state of
    // nearby cells that may have been disrupted.
    fn text_cleanup(&mut self, base_pos: crate::vt100::grid::Pos, cell_was_wide: bool, cell_is_wide: bool) {
        let size = self.grid().size();
        let attrs = self.attrs;

        if base_pos.col > 0 {
            let prev_cell = self
                .grid_mut()
                .drawing_cell_mut(crate::vt100::grid::Pos { row: base_pos.row, col: base_pos.col - 1 })
                // pos.row is assumed valid, and pos.col was just
                // checked to be greater than 0.
                .unwrap();
            if prev_cell.is_wide() {
                prev_cell.clear(*prev_cell.attrs());
            }
        }

        if cell_was_wide && !cell_is_wide {
            let next_cell = self
                .grid_mut()
                .drawing_cell_mut(crate::vt100::grid::Pos { row: base_pos.row, col: base_pos.col + 1 })
                // base_pos.row is assumed valid, and this cell previously
                // was wide, so there must have been a continuation cell
                // after it.
                .unwrap();
            next_cell.set(' ', attrs);
        }

        if !cell_was_wide && cell_is_wide {
            let next_cell = self
                .grid_mut()
                .drawing_cell_mut(crate::vt100::grid::Pos { row: base_pos.row, col: base_pos.col + 1 })
                // base_pos.row is assumed valid, and the cell at base_pos
                // has become wide, so it must have a cell after it (if it
                // didn't, we would have wrapped earlier when widening the
                // cell).
                .unwrap();
            let next_cell_was_wide = next_cell.is_wide();
            next_cell.clear(crate::vt100::attrs::Attrs::default());
            next_cell.set_wide_continuation(true);
            if next_cell_was_wide {
                let next_next_cell = self
                    .grid_mut()
                    .drawing_cell_mut(crate::vt100::grid::Pos { row: base_pos.row, col: base_pos.col + 2 })
                    // the cell at base_pos.col + 1 was previously wide, so
                    // it must have had a continuation cell after it.
                    .unwrap();
                next_next_cell.clear(attrs);
                if base_pos.col + 2 == size.cols() - 1 {
                    self.grid_mut().drawing_row_mut(base_pos.row).unwrap().wrap(false);
                }
            }
        }
    }
}

pub struct WrappedScreen<CB: crate::vt100::Callbacks> {
    pub screen: Screen,
    pub callbacks: CB,
}

impl<CB: crate::vt100::Callbacks> WrappedScreen<CB> {
    /// Gets the `on_row` callback for methods like
    /// [`crate::vt100::grid::Grid::scroll_up`].
    fn on_row<'cb>(screen: &Screen, callbacks: &'cb mut CB) -> impl FnMut(&crate::vt100::row::Row) + 'cb {
        let is_alternate = screen.mode(MODE_ALTERNATE_SCREEN);
        move |row| {
            callbacks.on_scroll(crate::vt100::capture::RowContents(row), is_alternate);
        }
    }

    /// Resizes the screen, calling [`on_scroll`] for each row that is pushed
    /// off the top of the screen's scrollback.
    ///
    /// [`on_scroll`]: crate::vt100::Callbacks::on_scroll
    pub fn set_size_with_cb(&mut self, rows: core::num::NonZeroU16, cols: core::num::NonZeroU16) {
        let callbacks = &mut self.callbacks;
        self.screen.grid.set_size(crate::vt100::grid::Size { rows, cols }, |row| {
            callbacks.on_scroll(crate::vt100::capture::RowContents(row), false);
        });
        self.screen.alternate_grid.set_size(crate::vt100::grid::Size { rows, cols }, |row| {
            callbacks.on_scroll(crate::vt100::capture::RowContents(row), true);
        });
    }

    pub fn text(&mut self, c: char) {
        let screen = &mut self.screen;
        let pos = screen.grid().pos();
        let attrs = screen.attrs;

        let Some(width) = screen.calculate_width(c) else {
            return;
        };

        // base_pos will always be set to a valid cell, so
        // .drawing_cell(base_pos).unwrap() will always be valid
        let mut base_pos;
        let cell_was_wide;

        if width > 0 {
            let wrap = screen.new_character_should_wrap(width);
            let on_row = Self::on_row(screen, &mut self.callbacks);
            screen.grid_mut().col_wrap(width, wrap, on_row);
            // col_wrap always sets self.grid().pos() to a valid cell
            base_pos = screen.grid().pos();
            cell_was_wide = screen.grid().drawing_cell(base_pos).unwrap().is_wide();

            screen.grid_mut().drawing_cell_mut(base_pos).unwrap().set(c, attrs);
            screen.last_char = Some(c);
            screen.grid_mut().col_inc(width);
        } else {
            // pos.row is valid, since it comes directly from
            // self.grid().pos() which we assume to always have a
            // valid row value. pos.col - 1 is valid because we just
            // returned early if pos.col == 0.
            base_pos = crate::vt100::grid::Pos { row: pos.row, col: pos.col - 1 };
            if screen.grid_mut().drawing_cell_mut(base_pos).unwrap().is_wide_continuation() {
                // pos.row is valid, since it comes directly from
                // self.grid().pos() which we assume to always have a
                // valid row value. we know pos.col - 2 is valid
                // because the cell at pos.col - 1 is a wide
                // continuation character, which means there must be
                // the first half of the wide character before it.
                base_pos = crate::vt100::grid::Pos { row: pos.row, col: pos.col - 2 };
            }
            cell_was_wide = screen.grid().drawing_cell(base_pos).unwrap().is_wide();

            screen.grid_mut().drawing_cell_mut(base_pos).unwrap().append(c);
            let cell_is_wide = screen.grid().drawing_cell(base_pos).unwrap().is_wide();
            if !cell_was_wide && cell_is_wide {
                let on_row = Self::on_row(screen, &mut self.callbacks);
                screen.grid_mut().col_wrap(1, false, on_row);
                // col_wrap always leaves self.grid().pos() on a valid cell,
                // so .drawing_cell(new_pos).unwrap() will always be valid.
                let new_pos = screen.grid().pos();
                screen.wrap_widened_cell(base_pos, new_pos);
                if new_pos.col == 0 {
                    base_pos = new_pos;
                }
            }
        }

        let cell_is_wide = screen.grid().drawing_cell(base_pos).unwrap().is_wide();

        screen.text_cleanup(base_pos, cell_was_wide, cell_is_wide);
    }

    // control codes

    pub fn bs(&mut self) {
        self.screen.grid_mut().col_dec(1);
    }

    pub fn tab(&mut self) {
        self.screen.grid_mut().col_tab();
    }

    pub fn lf(&mut self) {
        let on_row = Self::on_row(&self.screen, &mut self.callbacks);
        self.screen.grid_mut().row_inc_scroll(1, on_row);
    }

    pub fn vt(&mut self) {
        self.lf();
    }

    pub fn ff(&mut self) {
        self.lf();
    }

    pub fn cr(&mut self) {
        self.screen.grid_mut().col_set(0);
    }

    // escape codes

    // ESC 7
    pub fn decsc(&mut self) {
        self.screen.save_cursor();
    }

    // ESC 8
    pub fn decrc(&mut self) {
        self.screen.restore_cursor();
    }

    // ESC =
    pub fn deckpam(&mut self) {
        self.screen.set_mode(MODE_APPLICATION_KEYPAD);
    }

    // ESC >
    pub fn deckpnm(&mut self) {
        self.screen.clear_mode(MODE_APPLICATION_KEYPAD);
    }

    // ESC M
    pub fn ri(&mut self) {
        self.screen.grid_mut().row_dec_scroll(1);
    }

    // ESC c
    pub fn ris(&mut self) {
        let epoch = self.screen.grid.epoch();
        let (reflow, pull) = (self.screen.reflow(), self.screen.pull_history());
        self.screen = Screen::new(self.screen.grid.size(), self.screen.grid.scrollback_capacity());
        // The settings of the embedding program survive the reset.
        self.screen.set_reflow(reflow);
        self.screen.set_pull_history(pull);
        // The lines start from 0 again.
        self.screen.grid.continue_epoch(epoch);
    }

    // csi codes

    // CSI SP
    pub(crate) fn decscusr(&mut self, cursor_style: u16, mut unhandled: impl FnMut(&mut Self)) {
        match cursor_style {
            0 => self.screen.set_cursor_style(CursorStyle::Default),
            1 => self.screen.set_cursor_style(CursorStyle::BlinkBlock),
            2 => self.screen.set_cursor_style(CursorStyle::SteadyBlock),
            3 => self.screen.set_cursor_style(CursorStyle::BlinkUnderline),
            4 => self.screen.set_cursor_style(CursorStyle::SteadyUnderline),
            5 => self.screen.set_cursor_style(CursorStyle::BlinkVerticalBar),
            6 => self.screen.set_cursor_style(CursorStyle::SteadyVerticalBar),
            _ => unhandled(self),
        }
    }

    // CSI @
    pub fn ich(&mut self, count: u16) {
        self.screen.grid_mut().insert_cells(count);
    }

    // CSI A
    pub fn cuu(&mut self, offset: u16) {
        self.screen.grid_mut().row_dec_clamp(offset);
    }

    // CSI B
    pub fn cud(&mut self, offset: u16) {
        self.screen.grid_mut().row_inc_clamp(offset);
    }

    // CSI C
    pub fn cuf(&mut self, offset: u16) {
        self.screen.grid_mut().col_inc_clamp(offset);
    }

    // CSI D
    pub fn cub(&mut self, offset: u16) {
        self.screen.grid_mut().col_dec(offset);
    }

    // CSI E
    pub fn cnl(&mut self, offset: u16) {
        self.screen.grid_mut().col_set(0);
        self.screen.grid_mut().row_inc_clamp(offset);
    }

    // CSI F
    pub fn cpl(&mut self, offset: u16) {
        self.screen.grid_mut().col_set(0);
        self.screen.grid_mut().row_dec_clamp(offset);
    }

    // CSI G
    pub fn cha(&mut self, col: u16) {
        self.screen.grid_mut().col_set(col - 1);
    }

    // CSI `  (HPA - Horizontal Position Absolute, equivalent to CHA)
    pub(crate) fn hpa(&mut self, col: u16) {
        self.cha(col);
    }

    // CSI b (REP - Repeat last printed character)
    pub(crate) fn rep(&mut self, count: u16) {
        if let Some(c) = self.screen.last_char {
            for _ in 0..count {
                self.text(c);
            }
        }
    }

    // CSI H
    pub fn cup(&mut self, (row, col): (u16, u16)) {
        self.screen.grid_mut().set_pos(crate::vt100::grid::Pos { row: row - 1, col: col - 1 });
    }

    // CSI J
    pub fn ed(&mut self, mode: u16, mut unhandled: impl FnMut(&mut Self)) {
        let attrs = self.screen.attrs;
        match mode {
            0 => self.screen.grid_mut().erase_all_forward(attrs),
            1 => self.screen.grid_mut().erase_all_backward(attrs),
            2 => self.screen.grid_mut().erase_all(attrs),
            3 => {
                let on_row = Self::on_row(&self.screen, &mut self.callbacks);
                self.screen.grid_mut().erase_all_scrollback(on_row);
            }
            _ => unhandled(self),
        }
    }

    // CSI ? J
    pub fn decsed(&mut self, mode: u16, unhandled: impl FnMut(&mut Self)) {
        self.ed(mode, unhandled);
    }

    // CSI K
    pub fn el(&mut self, mode: u16, mut unhandled: impl FnMut(&mut Self)) {
        let attrs = self.screen.attrs;
        match mode {
            0 => self.screen.grid_mut().erase_row_forward(attrs),
            1 => self.screen.grid_mut().erase_row_backward(attrs),
            2 => self.screen.grid_mut().erase_row(attrs),
            _ => unhandled(self),
        }
    }

    // CSI ? K
    pub fn decsel(&mut self, mode: u16, unhandled: impl FnMut(&mut Self)) {
        self.el(mode, unhandled);
    }

    // CSI L
    pub fn il(&mut self, count: u16) {
        self.screen.grid_mut().insert_lines(count);
    }

    // CSI M
    pub fn dl(&mut self, count: u16) {
        self.screen.grid_mut().delete_lines(count);
    }

    // CSI P
    pub fn dch(&mut self, count: u16) {
        self.screen.grid_mut().delete_cells(count);
    }

    // CSI S
    pub fn su(&mut self, count: u16) {
        let on_row = Self::on_row(&self.screen, &mut self.callbacks);
        self.screen.grid_mut().scroll_up(count, on_row);
    }

    // CSI T
    pub fn sd(&mut self, count: u16) {
        self.screen.grid_mut().scroll_down(count);
    }

    // CSI X
    pub fn ech(&mut self, count: u16) {
        let attrs = self.screen.attrs;
        self.screen.grid_mut().erase_cells(count, attrs);
    }

    // CSI d
    pub fn vpa(&mut self, row: u16) {
        self.screen.grid_mut().row_set(row - 1);
    }

    // CSI ? h
    pub fn decset(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            let screen = &mut self.screen;
            match param {
                [1] => screen.set_mode(MODE_APPLICATION_CURSOR),
                [6] => screen.grid_mut().set_origin_mode(true),
                [9] => screen.set_mouse_mode(MouseProtocolMode::Press),
                [25] => screen.clear_mode(MODE_HIDE_CURSOR),
                [47] => screen.enter_alternate_grid(),
                [1000] => {
                    screen.set_mouse_mode(MouseProtocolMode::PressRelease);
                }
                [1002] => {
                    screen.set_mouse_mode(MouseProtocolMode::ButtonMotion);
                }
                [1003] => screen.set_mouse_mode(MouseProtocolMode::AnyMotion),
                [1005] => {
                    screen.set_mouse_encoding(MouseProtocolEncoding::Utf8);
                }
                [1006] => {
                    screen.set_mouse_encoding(MouseProtocolEncoding::Sgr);
                }
                [1049] => {
                    self.decsc();
                    self.screen.alternate_grid.clear();
                    self.screen.enter_alternate_grid();
                }
                [2004] => screen.set_mode(MODE_BRACKETED_PASTE),
                _ => unhandled(self),
            }
        }
    }

    // CSI ? l
    pub fn decrst(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            let screen = &mut self.screen;
            match param {
                [1] => screen.clear_mode(MODE_APPLICATION_CURSOR),
                [6] => screen.grid_mut().set_origin_mode(false),
                [9] => screen.clear_mouse_mode(MouseProtocolMode::Press),
                [25] => screen.set_mode(MODE_HIDE_CURSOR),
                [47] => {
                    screen.exit_alternate_grid();
                }
                [1000] => {
                    screen.clear_mouse_mode(MouseProtocolMode::PressRelease);
                }
                [1002] => {
                    screen.clear_mouse_mode(MouseProtocolMode::ButtonMotion);
                }
                [1003] => {
                    screen.clear_mouse_mode(MouseProtocolMode::AnyMotion);
                }
                [1005] => {
                    screen.clear_mouse_encoding(MouseProtocolEncoding::Utf8);
                }
                [1006] => {
                    screen.clear_mouse_encoding(MouseProtocolEncoding::Sgr);
                }
                [1049] => {
                    screen.exit_alternate_grid();
                    self.decrc();
                }
                [2004] => screen.clear_mode(MODE_BRACKETED_PASTE),
                _ => unhandled(self),
            }
        }
    }

    // CSI m
    pub fn sgr(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        // XXX really i want to just be able to pass in a default Params
        // instance with a 0 in it, but vte doesn't allow creating new Params
        // instances
        if params.is_empty() {
            self.screen.attrs = crate::vt100::attrs::Attrs::default();
            return;
        }

        let mut iter = params.iter();

        macro_rules! next_param {
            () => {
                match iter.next() {
                    Some(n) => n,
                    _ => return,
                }
            };
        }

        macro_rules! to_u8 {
            ($n:expr) => {
                if let Some(n) = u16_to_u8($n) {
                    n
                } else {
                    return;
                }
            };
        }

        macro_rules! next_param_u8 {
            () => {
                if let &[n] = next_param!() {
                    to_u8!(n)
                } else {
                    return;
                }
            };
        }

        loop {
            let screen = &mut self.screen;
            match next_param!() {
                [0] => screen.attrs = crate::vt100::attrs::Attrs::default(),
                [1] => screen.attrs.set_bold(),
                [2] => screen.attrs.set_dim(),
                [3] => screen.attrs.set_italic(true),
                [4] => screen.attrs.set_underline(true),
                [7] => screen.attrs.set_inverse(true),
                [22] => screen.attrs.set_normal_intensity(),
                [23] => screen.attrs.set_italic(false),
                [24] => screen.attrs.set_underline(false),
                [27] => screen.attrs.set_inverse(false),
                [n] if (30..=37).contains(n) => {
                    screen.attrs.fgcolor = crate::vt100::Color::Idx(to_u8!(*n) - 30);
                }
                [38, 2, r, g, b] => {
                    screen.attrs.fgcolor = crate::vt100::Color::Rgb(to_u8!(*r), to_u8!(*g), to_u8!(*b));
                }
                [38, 5, i] => {
                    screen.attrs.fgcolor = crate::vt100::Color::Idx(to_u8!(*i));
                }
                [38] => match next_param!() {
                    [2] => {
                        let r = next_param_u8!();
                        let g = next_param_u8!();
                        let b = next_param_u8!();
                        screen.attrs.fgcolor = crate::vt100::Color::Rgb(r, g, b);
                    }
                    [5] => {
                        screen.attrs.fgcolor = crate::vt100::Color::Idx(next_param_u8!());
                    }
                    _ => {
                        unhandled(self);
                        return;
                    }
                },
                [39] => {
                    screen.attrs.fgcolor = crate::vt100::Color::Default;
                }
                [n] if (40..=47).contains(n) => {
                    screen.attrs.bgcolor = crate::vt100::Color::Idx(to_u8!(*n) - 40);
                }
                [48, 2, r, g, b] => {
                    screen.attrs.bgcolor = crate::vt100::Color::Rgb(to_u8!(*r), to_u8!(*g), to_u8!(*b));
                }
                [48, 5, i] => {
                    screen.attrs.bgcolor = crate::vt100::Color::Idx(to_u8!(*i));
                }
                [48] => match next_param!() {
                    [2] => {
                        let r = next_param_u8!();
                        let g = next_param_u8!();
                        let b = next_param_u8!();
                        screen.attrs.bgcolor = crate::vt100::Color::Rgb(r, g, b);
                    }
                    [5] => {
                        screen.attrs.bgcolor = crate::vt100::Color::Idx(next_param_u8!());
                    }
                    _ => {
                        unhandled(self);
                        return;
                    }
                },
                [49] => {
                    screen.attrs.bgcolor = crate::vt100::Color::Default;
                }
                [n] if (90..=97).contains(n) => {
                    screen.attrs.fgcolor = crate::vt100::Color::Idx(to_u8!(*n) - 82);
                }
                [n] if (100..=107).contains(n) => {
                    screen.attrs.bgcolor = crate::vt100::Color::Idx(to_u8!(*n) - 92);
                }
                _ => unhandled(self),
            }
        }
    }

    // CSI r
    pub fn decstbm(&mut self, (top, bottom): (u16, u16)) {
        self.screen.grid_mut().set_scroll_region(top - 1, bottom - 1);
    }
}

fn u16_to_u8(i: u16) -> Option<u8> {
    if i > u16::from(u8::MAX) {
        None
    } else {
        // safe because we just ensured that the value fits in a u8
        Some(i.try_into().unwrap())
    }
}
