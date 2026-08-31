//! `PgUp`/`PgDn`/`Home`/`End`: the cursor by a screenful, or to an end.
//!
//! Cursor moves, not view scrolls: rv steers a selection and the view follows
//! it, so a page is many `j`s and `Home`/`End` is `k`/`j` run to the wall. The
//! page's size is the focused pane's height, read off the last frame's
//! [`crate::layout::Layout`] — the one thing here that needs the terminal's
//! shape, and the reason these live apart from [`super::navigate`].

use anyhow::Result;

use super::App;
use super::Focus;
use super::SidebarTab;

/// The page for a pane whose height was never painted — the first key before
/// the first frame, and the unit tests that never draw.
const UNPAINTED_PAGE: usize = 10;

impl App {
    /// `PgDn`: the cursor forward one screenful in the focused pane.
    pub(super) fn page_forward(&mut self) -> Result<()> {
        self.page(true)
    }

    /// `PgUp`: the cursor back one screenful.
    pub(super) fn page_backward(&mut self) -> Result<()> {
        self.page(false)
    }

    fn page(&mut self, forward: bool) -> Result<()> {
        let step = self.focused_pane_rows();
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.step_sidebar(forward, step)?,
                SidebarTab::Comments => self.step_browser(forward, step),
            },
            Focus::Diff => {
                let row = if forward {
                    self.cursor_row().saturating_add(step)
                } else {
                    self.cursor_row().saturating_sub(step)
                };
                self.set_cursor_row(row);
            }
            Focus::Stack => {
                let last = self.stack_len().saturating_sub(1);
                self.comment_index = if forward {
                    self.comment_index.saturating_add(step).min(last)
                } else {
                    self.comment_index.saturating_sub(step)
                };
            }
        }
        Ok(())
    }

    /// `End`: the cursor to the last row of the focused pane.
    pub(super) fn jump_last(&mut self) -> Result<()> {
        self.jump(true)
    }

    /// `Home`: the cursor to the first row.
    pub(super) fn jump_first(&mut self) -> Result<()> {
        self.jump(false)
    }

    fn jump(&mut self, forward: bool) -> Result<()> {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.jump_sidebar(forward)?,
                SidebarTab::Comments => self.jump_browser(forward),
            },
            Focus::Diff => {
                let row = if forward {
                    self.row_count().saturating_sub(1)
                } else {
                    0
                };
                self.set_cursor_row(row);
            }
            Focus::Stack => {
                self.comment_index = if forward {
                    self.stack_len().saturating_sub(1)
                } else {
                    0
                };
            }
        }
        Ok(())
    }

    /// How many rows a page moves: the focused pane's content height, less its
    /// borders, less one row of overlap so a page keeps a line of context.
    fn focused_pane_rows(&self) -> usize {
        let painted = self.painted.get();
        let pane = match self.focus {
            Focus::Sidebar => painted.sidebar,
            Focus::Diff | Focus::Stack => painted.diff,
        };
        let rows = usize::from(pane.height.saturating_sub(crate::ui::BORDER_ROWS));
        match rows {
            0 => UNPAINTED_PAGE,
            _ => rows.saturating_sub(1).max(1),
        }
    }
}
