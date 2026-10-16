//! The keyboard reference (`?`): its rows, the filter typed into it, the column layout and the
//! scroll geometry. Drawing is in `ui/overlay.rs`; keys and clicks are in `app/input.rs`. Both read
//! the same rows, so a click or ⏎ runs the row that is drawn.

use std::ops::Range;

use ratatui::layout::Rect;

use super::App;
use crate::keys::Action;
use crate::util;

/// Width of the key column of one row.
pub const KEY_W: usize = 18;
/// Longest description that sets the width of a column (longer ones are cut with "…").
const DESC_CAP: usize = 56;
/// Blank columns between two columns of the list.
pub const GAP: u16 = 3;

/// Typed state of the overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HelpState {
    pub query: String,
    /// Index into the selectable rows (the rows with an action, in reading order).
    pub selected: usize,
    /// First visible line of every column.
    pub scroll: usize,
}

/// One key and what it does. `action` is `None` for rows that only explain something (mouse tips).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: String,
    pub desc: String,
    pub action: Option<Action>,
}

/// A titled group of rows; an empty title is a group without a header.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

/// One drawn line of a column.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Header(String),
    /// `sel` is the row's index among the selectable rows, `None` when it has no action.
    Row {
        key: String,
        desc: String,
        sel: Option<usize>,
    },
    Gap,
}

/// Where the overlay box is and what is inside it at the current size.
#[derive(Debug)]
pub struct Geometry {
    pub rect: Rect,
    /// Text width of one column.
    pub col_w: u16,
    /// Lines shown per column (the footer takes the last row of the box).
    pub rows: usize,
    pub columns: Vec<Vec<Line>>,
}

impl Geometry {
    /// The most the list can scroll: the tallest column minus the rows shown.
    pub fn max_scroll(&self) -> usize {
        self.columns.iter().map(Vec::len).max().unwrap_or(0).saturating_sub(self.rows)
    }

    /// The column and the line of the selectable row `sel`.
    pub fn line_of(&self, sel: usize) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(c, col)| {
            col.iter().position(|l| matches!(l, Line::Row { sel: Some(s), .. } if *s == sel)).map(|i| (c, i))
        })
    }
}

/// The rows of the reference, as the keymap has them now.
pub fn sections(app: &App) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let row = |key: String, desc: String, action: Option<Action>| Row { key, desc, action };
    let prefix = app.keymap.prefix.to_string();

    let mut prefix_rows = Vec::new();
    let mut by_action: Vec<(Action, Vec<String>)> = Vec::new();
    for a in Action::ALL {
        let mut keys: Vec<String> =
            app.keymap.prefix_map.iter().filter(|(_, x)| **x == a).map(|(k, _)| k.to_string()).collect();
        if keys.is_empty() {
            continue;
        }
        keys.sort_by_key(|k| (k.len(), k.clone()));
        by_action.push((a, keys));
    }
    // The tab numbers share one row (rebound ones keep their own rows).
    let tab_range = app.keymap.tab_range(true);
    let mut tabs_done = false;
    for (a, keys) in &by_action {
        if matches!(a, Action::GoTab(_))
            && let Some(range) = &tab_range
        {
            if !tabs_done {
                prefix_rows.push(row(range.clone(), "go to tab".into(), None));
                tabs_done = true;
            }
            continue;
        }
        prefix_rows.push(row(keys.join("  "), a.title(), Some(*a)));
    }
    prefix_rows.push(row(prefix.clone(), "send prefix to the shell".into(), None));
    out.push(Section { title: format!("PREFIX · press {prefix}, then"), rows: prefix_rows });

    let mut direct: Vec<(String, Action)> = app.keymap.direct_map.iter().map(|(k, a)| (k.to_string(), *a)).collect();
    direct.sort_by_key(|(k, _)| k.clone());
    let mut global_rows = Vec::new();
    let tab_range = app.keymap.tab_range(false);
    let mut tabs_done = false;
    for (k, a) in direct {
        if matches!(a, Action::GoTab(_))
            && let Some(range) = &tab_range
        {
            if !tabs_done {
                global_rows.push(row(range.clone(), "go to tab".into(), None));
                tabs_done = true;
            }
            continue;
        }
        // Shell keys reach the shell in a terminal (`keys.shell_first`).
        let title = if app.keymap.shell_first.iter().any(|c| c.to_string() == k) {
            format!("{} (not in terminals)", a.title())
        } else {
            a.title()
        };
        global_rows.push(row(k, title, Some(a)));
    }
    out.push(Section { title: "GLOBAL".into(), rows: global_rows });

    let plain = |pairs: &[(&str, &str)]| -> Vec<Row> {
        pairs.iter().map(|(k, d)| row((*k).into(), (*d).into(), None)).collect()
    };
    out.push(Section {
        title: "HOME".into(),
        rows: {
            let mut rows = plain(&[
                ("↑ ↓  j k", "select project"),
                ("⏎  double-click", "open a terminal in the project"),
                ("→  ⏎", "pin to top ★ · more actions ⋯ (← → choose)"),
                ("/", "search projects"),
                ("● 3  ✓  ↑ ↓  …", "uncommitted changes · clean · commits to push / pull · checking"),
                ("t", "terminal in home folder"),
                ("o", "open folder in file manager"),
                ("w", "save open tabs as a workspace"),
                ("a  A", "add a folder to scan for projects · add one project folder"),
                ("⋯  remove", "Remove from list: hide a project for good (A adds it back)"),
                ("r  R", "rescan projects · refresh AI usage"),
                ("m  s", "system · settings"),
                ("p  ?  q", "commands · help · quit"),
            ]);
            for (_, l) in app.quick_launchers() {
                rows.push(row(l.key.clone(), format!("run {} in the project", l.command), None));
            }
            rows
        },
    });

    out.push(Section {
        title: "SYSTEM".into(),
        rows: plain(&[
            ("↑ ↓  pgup pgdn", "select process"),
            ("c m p n", "sort by cpu · mem · pid · name"),
            ("/", "filter"),
            ("K  del", "terminate (asks first)"),
            ("esc", "back"),
        ]),
    });

    out.push(Section {
        title: "MOUSE".into(),
        rows: plain(&[
            ("click", "everything: tabs, rows, buttons, settings"),
            ("◫ ⊟ ⤢ ✕", "pane buttons: split right · split down · zoom · close (focused or hovered pane)"),
            ("double-click title", "zoom / restore the pane"),
            ("middle-click tab", "close the tab"),
            ("drag border", "resize panes"),
            ("drag text", "select (copied on release)"),
            ("right-click", "paste into terminal"),
            ("ctrl+click", "open a link or file:line (underlined while ctrl is held)"),
            ("wheel", "scroll history / lists"),
            ("shift+drag", "select even when the app captures the mouse"),
        ]),
    });
    // The config file's location: a row of its own, below the groups.
    out.push(Section { title: String::new(), rows: plain(&[("config", &util::tilde(&app.paths.config))]) });
    out
}

/// The sections that keep rows matching `query` (key or description, case-insensitive). A section
/// without a matching row is dropped, so its header goes with it.
pub fn filter(sections: Vec<Section>, query: &str) -> Vec<Section> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return sections;
    }
    sections
        .into_iter()
        .filter_map(|s| {
            let rows: Vec<Row> = s
                .rows
                .into_iter()
                .filter(|r| r.key.to_lowercase().contains(&q) || r.desc.to_lowercase().contains(&q))
                .collect();
            (!rows.is_empty()).then_some(Section { title: s.title, rows })
        })
        .collect()
}

/// Columns the box is laid out in at this screen width.
fn column_count(screen_w: u16) -> usize {
    if screen_w >= 170 {
        3
    } else if screen_w >= 120 {
        2
    } else {
        1
    }
}

/// Splits the sections (in order, each whole) into `k` columns so that the tallest one is as short
/// as it can be. `heights[i]` is the height of section `i` without the gaps between sections.
fn split(heights: &[usize], k: usize) -> Vec<Range<usize>> {
    let n = heights.len();
    if n == 0 {
        return Vec::new();
    }
    let k = k.clamp(1, n);
    // A column holding sections `a..b` is their heights plus one gap between each two of them.
    let cost = |a: usize, b: usize| heights[a..b].iter().sum::<usize>() + (b - a - 1);
    // best[j][i]: the shortest tallest column when the first `i` sections fill `j` columns.
    const NONE: usize = usize::MAX;
    let mut best = vec![vec![NONE; n + 1]; k + 1];
    let mut from = vec![vec![0usize; n + 1]; k + 1];
    best[0][0] = 0;
    for j in 1..=k {
        for i in j..=n {
            for p in (j - 1)..i {
                if best[j - 1][p] == NONE {
                    continue;
                }
                let c = best[j - 1][p].max(cost(p, i));
                if c < best[j][i] {
                    best[j][i] = c;
                    from[j][i] = p;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(k);
    let mut end = n;
    for j in (1..=k).rev() {
        let start = from[j][end];
        out.push(start..end);
        end = start;
    }
    out.reverse();
    out
}

/// The box and its columns for `sections` inside `area`; `None` when the area is too small for
/// the key list (the overlay then says so).
pub fn geometry(area: Rect, sections: &[Section]) -> Option<Geometry> {
    let heights: Vec<usize> = sections.iter().map(|s| usize::from(!s.title.is_empty()) + s.rows.len()).collect();
    let k = column_count(area.width).min(heights.len()).max(1);
    let ranges = split(&heights, k);
    let k = ranges.len().max(1);

    // The box: one column keeps the old 78 columns; more columns take the width their longest
    // description needs, up to the screen.
    let desc_w = sections.iter().flat_map(|s| &s.rows).map(|r| util::width(&r.desc).min(DESC_CAP)).max().unwrap_or(0);
    let w = if k == 1 {
        78.min(area.width.saturating_sub(2))
    } else {
        let natural = 4 + k * (1 + KEY_W + desc_w) + (k - 1) * usize::from(GAP);
        u16::try_from(natural).unwrap_or(u16::MAX).min(area.width.saturating_sub(4))
    };
    // One column scrolls, so it keeps a compact box; several columns use the whole height.
    let h = if k == 1 { area.height.saturating_sub(2).min(40) } else { area.height.saturating_sub(2) };
    if w < 30 || h < 6 {
        return None;
    }
    let rect = {
        let w = w.min(area.width);
        let h = h.min(area.height);
        Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
    };
    // Inside the border: a one-column padding on each side; the footer takes the last row.
    let inner_w = w.saturating_sub(4);
    let rows = usize::from(h.saturating_sub(3));
    let col_w = inner_w.saturating_sub((k as u16 - 1) * GAP) / k as u16;

    // Selectable rows are numbered in reading order: the columns hold the sections in order.
    let mut sel = 0;
    let mut columns: Vec<Vec<Line>> = Vec::with_capacity(k);
    for range in &ranges {
        let mut lines = Vec::new();
        for (idx, section) in sections[range.clone()].iter().enumerate() {
            if idx > 0 {
                lines.push(Line::Gap);
            }
            if !section.title.is_empty() {
                lines.push(Line::Header(section.title.clone()));
            }
            for r in &section.rows {
                let s = r.action.map(|_| {
                    sel += 1;
                    sel - 1
                });
                lines.push(Line::Row { key: r.key.clone(), desc: r.desc.clone(), sel: s });
            }
        }
        columns.push(lines);
    }
    if columns.is_empty() {
        columns.push(Vec::new());
    }
    Some(Geometry { rect, col_w, rows, columns })
}

impl App {
    /// The reference rows that match `query`.
    pub fn help_sections(&self, query: &str) -> Vec<Section> {
        filter(sections(self), query)
    }

    /// The actions of the selectable rows, in the order they are drawn.
    pub fn help_actions(&self, query: &str) -> Vec<Action> {
        self.help_sections(query).iter().flat_map(|s| s.rows.iter().filter_map(|r| r.action)).collect()
    }

    /// The layout at the size of the last frame (`App::size`).
    pub fn help_geometry(&self, query: &str) -> Option<Geometry> {
        geometry(Rect::new(0, 0, self.size.0, self.size.1), &self.help_sections(query))
    }

    /// Scrolls the list so that the selected row is on screen.
    pub fn help_follow(&self, st: &mut HelpState) {
        let Some(g) = self.help_geometry(&st.query) else { return };
        if let Some((_, line)) = g.line_of(st.selected) {
            if line < st.scroll {
                st.scroll = line;
            } else if line >= st.scroll + g.rows {
                st.scroll = line + 1 - g.rows;
            }
        }
        st.scroll = st.scroll.min(g.max_scroll());
    }

    /// Runs the selectable row `sel` and closes the overlay; `false` when there is no such row.
    pub fn help_run(&mut self, query: &str, sel: usize) -> bool {
        let Some(action) = self.help_actions(query).get(sel).copied() else { return false };
        self.overlay = None;
        self.run(action);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_sections_in_order_and_balances_height() {
        // 5, 1, 4, 2, 3 lines (gaps between sections count): the cut after the second section leaves
        // columns of 7 and 11 lines, the tallest one is the least of the possible cuts.
        let heights = [5, 1, 4, 2, 3];
        let cols = split(&heights, 2);
        assert_eq!(cols, vec![0..2, 2..5]);
        // Heights 3, 3, 3 in three columns: one each; a single column keeps them all.
        assert_eq!(split(&[3, 3, 3], 3), vec![0..1, 1..2, 2..3]);
        assert_eq!(split(&[3, 3, 3], 1), vec![0..3]);
        assert!(split(&[], 2).is_empty());
    }

    #[test]
    fn filter_drops_sections_without_a_matching_row() {
        let row = |k: &str, d: &str| Row { key: k.into(), desc: d.into(), action: None };
        let sections = vec![
            Section { title: "A".into(), rows: vec![row("ctrl+s", "split right")] },
            Section { title: "B".into(), rows: vec![row("x", "close")] },
        ];
        let kept = filter(sections.clone(), "SPLIT");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "A");
        assert_eq!(kept[0].rows.len(), 1);
        // The key text matches too; a header never does.
        assert_eq!(filter(sections.clone(), "ctrl").len(), 1);
        assert!(filter(sections.clone(), "B").is_empty());
        assert_eq!(filter(sections, "  ").len(), 2);
    }
}
