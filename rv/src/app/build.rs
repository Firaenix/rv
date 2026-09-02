//! Opening a review: assembling the initial [`App`], one field at a time.
//!
//! Split out of `app.rs` at the 400-line rule; `struct App`'s field
//! declarations stayed in `app.rs` since this module needs every one of them
//! visible, and moving both together would just relocate the whole problem.

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::store::CommentState;

use super::App;
use super::Focus;
use super::HelpStage;
use super::Mode;
use super::SidebarTab;
use super::commits;
use super::diffs;
use super::keymap::Keymap;
use super::paint;
use super::status::HELP;
use crate::config::Config;
use crate::config::Settings;
use crate::layout::Split;
use crate::session::Review;
use crate::statusbar;
use crate::tree::Sort;
use crate::ui;

impl App {
    /// Opens `review` in the reviewer, loading the first file's diff.
    ///
    /// # Why the engine is a parameter and not an environment variable
    ///
    /// `RV_NO_DIFFT` exists and `diff::compute` honours it, but a *test* cannot
    /// use it: `std::env::set_var` is unsafe in edition 2024 precisely because it
    /// races any concurrent `getenv`, and cargo runs integration tests on many
    /// threads. The alternative this replaces is therefore a data race, not an
    /// inconvenience — which is the argument for the parameter, and a stronger one
    /// than the process-wide-and-impolite reasoning that used to stand here.
    pub fn open(review: Review, engine: super::DiffEngine) -> Result<Self> {
        Self::build(review, engine, &Config::default(), &Settings::default())
    }

    pub fn open_with_config(
        review: Review,
        engine: super::DiffEngine,
        config: &Config,
        settings: &Settings,
    ) -> Result<Self> {
        Self::build(review, engine, config, settings)
    }

    pub(super) fn build(
        review: Review,
        engine: super::DiffEngine,
        config: &Config,
        settings: &Settings,
    ) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        let blobs = vec![None; review.files.len()];
        let merges = (0..review.files.len()).map(|_| None).collect::<Vec<_>>();
        // Read before the first diff is computed: a reviewer who quit halfway
        // through yesterday opens on the notes they already made.
        let mut comments = crate::session::in_range(
            &review,
            review
                .store
                .comments()
                .context("could not read the saved comments")?,
        );
        // Derived, never stored: a comment whose anchor no longer resolves is
        // outdated for as long as that stays true and no longer. One pass, its
        // findings kept — see the `drift` field.
        let drift = crate::stale::survey(&review, &mut comments);
        let cursor_rows = vec![0; review.files.len()];
        // A comment that is no longer open starts folded: still exactly where
        // the reviewer left it, without competing for the screen with the ones
        // still asking for an answer. Seeded here rather than forced every
        // frame so that `s` can expand one like any other box.
        let collapsed = comments
            .iter()
            .filter(|comment| comment.state != CommentState::Open)
            .map(|comment| comment.id.clone())
            .collect();
        // Before anything is drawn: the sidebar's tint and counts are facts
        // about the whole review. Unreadable blobs are measured as zero *and
        // said out loud*, unstamped — opening a review has no more clock in
        // reach than a key press does.
        let (stats, unreadable) = Self::measure(&review);
        let mut keymap = Keymap::from_config(config)?;
        let keymap_warnings = keymap.take_warnings();
        let mut app = Self {
            review,
            diffs,
            blobs,
            merges,
            full_context: settings.full_context.unwrap_or(true),
            merger: super::merges::Merger::default(),
            comments,
            file_index: 0,
            cursor_rows,
            drift,
            focus: Focus::Diff,
            sidebar_tab: SidebarTab::Files,
            browser_index: 0,
            comment_index: 0,
            collapsed,
            collapsed_dirs: HashSet::new(),
            tree: settings.tree.unwrap_or(false),
            sort: settings.sort.map_or_else(Sort::default, Sort::from),
            tint: settings.tint.unwrap_or(true),
            counts: settings.counts.unwrap_or(true),
            zoom: Vec::new(),
            nodes_cache: std::cell::RefCell::new(None),
            sidebar_row: 0,
            stats,
            // `RV_ASCII` set still wins — an environment override outranks a
            // settings file the way a flag outranks both.
            ascii: statusbar::ascii_from_env() || settings.ascii.unwrap_or(false),
            split: settings.split.map_or_else(Split::default, Split::new),
            keymap,
            help: HelpStage::Closed,
            help_scroll: 0,
            pending_leader: None,
            grouped: settings.grouped.unwrap_or(false),
            view_side: super::viewside::ViewSide::default(),
            body_width: Cell::new(ui::default_body_width()),
            highlights: HashMap::new(),
            painted: Cell::new(ui::default_layout()),
            dragging: false,
            diff_scroll: None,
            sidebar_scroll: None,
            diff_hscroll: 0,
            sidebar_hscroll: 0,
            alerts: Vec::new(),
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
            status_stamp: None,
            status_seen: String::new(),
            engine,
            parsing: HashSet::new(),
            painter: paint::Painter::default(),
            commit_pair: None,
            commit_diffs: HashMap::new(),
            commit_blobs: HashMap::new(),
            symbol_index: crate::index::Index::default(),
            indexed_scope: None,
            refining: HashSet::new(),
            refined: HashSet::new(),
            refiner: diffs::Refiner::default(),
            info_dismissed: false,
            info_scroll: 0,
            sidebar_hidden: settings.sidebar_hidden.unwrap_or(false),
            commits: commits::Commits::default(),
            pending_edit: None,
        };
        for message in unreadable {
            app.raise(message);
        }
        for warning in keymap_warnings {
            app.raise(warning);
        }
        app.load_selected()?;
        Ok(app)
    }
}
