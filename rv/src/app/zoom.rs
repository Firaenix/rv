//! Zooming the sidebar into one directory or one change, and back out.
//!
//! Deep trees in a narrow pane truncate every name to punctuation. The zoom is
//! the answer: `Enter` on a row that holds things makes that row the view — its
//! contents at shallow indent, under one [`NodeKind::Up`] row that names where
//! the reviewer is and leads back out.
//!
//! The zoom is a **view**, not a different tree: [`App::zoom_view`] carves the
//! full node list down after it is built, so folding, sorting, selection and
//! conservation all keep meaning exactly what they mean unzoomed. A key that no
//! longer names a row — the tree was flattened with `t`, the tab changed, a
//! refresh dropped the directory — renders the whole list rather than an error:
//! the zoom is dormant, not wrong.

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::tree::KEY_SEPARATOR;
use crate::tree::Node;
use crate::tree::NodeKind;

/// One level of zoom: the key of the row the view is carved to, and what the
/// [`NodeKind::Up`] row calls it.
///
/// The name is captured when the zoom happens rather than derived from the key
/// per frame, because a commits-view key is `change:path` and splitting that
/// back apart would be guessing at a separator a path is merely unlikely to
/// contain.
pub(super) struct Zoom {
    pub key: String,
    pub name: String,
}

impl App {
    /// `nodes`, carved down to the current zoom where it still names a row.
    ///
    /// The zoomed row's children become the view, one level shallower, under an
    /// [`NodeKind::Up`] row carrying the zoom's name and the subtree's whole
    /// cost.
    pub(super) fn zoom_view(&self, nodes: Vec<Node>) -> Vec<Node> {
        let Some(zoom) = self.zoom.last() else {
            return nodes;
        };
        let Some(root) = nodes.iter().position(|node| zoomable_key(node) == Some(&zoom.key))
        else {
            return nodes;
        };
        let depth = nodes[root].depth;
        let end = nodes[root + 1..]
            .iter()
            .position(|node| node.depth <= depth)
            .map_or(nodes.len(), |below| root + 1 + below);

        let mut view = Vec::with_capacity(end - root);
        view.push(Node {
            label: zoom.name.clone(),
            depth: 0,
            kind: NodeKind::Up,
            stat: nodes[root].stat,
        });
        view.extend(nodes[root + 1..end].iter().map(|node| Node {
            depth: node.depth - depth,
            ..node.clone()
        }));
        view
    }

    /// `Enter` in the file list: into the row under the cursor where it holds
    /// things, back out where it is the [`NodeKind::Up`] row.
    ///
    /// Whether anything happened, so [`App::on_enter`] knows to fall through to
    /// the stack for the rows this does not answer.
    pub(super) fn zoom_under_cursor(&mut self) -> bool {
        if self.focus != Focus::Sidebar || self.sidebar_tab == SidebarTab::Comments {
            return false;
        }
        if let Some(NodeKind::Up) = self.nodes().get(self.sidebar_row).map(|node| &node.kind) {
            self.zoom_out();
            return true;
        }
        self.zoom_into_under_cursor()
    }

    /// `Shift+→` in the file list: into the row under the cursor where it
    /// holds things, and nothing anywhere else — the inward half of the
    /// shifted arrows' walk, whose outward half is [`App::zoom_out`].
    pub(super) fn zoom_into_under_cursor(&mut self) -> bool {
        if self.focus != Focus::Sidebar || self.sidebar_tab == SidebarTab::Comments {
            return false;
        }
        let Some(node) = self.nodes().get(self.sidebar_row).cloned() else {
            return false;
        };
        match &node.kind {
            NodeKind::File { .. } | NodeKind::Up => false,
            NodeKind::Dir { key, .. } => {
                let name = match self.sidebar_tab {
                    // The change's half of the key is address, not name: the
                    // Up row should read as a place, and the change it is
                    // under is one zoom-out away.
                    SidebarTab::Commits => key
                        .split_once(KEY_SEPARATOR)
                        .map_or(key.as_str(), |(_, path)| path)
                        .to_owned(),
                    _ => key.clone(),
                };
                self.zoom_in(key.clone(), name);
                true
            }
            NodeKind::Commit {
                change_id,
                short_change,
                subject,
                ..
            } => {
                self.zoom_in(change_id.clone(), format!("{short_change} {subject}"));
                true
            }
        }
    }

    /// Makes the row folding under `key` the view. Unfolded first: a zoom into
    /// a collapsed row would show its Up row over nothing.
    fn zoom_in(&mut self, key: String, name: String) {
        self.collapsed_dirs.remove(&key);
        self.zoom.push(Zoom { key, name });
        self.sidebar_row = 0;
        self.sidebar_scroll = None;
    }

    /// One level back out, leaving the cursor on the row that was zoomed into —
    /// the reviewer is *at* that directory, looking at it from outside now.
    pub(super) fn zoom_out(&mut self) {
        let Some(left) = self.zoom.pop() else {
            return;
        };
        self.sidebar_row = self
            .nodes()
            .iter()
            .position(|node| zoomable_key(node) == Some(&left.key))
            .unwrap_or(0);
        self.sidebar_scroll = None;
    }

    /// Whether the sidebar is zoomed in, which is what makes `Esc` mean "back
    /// out" there.
    pub(super) fn zoomed(&self) -> bool {
        !self.zoom.is_empty()
    }
}

/// The key a zoom would carve to, for the rows that hold things.
fn zoomable_key(node: &Node) -> Option<&String> {
    match &node.kind {
        NodeKind::Dir { key, .. } => Some(key),
        NodeKind::Commit { change_id, .. } => Some(change_id),
        NodeKind::File { .. } | NodeKind::Up => None,
    }
}
