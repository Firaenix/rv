//! The comment browser's rows: what the Comments tab lists, and which of
//! those rows its cursor is allowed to be on.

use rv_core::model::Anchor;
use rv_core::store::Flag;

use crate::app::App;
use crate::app::SidebarTab;
use crate::tree;
use crate::tree::NodeKind;

/// One row of the comment browser.
///
/// The browser draws file headings between the comments, so a row and a
/// comment stopped being the same number. **The cursor is a row and the
/// comment is derived from it** — the sibling of the ruling `cursor_rows`
/// records for the diff pane, and for the same reason: two cursors kept in
/// step is the defect, not the fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserRow {
    /// A directory of the tree the commented files hang from — present only
    /// while the sidebar is in tree mode, and never collapsible: comments are
    /// few enough at review scale that hiding them costs more keystrokes than
    /// it saves (inline-comments spec §3).
    Dir { label: String, depth: usize },
    /// The file every comment row under it is anchored in. `label` is how the
    /// row reads — the whole path in the flat list, the file's own name in the
    /// tree — and `path` stays the whole path, because it is what `Enter`
    /// opens.
    File {
        path: String,
        label: String,
        depth: usize,
    },
    /// A comment, addressing [`App::comments`] — its *store* position, which
    /// is not this row's number and never becomes it.
    Comment { index: usize, depth: usize },
    /// A flag, addressing [`App::flags`] the same way.
    Flag { index: usize, depth: usize },
}

impl BrowserRow {
    /// Whether the row is a note the cursor can rest on, rather than a
    /// heading it steps over.
    #[must_use]
    pub fn is_note(&self) -> bool {
        matches!(self, BrowserRow::Comment { .. } | BrowserRow::Flag { .. })
    }
}

impl App {
    /// The comment browser's rows: every comment in the review under a heading
    /// naming its file, ordered by `(file, line)`.
    ///
    /// Grouped rather than flat because a reviewer returning to a review moves
    /// through what they *said*, and what they said is organised by where they
    /// said it. The heading is a row and not a collapsible node — inline
    /// comments spec §3 — so nothing here can hide a comment behind an
    /// expansion.
    ///
    /// Derived per call like every other list in [`crate::app::query`]: it is a pure
    /// function of `comments`, and a cached copy would be one more thing to
    /// keep in step.
    #[must_use]
    pub fn browser_rows(&self) -> Vec<BrowserRow> {
        // The Flags tab lists the flags the same way; the anchors are what
        // the grouping is built from, and both kinds have one.
        let anchors: Vec<&Anchor> = if self.sidebar_tab == SidebarTab::Flags {
            self.flags.iter().map(|flag| &flag.anchor).collect()
        } else {
            self.comments
                .iter()
                .map(|comment| &comment.anchor)
                .collect()
        };
        let note = |index: usize, depth: usize| {
            if self.sidebar_tab == SidebarTab::Flags {
                BrowserRow::Flag { index, depth }
            } else {
                BrowserRow::Comment { index, depth }
            }
        };
        let mut order: Vec<usize> = (0..anchors.len()).collect();
        // By file and line, with the store position last so that two comments
        // on one line keep the order they were written in — the browser has
        // always opened on the oldest.
        order.sort_by(|a, b| {
            let (left, right) = (anchors[*a], anchors[*b]);
            left.file
                .cmp(&right.file)
                .then(left.line.cmp(&right.line))
                .then(a.cmp(b))
        });
        // One entry per commented file, in the sorted order — the sort makes
        // a file's comments adjacent, so `paths` and `per_file` stay parallel.
        let mut paths: Vec<&str> = Vec::new();
        let mut per_file: Vec<Vec<usize>> = Vec::new();
        for index in order {
            let file = anchors[index].file.as_str();
            if paths.last() != Some(&file) {
                paths.push(file);
                per_file.push(Vec::new());
            }
            per_file
                .last_mut()
                .expect("a comment always has a file entry")
                .push(index);
        }

        // The same builder the files pane uses, over the commented files, so
        // the two tabs answer the tree and sort toggles with the same shape.
        // Folding stays off (empty collapsed set): a heading that hid its
        // comments would cost more keystrokes than it saves.
        let stat_of = |list_index: usize| {
            self.review
                .files
                .iter()
                .position(|file| file.path == paths[list_index])
                .and_then(|file_index| self.stats.get(file_index).copied())
                .unwrap_or_default()
        };
        let nodes = tree::build(
            &paths,
            &std::collections::HashSet::new(),
            self.view.tree,
            self.view.sort,
            &stat_of,
        );

        let mut rows = Vec::new();
        for node in nodes {
            match node.kind {
                NodeKind::Dir { .. } => rows.push(BrowserRow::Dir {
                    label: node.label,
                    depth: node.depth,
                }),
                NodeKind::File { index } => {
                    rows.push(BrowserRow::File {
                        path: paths[index].to_owned(),
                        label: node.label,
                        depth: node.depth,
                    });
                    rows.extend(
                        per_file[index]
                            .iter()
                            .map(|entry| note(*entry, node.depth + 1)),
                    );
                }
                NodeKind::Commit { .. } | NodeKind::Up => {}
            }
        }
        rows
    }

    /// Which comment the browser's cursor is on, as a position in
    /// [`App::comments`] — `None` on a heading row and on an empty review.
    pub(in crate::app) fn browsed_index(&self) -> Option<usize> {
        match self.browser_rows().get(self.browser_index)? {
            BrowserRow::Comment { index, .. } => Some(*index),
            BrowserRow::File { .. } | BrowserRow::Dir { .. } | BrowserRow::Flag { .. } => None,
        }
    }

    /// Which flag the browser's cursor is on, as a position in [`App::flags`].
    pub fn browsed_flag(&self) -> Option<&Flag> {
        match self.browser_rows().get(self.browser_index)? {
            BrowserRow::Flag { index, .. } => self.flags.get(*index),
            _ => None,
        }
    }

    /// The file the browser's cursor is in: the heading's own path, or the
    /// browsed comment's file as the review lists it.
    pub(in crate::app) fn browsed_file_path(&self) -> Option<String> {
        match self.browser_rows().get(self.browser_index)? {
            BrowserRow::File { path, .. } => Some(path.clone()),
            BrowserRow::Comment { .. } | BrowserRow::Flag { .. } => {
                let anchored = match self.browser_rows().get(self.browser_index)? {
                    BrowserRow::Comment { index, .. } => &self.comments.get(*index)?.anchor.file,
                    BrowserRow::Flag { index, .. } => &self.flags.get(*index)?.anchor.file,
                    _ => return None,
                }
                .clone();
                let anchored = &anchored;
                self.review
                    .files
                    .iter()
                    .find(|file| {
                        file.path == *anchored || file.source_path.as_deref() == Some(anchored)
                    })
                    .map(|file| file.path.clone())
            }
            BrowserRow::Dir { .. } => None,
        }
    }

    /// Keeps the browser's cursor on the list after the list has changed under
    /// it, and off a heading.
    ///
    /// A heading is not selectable, so a clamp that landed on one would leave
    /// `d` and `s` with no target on a row the reviewer can see. The step is
    /// **forwards**, onto the first comment of the file the heading names,
    /// because that is the row the reviewer was reaching for; only a cursor
    /// clamped onto a trailing heading — which cannot happen, every heading has
    /// a comment under it — would have nowhere forward to go. An empty list
    /// parks it at 0, which is where the next comment lands.
    pub(in crate::app) fn clamp_browser(&mut self) {
        let rows = self.browser_rows();
        self.browser_index = self.browser_index.min(rows.len().saturating_sub(1));
        // Forwards over every heading — in tree mode a file sits under a chain
        // of directory rows, so one step is not enough. Every file heading has
        // a comment under it, so the walk always lands.
        while matches!(
            rows.get(self.browser_index),
            Some(BrowserRow::File { .. } | BrowserRow::Dir { .. })
        ) {
            self.browser_index = self.browser_index.saturating_add(1);
        }
        self.browser_index = self.browser_index.min(rows.len().saturating_sub(1));
    }
}
