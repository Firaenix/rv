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
use super::view::View;
use crate::config::Config;
use crate::config::Settings;
use crate::session::Review;
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
        Self::open_with_config(review, engine, &Config::default(), &Settings::default())
    }

    pub fn open_with_config(
        review: Review,
        engine: super::DiffEngine,
        config: &Config,
        settings: &Settings,
    ) -> Result<Self> {
        Self::build(
            review,
            engine,
            config,
            View::from_settings(settings),
            settings.auto_refresh.unwrap_or(true),
        )
    }

    /// The one constructor. `view` is the reviewer's whole display state,
    /// handed in rather than read from a settings file here so that a refresh
    /// can build a fresh app that *starts* with the preferences it has — there
    /// is no list of them to copy afterwards, and nothing to leave off it.
    pub(super) fn build(
        review: Review,
        engine: super::DiffEngine,
        config: &Config,
        view: View,
        auto_refresh: bool,
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
        let watch = super::watch::Watch::new(auto_refresh, review.store.root());
        let mut app = Self {
            review,
            diffs,
            blobs,
            merges,
            view,
            merger: super::merges::Merger::default(),
            comments,
            flags: Vec::new(),
            reviewed: Vec::new(),
            query: String::new(),
            column: 0,
            file_index: 0,
            cursor_rows,
            cursor_anchor: None,
            drift,
            focus: Focus::Diff,
            sidebar_tab: SidebarTab::Files,
            browser_index: 0,
            comment_index: 0,
            flag_index: 0,
            collapsed,
            collapsed_dirs: HashSet::new(),
            zoom: Vec::new(),
            nodes_cache: std::cell::RefCell::new(None),
            sidebar_row: 0,
            stats,
            keymap,
            watch,
            help: HelpStage::Closed,
            help_scroll: 0,
            pending_leader: None,
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
            commit_merges: HashMap::new(),
            symbol_index: crate::index::Index::default(),
            indexed_scope: None,
            refining: HashSet::new(),
            refined: HashSet::new(),
            refiner: diffs::Refiner::default(),
            info_scroll: 0,
            commits: commits::Commits::default(),
            pending_edit: None,
        };
        for message in unreadable {
            app.raise(message);
        }
        for warning in keymap_warnings {
            app.raise(warning);
        }
        app.reload_flags()?;
        app.load_reviewed()?;
        app.fold_all_reviewed();
        app.load_selected()?;
        Ok(app)
    }
}
