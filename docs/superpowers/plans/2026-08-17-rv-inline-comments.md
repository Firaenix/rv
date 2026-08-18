# rv Inline Comments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show every saved comment inline in the diff as a blue bordered box, make the two panes navigable with Left/Right, let the reviewer step into a line's comment stack to select and delete comments, and make boxes collapsible.

**Architecture:** All rendering stays inside `rv/src/ui.rs` as pure `state → Text` functions so `TestBackend` can assert on them; all state and key handling stays in `rv/src/app.rs`. The only `rv-core` change is one new store method. A new pure row model turns "diff lines plus their comments" into a flat list of terminal rows, which is what makes variable-height comment boxes windowable and unit-testable apart from any terminal.

**Tech Stack:** Rust 2024, ratatui 0.30, crossterm 0.29, rv-core (existing), rstest + proptest (dev).

**Spec:** `docs/superpowers/specs/2026-08-17-rv-inline-comments-design.md`

## Global Constraints

- **`rv-core` MUST NOT depend on `ratatui`, `crossterm`, or `tui-textarea`.** `rv-core/tests/constraints.rs` enforces this mechanically; a violation fails the build.
- **`jj_lib` is imported only in `rv-core/src/vcs.rs`.** Enforced by the same test file.
- **Never read the user's jj config** — no `config_path`, no `ConfigSource::User`, no `ConfigSource::Repo`. Enforced mechanically.
- **`App::on_key` must stay terminal-free.** It is the state machine's only entry point and the reason the app is testable without a pty. Never call into ratatui from it.
- **The terminal must be restored on every exit path**, including panic. The existing panic hook in `rv/src/app.rs` does this; do not disturb it.
- **`rv` writes only under `.review/`** plus the one line in `.git/info/exclude`.
- **Display and storage must agree on which line a comment belongs to.** Both go through `rv::app::anchored_side(kind)`: `Side::Left` for `LineKind::Removed`, `Side::Right` otherwise. Milestone 1 shipped a bug where the pane and the anchor disagreed; never reintroduce a second side rule.
- **A comment's anchor path is `source_path` for a `Left` anchor and `path` for a `Right` anchor.**
- **`Store` is stateless** — every method hits disk. Do not add caching to it.
- **Every `.review/` write goes through the store's `write_atomic`.** Never `fs::write` a file under `.review/` from `rv`.
- Commits use jj, not git: `jj describe -m "…" && jj new`, with the two trailer lines the repo already uses on every commit.
- Blue is reserved for comments. Focus is indicated with `Modifier::BOLD` borders and a `▸` title prefix, never with colour.
- **Dependency:** a separate fix corrects `Store::append_comment` to upsert by `id` rather than `change_id`. This plan assumes the corrected behaviour: distinct ids coexist, and a repeated id updates in place keeping its position. If `append_comment` still matches on `change_id` when you start, stop and report it.
- **Storage model** (see `docs/superpowers/specs/2026-08-17-rv-storage-model-design.md`, which is the authority): the end state is that **`.review/session.toml` is the only file rv maintains**, holding the comments alongside the session scope, with `comments.json` and `snapshots/` retired and `REVIEW-FEEDBACK.md` an **export** produced by `rv render`. That migration has its own plan and is not this one's work. What binds this plan is the part that must not regress: **saving or deleting a comment must not rewrite the markdown.** Assert persistence against whichever file the store currently owns — `comments.json` today, `session.toml` after the migration — and use one end-to-end `rv render` test where the exported document itself matters. Write persistence assertions through `Store`'s API rather than by reading a filename directly, so they survive the migration.

### Existing interfaces you build on

```rust
// rv/src/app.rs
pub enum Mode { Browse, Comment }
pub enum Action { Continue, Quit }
pub struct App { review: Review, diffs: Vec<Option<FileDiff>>, file_index: usize,
                 line_index: usize, mode: Mode, buffer: String, status: String }
impl App {
    pub fn new(review: Review) -> Result<Self>;
    pub fn run(review: Review) -> Result<()>;
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action>;
    pub fn selected_file(&self) -> Option<&FileChange>;
    pub fn selected_diff(&self) -> Option<&FileDiff>;
    pub fn files(&self) -> &[FileChange];
    pub fn file_index(&self) -> usize;
    pub fn line_index(&self) -> usize;
    pub fn mode(&self) -> Mode;
    pub fn buffer(&self) -> &str;
    pub fn status(&self) -> &str;
}
pub fn anchored_side(kind: LineKind) -> Side;

// rv-core
pub struct Comment { pub id: String, pub change_id: String, pub commit_id: String,
                     pub anchor: Anchor, pub body: String, pub state: CommentState,
                     pub reply: Option<String> }
pub struct Anchor { pub file: String, pub side: Side, pub line: u32,
                    pub content_hash: String, pub context: Vec<String> }
pub enum CommentState { Open, AwaitingVerification, Resolved, Outdated }
impl Store {
    pub fn comments(&self) -> Result<Vec<Comment>, Error>;
    pub fn append_comment(&self, comment: &Comment) -> Result<(), Error>;
    pub fn write_markdown(&self, document: &str) -> Result<(), Error>;
    pub fn markdown_path(&self) -> PathBuf;
}
// rv/src/session.rs
pub fn write_markdown(review: &Review) -> anyhow::Result<()>;  // folds replies, then writes
```

---

## Task 1: `Store::remove_comment`

**Files:**
- Modify: `rv-core/src/store.rs`
- Test: `rv-core/tests/store.rs`

**Interfaces:**
- Consumes: `Store::comments`, the private `write_atomic`, `snapshots_dir`, `comments_path`.
- Produces: `pub fn remove_comment(&self, id: &str) -> Result<bool, Error>` — `true` when a comment was removed, `false` when no comment had that id.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn removing_a_comment_drops_it_and_its_snapshot() {
    let fixture = Fixture::new();
    let store = Store::open(fixture.root()).expect("open");
    let first = sample_comment("aaaaaaaa", "first finding", 1);
    let second = sample_comment("bbbbbbbb", "second finding", 2);
    store.append_comment(&first).expect("append first");
    store.append_comment(&second).expect("append second");

    let removed = store.remove_comment("aaaaaaaa").expect("remove");

    assert!(removed, "remove_comment reports it removed something");
    let left = Store::open(fixture.root())
        .expect("reopen")
        .comments()
        .expect("read");
    assert_eq!(
        left.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["bbbbbbbb"],
        "only the other comment survives, and a fresh Store sees it"
    );
    assert!(
        !fixture.root().join(".review/snapshots/aaaaaaaa").exists(),
        "the removed comment's snapshot is gone"
    );
    assert!(
        fixture.root().join(".review/snapshots/bbbbbbbb").exists(),
        "the surviving comment keeps its snapshot"
    );
}

#[test]
fn removing_an_unknown_id_is_not_an_error() {
    let fixture = Fixture::new();
    let store = Store::open(fixture.root()).expect("open");
    store
        .append_comment(&sample_comment("aaaaaaaa", "finding", 1))
        .expect("append");

    let removed = store.remove_comment("nosuchid").expect("remove");

    assert!(!removed, "nothing was removed");
    assert_eq!(store.comments().expect("read").len(), 1, "nothing was lost");
}
```

Match `sample_comment`'s real signature in this file; if it does not take `(id, body, line)`, adapt these calls to the helper that exists rather than adding a second helper.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rv-core --test store`
Expected: FAIL — `no method named remove_comment found for struct Store`.

- [ ] **Step 3: Implement `remove_comment`**

```rust
    /// Removes the comment with `id`, returning whether one was there.
    ///
    /// `comments.json` is written before the snapshot file is deleted, mirroring
    /// [`Store::append_comment`]'s ordering for the same reason: a crash between
    /// the two leaves an orphaned snapshot, which is inert, rather than a comment
    /// whose snapshot is missing.
    ///
    /// An unknown id is not an error. Deletion is idempotent, so a retry after a
    /// crash cannot fail on the second attempt.
    pub fn remove_comment(&self, id: &str) -> Result<bool, Error> {
        let mut comments = self.comments()?;
        let before = comments.len();
        comments.retain(|existing| existing.id != id);
        if comments.len() == before {
            return Ok(false);
        }

        let serialized =
            serde_json::to_string_pretty(&comments).map_err(Error::SerializeComments)?;
        write_atomic(&self.comments_path(), serialized.as_bytes())?;

        let snapshot_path = self.snapshots_dir().join(id);
        match fs::remove_file(&snapshot_path) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(true),
            Err(source) => Err(Error::Io { path: snapshot_path, source }),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rv-core --test store`
Expected: PASS, and every pre-existing store test still passes.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv-core): remove a comment and its snapshot" && jj new
```

---

## Task 2: `App` loads comments and indexes them by anchor

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Consumes: `Store::comments`, `anchored_side`, `FileChange { path, source_path, kind }`, `DiffLine { kind, left, right, text }`.
- Produces:
  - `pub fn comments_for_line(&self, index: usize) -> &[Comment]` — the comments anchored to diff line `index` of the selected file, in `comments.json` order (oldest first).
  - `pub fn comments(&self) -> &[Comment]` — every loaded comment.
  - private `fn reload_comments(&mut self) -> Result<()>` — re-reads the store; called after every save and delete.
  - private `fn anchor_key(&self, line: &DiffLine) -> Option<(String, Side, u32)>` — the key a diff line would anchor under, `None` when that line has no number on its anchored side.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_saved_comment_is_visible_on_the_line_it_anchored_to() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);

    app.on_key(KeyCode::Char('j')).expect("move down");
    let line = app.line_index();
    app.on_key(KeyCode::Char('c')).expect("begin comment");
    for character in "needs a doc".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("save");

    let on_line = app.comments_for_line(line);
    assert_eq!(on_line.len(), 1, "the comment shows up on its own line");
    assert_eq!(on_line[0].body, "needs a doc");
    assert!(
        app.comments_for_line(line + 1).is_empty(),
        "and not on the next line"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p rv --test app`
Expected: FAIL — `no method named comments_for_line`.

- [ ] **Step 3: Implement loading and indexing**

Add `comments: Vec<Comment>` to `App`. In `App::new`, after building the review, populate it with `review.store.comments()?`. Then:

```rust
    /// Every comment in the review, in `comments.json` order.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The comments anchored to diff line `index` of the selected file, oldest
    /// first — so the newest is last, which is what `d` targets from the diff.
    ///
    /// A line is matched by the key it would anchor *under*, not by its raw
    /// number: the side rule and the base-side path for a rename both come from
    /// the same place the save path uses, so a comment can never be stored
    /// against one line and displayed against another.
    pub fn comments_for_line(&self, index: usize) -> Vec<&Comment> {
        let Some(line) = self.selected_diff().and_then(|diff| diff.lines.get(index)) else {
            return Vec::new();
        };
        let Some((path, side, number)) = self.anchor_key(line) else {
            return Vec::new();
        };
        self.comments
            .iter()
            .filter(|comment| {
                comment.anchor.file == path
                    && comment.anchor.side == side
                    && comment.anchor.line == number
            })
            .collect()
    }

    fn anchor_key(&self, line: &DiffLine) -> Option<(String, Side, u32)> {
        let file = self.selected_file()?;
        let side = anchored_side(line.kind);
        let number = match side {
            Side::Left => line.left,
            Side::Right => line.right,
        }?;
        let path = match side {
            Side::Left => file.source_path.as_deref().unwrap_or(&file.path),
            Side::Right => &file.path,
        };
        Some((path.to_owned(), side, number))
    }

    fn reload_comments(&mut self) -> Result<()> {
        self.comments = self
            .review
            .store
            .comments()
            .context("could not re-read the saved comments")?;
        Ok(())
    }
```

Call `self.reload_comments()?` at the end of `commit_comment`, right after `append_comment` succeeds, so the view reflects what is on disk. Leave the existing `session::write_markdown` call alone for now — the storage plan removes it; this task must not depend on whether it is still there.

Return type note: `comments_for_line` returns `Vec<&Comment>` rather than `&[Comment]` because the comments are filtered, not contiguous. Update the Interfaces block above in your head accordingly — the test only needs `.len()` and `.body`.

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p rv --test app`
Expected: PASS, and the existing app tests still pass.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): load saved comments and index them by anchor" && jj new
```

---

## Task 3: Pane focus with Left and Right

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Produces: `pub enum Focus { Sidebar, Diff, Stack }` (the `Stack` variant is added here but only reached in Task 6), `pub fn focus(&self) -> Focus`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn left_and_right_move_focus_between_the_panes() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    assert_eq!(app.focus(), Focus::Diff, "the diff has focus on launch");

    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Sidebar);
    app.on_key(KeyCode::Left).expect("left again");
    assert_eq!(app.focus(), Focus::Sidebar, "there is nothing left of the files");

    app.on_key(KeyCode::Right).expect("right");
    assert_eq!(app.focus(), Focus::Diff);
}

#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn with_the_files_focused_both_key_pairs_move_the_file_selection(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Left).expect("focus files");

    app.on_key(forward).expect("forward");
    assert_eq!(app.file_index(), 1, "moved to the second file");
    app.on_key(back).expect("back");
    assert_eq!(app.file_index(), 0, "and back to the first");
}

#[test]
fn with_the_diff_focused_j_and_k_still_move_the_line() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Char('j')).expect("down");
    assert_eq!(app.line_index(), 1);
    app.on_key(KeyCode::Up).expect("up");
    assert_eq!(app.line_index(), 0);
}

#[test]
fn file_navigation_keys_work_from_either_pane() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.file_index(), 1);
    app.on_key(KeyCode::Left).expect("focus files");
    app.on_key(KeyCode::Char('[')).expect("previous file");
    assert_eq!(app.file_index(), 0);
}
```

The fixture must have at least two files. If `Fixture`/`app_from` in `rv/tests/app.rs` builds only one, extend the fixture to write and commit a second file rather than weakening the assertions.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — `Focus` not found.

- [ ] **Step 3: Implement focus**

```rust
/// Which pane the keys act on. The comment stack is a focus target rather than
/// a mode because typing is what modes are for; this is only about where the
/// cursor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The left column, which lists either files or comments — see
    /// `SidebarTab` in Task 13.
    Sidebar,
    Diff,
    /// Inside the comment stack of the selected diff line (Task 6).
    Stack,
}
```

Add `focus: Focus` to `App`, initialised to `Focus::Diff`, with a `pub fn focus(&self) -> Focus` accessor. Restructure `on_key_browse` so movement dispatches on focus:

```rust
    fn on_key_browse(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Char('q') => return Ok(Action::Quit),
            KeyCode::Left => self.focus_left(),
            KeyCode::Right => self.focus_right(),
            KeyCode::Char('j') | KeyCode::Down => self.move_forward()?,
            KeyCode::Char('k') | KeyCode::Up => self.move_back()?,
            KeyCode::Char(']') => self.select_file(self.file_index.saturating_add(1))?,
            KeyCode::Char('[') => self.select_file(self.file_index.saturating_sub(1))?,
            KeyCode::Char('c') => self.begin_comment(),
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Stack => Focus::Diff,
            Focus::Diff | Focus::Sidebar => Focus::Sidebar,
        };
    }

    /// `Right` from the comment stack does nothing: the stack is drawn inside
    /// the diff pane, so there is no pane to its right. `Left` and `Esc` are
    /// the two ways out, which is why no focus is ever a trap.
    fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Diff,
            Focus::Diff | Focus::Stack => self.focus,
        };
    }

    fn move_forward(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => self.select_file(self.file_index.saturating_add(1))?,
            Focus::Diff => {
                let last = self.line_count().saturating_sub(1);
                self.line_index = self.line_index.saturating_add(1).min(last);
            }
            Focus::Stack => {} // Task 6
        }
        Ok(())
    }

    fn move_back(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => self.select_file(self.file_index.saturating_sub(1))?,
            Focus::Diff => self.line_index = self.line_index.saturating_sub(1),
            Focus::Stack => {} // Task 6
        }
        Ok(())
    }
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS. Note `select_file(0)` when already at 0 is a no-op by its existing guard, so `k` at the top of the file list stays put.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): arrow-key focus between the file list and the diff" && jj new
```

---

## Task 4: The row model

**Files:**
- Create: `rv/src/rows.rs`
- Modify: `rv/src/lib.rs` (add `pub mod rows;`)
- Test: `rv/tests/rows.rs`

**Interfaces:**
- Consumes: `FileDiff`, `DiffLine`, `Comment`.
- Produces:

```rust
pub enum Row<'a> {
    Diff { index: usize, line: &'a DiffLine },
    BoxTop { line: usize, comment: &'a Comment },
    BoxBody { line: usize, comment: &'a Comment, text: String },
    BoxBottom { line: usize, comment: &'a Comment },
    BoxCollapsed { line: usize, comment: &'a Comment },
}
pub struct Plan<'a> { pub rows: Vec<Row<'a>> }
pub fn plan<'a>(
    diff: &'a FileDiff,
    comments_for: &dyn Fn(usize) -> Vec<&'a Comment>,
    collapsed: &HashSet<String>,
    width: usize,
) -> Plan<'a>;
impl Plan<'_> {
    pub fn row_of_line(&self, line: usize) -> Option<usize>;
    pub fn row_of_comment(&self, line: usize, comment_index: usize) -> Option<usize>;
}
pub fn window(rows: usize, anchor: usize, height: usize) -> Range<usize>;
```

`width` is the pane's inner width, used to wrap comment bodies.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_line_with_no_comments_is_one_row() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);
    let plan = plan(&diff, &|_| Vec::new(), &HashSet::new(), 40);
    assert_eq!(plan.rows.len(), 3);
    assert!(matches!(plan.rows[0], Row::Diff { index: 0, .. }));
}

#[test]
fn an_expanded_comment_adds_a_bordered_box_under_its_line() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);
    let comment = comment_with_body("needs a doc");
    let plan = plan(&diff, &|line| if line == 1 { vec![&comment] } else { Vec::new() },
                    &HashSet::new(), 40);

    // three diff rows, plus top border, one wrapped body row, bottom border
    assert_eq!(plan.rows.len(), 6);
    assert!(matches!(plan.rows[1], Row::Diff { index: 1, .. }));
    assert!(matches!(plan.rows[2], Row::BoxTop { line: 1, .. }));
    assert!(matches!(plan.rows[3], Row::BoxBody { line: 1, .. }));
    assert!(matches!(plan.rows[4], Row::BoxBottom { line: 1, .. }));
    assert!(matches!(plan.rows[5], Row::Diff { index: 2, .. }));
}

#[test]
fn a_collapsed_comment_is_one_row() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);
    let comment = comment_with_body("needs a doc");
    let collapsed = HashSet::from([comment.id.clone()]);
    let plan = plan(&diff, &|line| if line == 1 { vec![&comment] } else { Vec::new() },
                    &collapsed, 40);
    assert_eq!(plan.rows.len(), 4);
    assert!(matches!(plan.rows[2], Row::BoxCollapsed { line: 1, .. }));
}

#[test]
fn a_long_body_wraps_instead_of_truncating() {
    let diff = diff_of(&["fn a() {"]);
    let comment = comment_with_body("the quick brown fox jumps over the lazy dog again");
    let plan = plan(&diff, &|_| vec![&comment], &HashSet::new(), 20);
    let body: Vec<&str> = plan.rows.iter().filter_map(|row| match row {
        Row::BoxBody { text, .. } => Some(text.as_str()),
        _ => None,
    }).collect();
    assert!(body.len() > 1, "wrapped across rows");
    assert!(body.iter().all(|row| row.chars().count() <= 20), "no row exceeds the width");
    assert_eq!(body.join(" ").split_whitespace().collect::<Vec<_>>(),
               "the quick brown fox jumps over the lazy dog again".split_whitespace().collect::<Vec<_>>(),
               "every word survives");
}

#[test]
fn a_reply_renders_inside_the_same_box() {
    let diff = diff_of(&["fn a() {"]);
    let mut comment = comment_with_body("needs a doc");
    comment.reply = Some("added one".to_owned());
    let plan = plan(&diff, &|_| vec![&comment], &HashSet::new(), 40);
    let body: Vec<&str> = plan.rows.iter().filter_map(|row| match row {
        Row::BoxBody { text, .. } => Some(text.as_str()),
        _ => None,
    }).collect();
    assert!(body.iter().any(|row| row.contains("reply:")), "the reply is in the box");
}

#[test]
fn several_comments_stack_in_order() {
    let diff = diff_of(&["fn a() {"]);
    let first = comment_with_id_and_body("aaaaaaaa", "first");
    let second = comment_with_id_and_body("bbbbbbbb", "second");
    let plan = plan(&diff, &|_| vec![&first, &second], &HashSet::new(), 40);
    let tops: Vec<&str> = plan.rows.iter().filter_map(|row| match row {
        Row::BoxTop { comment, .. } => Some(comment.id.as_str()),
        _ => None,
    }).collect();
    assert_eq!(tops, ["aaaaaaaa", "bbbbbbbb"], "oldest first, newest last");
}

#[test]
fn the_window_keeps_the_anchor_row_visible() {
    assert_eq!(window(100, 0, 10), 0..10, "at the top");
    assert_eq!(window(100, 99, 10), 90..100, "at the bottom");
    let visible = window(100, 50, 10);
    assert!(visible.contains(&50), "the anchor row is inside the window");
    assert_eq!(visible.len(), 10);
}

#[test]
fn the_window_survives_degenerate_sizes() {
    assert_eq!(window(0, 0, 10), 0..0, "no rows");
    assert_eq!(window(5, 0, 0), 0..0, "no height");
    assert_eq!(window(3, 1, 10), 0..3, "fewer rows than height");
}

#[test]
fn row_lookup_finds_a_line_pushed_down_by_a_tall_box() {
    let diff = diff_of(&["a", "b", "c"]);
    let comment = comment_with_body("a body long enough to wrap several times over");
    let plan = plan(&diff, &|line| if line == 0 { vec![&comment] } else { Vec::new() },
                    &HashSet::new(), 12);
    let row = plan.row_of_line(1).expect("line 1 has a row");
    assert!(row > 1, "the box pushed line 1 down the screen");
    assert!(matches!(plan.rows[row], Row::Diff { index: 1, .. }));
}
```

Write the `diff_of`, `comment_with_body`, and `comment_with_id_and_body` helpers at the top of the test file — `diff_of` builds a `FileDiff` with one `Context` line per string, `left` and `right` both set to the 1-based index.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test rows`
Expected: FAIL — unresolved import `rv::rows`.

- [ ] **Step 3: Implement `rows.rs`**

Build the row list by walking the diff's lines in order, pushing a `Row::Diff` for each and then, for each of that line's comments, either one `BoxCollapsed` row or a `BoxTop`, one `BoxBody` per wrapped line of the body (plus the wrapped reply prefixed `reply:`), and a `BoxBottom`.

Wrap on whitespace with a hard break for a word longer than the width, so no row exceeds `width` and no character is dropped. Guard `width == 0` by treating it as 1, so wrapping always makes progress and cannot loop forever.

`window(rows, anchor, height)`: return `0..0` when either `rows` or `height` is 0; when `rows <= height` return `0..rows`; otherwise centre on `anchor` and clamp to `0..rows`, returning exactly `height` rows.

`row_of_line` and `row_of_comment` scan for the matching row. Linear is right at review sizes; do not add an index.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test rows`
Expected: PASS.

- [ ] **Step 5: Add property tests**

```rust
proptest! {
    #[test]
    fn every_comment_appears_exactly_once(bodies in prop::collection::vec("[ -~]{0,80}", 0..5)) {
        let diff = diff_of(&["a", "b", "c"]);
        let comments: Vec<Comment> = bodies.iter().enumerate()
            .map(|(index, body)| comment_with_id_and_body(&format!("{index:08}"), body))
            .collect();
        let refs: Vec<&Comment> = comments.iter().collect();
        let plan = plan(&diff, &|line| if line == 0 { refs.clone() } else { Vec::new() },
                        &HashSet::new(), 30);
        for comment in &comments {
            let tops = plan.rows.iter().filter(|row| matches!(row,
                Row::BoxTop { comment: c, .. } if c.id == comment.id)).count();
            prop_assert_eq!(tops, 1, "each comment has exactly one box");
        }
    }

    #[test]
    fn planning_never_panics(width in 0usize..40, body in "[ -~]{0,200}") {
        let diff = diff_of(&["a", "b"]);
        let comment = comment_with_body(&body);
        let plan = plan(&diff, &|_| vec![&comment], &HashSet::new(), width);
        prop_assert!(plan.rows.len() >= 2);
    }
}
```

- [ ] **Step 6: Run the whole target, then commit**

Run: `cargo test -p rv --test rows`

```bash
jj describe -m "feat(rv): row model for diff lines and their comment boxes" && jj new
```

---

## Task 5: Render the boxes and the focused pane

**Files:**
- Modify: `rv/src/ui.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Consumes: `rv::rows::{plan, window, Row}`, `App::{comments_for_line, focus, collapsed}`.
- Produces: no new public API; `draw` now renders comment boxes and focus.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_comment_renders_as_a_blue_bordered_box_under_its_line() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");

    let buffer = render(&app, 100, 24);
    let text = buffer_text(&buffer);
    assert!(text.contains("needs a doc"), "the body is on screen");
    assert!(text.contains('╭') && text.contains('╰'), "the box has borders");
    assert!(
        styled_blue(&buffer, '╭'),
        "the border is blue, which is the requirement"
    );
}

#[test]
fn the_focused_pane_is_marked() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    let diff_focused = buffer_text(&render(&app, 100, 24));
    app.on_key(KeyCode::Left).expect("focus files");
    let files_focused = buffer_text(&render(&app, 100, 24));
    assert_ne!(diff_focused, files_focused, "focus is visible on screen");
    assert!(files_focused.contains("▸"), "the focused pane's title is marked");
}

#[rstest]
#[case(1, 1)]
#[case(2, 5)]
#[case(20, 3)]
fn drawing_never_panics_at_awkward_sizes(#[case] width: u16, #[case] height: u16) {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    let _ = render(&app, width, height);
}
```

Write `render(app, width, height) -> Buffer` using `ratatui::backend::TestBackend`, `buffer_text(&Buffer) -> String` concatenating the cells, `styled_blue(&Buffer, char) -> bool` finding the first cell holding that char and checking `cell.style().fg == Some(Color::Blue)`, and `save_comment(&mut App, &str)` driving `c`, the characters, and `Enter`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — no box characters on screen.

- [ ] **Step 3: Implement the rendering**

Replace `body()`'s windowing with the row model: build the plan from the selected diff, `app.comments_for_line`, `app.collapsed()`, and the pane's inner width; choose the anchor row with `plan.row_of_comment(...)` when focus is `Comments` and `plan.row_of_line(app.line_index())` otherwise; window over rows; map each `Row` to a styled `Line`.

Box rows are indented to the diff gutter width (the existing 5-column number field plus one space plus the sigil, so 7). Style border and title rows `Color::Blue`; leave `BoxBody` text at the default foreground so it stays readable; when focus is `Comments` and the row belongs to the selected comment, use `Color::LightBlue` with `Modifier::BOLD`.

For focus, give each pane's `Block` a `▸ ` title prefix and `Modifier::BOLD` border style when it holds focus, and drop the sidebar's `REVERSED` highlight to a dim style when it does not.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS, including the pre-existing `frame_renders_file_list_and_diff`.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): render inline comment boxes and the focused pane" && jj new
```

---

## Task 6: The comment stack

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Produces: `pub fn comment_index(&self) -> usize`, `pub fn selected_comment(&self) -> Option<&Comment>`; `Enter` enters the stack, `Esc`/`Left` leave it, `j`/`k`/Up/Down move within it, `c` adds another comment to the same line.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn enter_steps_into_the_comment_stack_and_esc_leaves_it() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(app.selected_comment().expect("selected").body, "needs a doc");

    app.on_key(KeyCode::Esc).expect("leave");
    assert_eq!(app.focus(), Focus::Diff);
}

#[test]
fn enter_on_a_line_without_comments_says_so_and_stays_put() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Enter).expect("enter");
    assert_eq!(app.focus(), Focus::Diff, "focus did not move");
    assert!(app.status().contains("no comments"), "and it said why");
}

#[test]
fn arrows_move_between_the_comments_in_a_stack() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    save_comment(&mut app, "second finding");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.selected_comment().expect("first").body, "first finding");
    app.on_key(KeyCode::Down).expect("next");
    assert_eq!(app.selected_comment().expect("second").body, "second finding");
    app.on_key(KeyCode::Down).expect("past the end");
    assert_eq!(app.selected_comment().expect("still second").body, "second finding");
    app.on_key(KeyCode::Up).expect("back");
    assert_eq!(app.selected_comment().expect("first again").body, "first finding");
}

#[test]
fn c_from_the_stack_adds_another_comment_to_the_same_line() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    save_comment(&mut app, "second finding");

    assert_eq!(app.comments_for_line(line).len(), 2, "both are on the line");
}

#[test]
fn left_also_leaves_the_stack() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Enter).expect("enter");
    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Diff);
}
```

`save_comment` must work from either focus, since the fourth test calls it from the stack.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — `no method named selected_comment`.

- [ ] **Step 3: Implement the stack**

Add `comment_index: usize` to `App`. Handle `KeyCode::Enter` in `on_key_browse`: from `Focus::Diff`, if `comments_for_line(self.line_index)` is non-empty, set `focus = Focus::Stack` and `comment_index = 0`; otherwise set the status to `"no comments on this line"`. Handle `KeyCode::Esc` in `on_key_browse` to leave the stack. Fill in the `Focus::Stack` arms of `move_forward`/`move_back` to move `comment_index` within the stack's length, saturating at both ends. Reset `comment_index` to 0 whenever the line or file selection changes.

`selected_comment` returns `comments_for_line(self.line_index).get(self.comment_index).copied()`.

`commit_comment` already anchors to `self.line_index`, so `c` from the stack lands on the same line with no change. After saving, keep focus where it was; the new comment appears at the end of the stack.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): step into a line's comment stack and navigate it" && jj new
```

---

## Task 7: Delete a comment, with confirmation

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Consumes: `Store::remove_comment` (Task 1), `session::write_markdown`.
- Produces: `Mode::ConfirmDelete { id: String, label: String }`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn d_then_y_deletes_the_comment_from_the_store_and_the_markdown() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(matches!(app.mode(), Mode::ConfirmDelete { .. }), "it asked first");
    assert!(app.status().contains("delete"), "and said what it would delete");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.comments_for_line(line).is_empty(), "gone from the view");
    assert!(
        fresh_store(&fixture).comments().expect("read").is_empty(),
        "gone from a freshly opened store — comments.json is the authority"
    );
}

#[test]
fn deleting_a_comment_does_not_rewrite_the_export() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");

    // The export is written only by `rv render`; produce one, then delete.
    let markdown = fixture.root().join(".review/REVIEW-FEEDBACK.md");
    std::fs::write(&markdown, "<!-- rv:v1 -->\nstale on purpose\n").expect("seed an export");
    let before = std::fs::metadata(&markdown).expect("stat").modified().expect("mtime");

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let after = std::fs::metadata(&markdown).expect("stat").modified().expect("mtime");
    assert_eq!(before, after, "the export is not touched by a delete");
    assert_eq!(
        std::fs::read_to_string(&markdown).expect("read"),
        "<!-- rv:v1 -->\nstale on purpose\n",
        "and its contents are left exactly as they were"
    );
}

#[test]
fn d_then_anything_else_cancels_and_keeps_the_comment() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.comments_for_line(line).len(), 1, "still there");
    assert_eq!(fresh_store(&fixture).comments().expect("read").len(), 1);
}

#[test]
fn from_the_diff_d_targets_the_newest_and_reports_how_many_remain() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    save_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].body, "first finding", "the newest went");
    assert!(app.status().contains("1 of 2"), "and it said so: {}", app.status());
}

#[test]
fn from_the_stack_d_targets_the_selected_comment() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    save_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].body, "second finding", "the selected one went");
}

#[test]
fn deleting_the_last_comment_on_a_line_returns_focus_to_the_diff() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.focus(), Focus::Diff, "no cursor left in an empty stack");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — no `ConfirmDelete` variant.

- [ ] **Step 3: Implement deletion**

Add the variant and route it in `on_key`:

```rust
pub enum Mode {
    Browse,
    Comment,
    /// Waiting for `y` before removing `id`. Deletion is unrecoverable — the
    /// comment leaves `comments.json` and its snapshot is deleted — so a
    /// mistyped `d` while browsing must not cost written work.
    ConfirmDelete { id: String, label: String },
}
```

`Mode` can no longer be `Copy`; `mode()` returns `&Mode` or a clone. Update the existing call sites and tests accordingly, and keep `PartialEq` so the tests above compile.

`d` in `on_key_browse` picks the target — `selected_comment()` when focus is `Comments`, otherwise the last of `comments_for_line(line_index)` — sets `Mode::ConfirmDelete` and a status reading `delete comment at <path>:<line>? (y/n)`. With no comment to delete, set a status and stay in `Browse`.

In the confirm handler, `KeyCode::Char('y')` calls `remove_comment`, then `reload_comments()`, sets the status to `deleted 1 of N on this line`, and returns focus to `Diff` when the line's stack is now empty (otherwise clamps `comment_index`). It does **not** call `session::write_markdown` — the markdown is an export now, per the storage constraint above, and a delete leaves it alone until the next `rv render`. Any other key restores `Browse` with a `deletion cancelled` status. Either way the mode leaves `ConfirmDelete`, so no keystroke can leave the app stuck waiting.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): delete a comment behind a confirmation" && jj new
```

---

## Task 8: Collapse a box with `s`

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Interfaces:**
- Produces: `pub fn collapsed(&self) -> &HashSet<String>`; `s` toggles collapse.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn s_collapses_and_expands_the_boxes_on_the_selected_line() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Char('s')).expect("collapse");
    assert!(app.collapsed().contains(&id));
    app.on_key(KeyCode::Char('s')).expect("expand");
    assert!(!app.collapsed().contains(&id));
}

#[test]
fn from_the_stack_s_collapses_only_the_selected_box() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    save_comment(&mut app, "second finding");
    let first = app.comments_for_line(app.line_index())[0].id.clone();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('s')).expect("collapse the first");

    assert!(app.collapsed().contains(&first));
    assert_eq!(app.collapsed().len(), 1, "the other box is untouched");
}

#[test]
fn collapse_state_never_reaches_disk() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Char('s')).expect("collapse");

    let json = std::fs::read_to_string(fixture.root().join(".review/comments.json"))
        .expect("read comments");
    assert!(!json.contains("collaps"), "collapse is a view preference, not review state");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — `no method named collapsed`.

- [ ] **Step 3: Implement collapse**

Add `collapsed: HashSet<String>` to `App` with an accessor. Handle `KeyCode::Char('s')`: from `Focus::Stack` toggle the selected comment's id; from `Focus::Diff` toggle every comment on the selected line, collapsing them all unless they are already all collapsed, in which case expand them all. Never persist it.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): collapse and expand comment boxes" && jj new
```

---

## Task 9: Document the new keymap

**Files:**
- Modify: `README.md`
- Test: `rv/tests/app.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[rstest]
#[case("Left")]
#[case("Right")]
#[case("Enter")]
#[case("d")]
#[case("s")]
fn the_readme_documents_every_new_binding(#[case] key: &str) {
    let readme = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"),
    )
    .expect("read README");
    assert!(readme.contains(key), "the README documents {key}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rv --test app`
Expected: FAIL for at least `Left`, `Right`, `d`, and `s`.

- [ ] **Step 3: Update the README**

Extend the keybinding tables with the focus keys, `Enter` into the comment stack, `Esc` out of it, `d` with its confirmation, and `s`. Add a short section describing inline comments: they render beneath the line they are anchored to, blue and bordered; a reply appears inside the same box; collapse is a per-session view preference. State plainly that deletion is permanent and confirmed with `y`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Run the whole suite and commit**

Run: `cargo test --workspace`
Expected: every test passes, output pristine, clippy and fmt clean.

```bash
jj describe -m "docs(rv): document inline comments, focus, and deletion" && jj new
```

---

## Task 10: Ctrl+C must abort (dogfood finding D1, safety)

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Background:** the first real user of rv found that `event_loop` passes only
`key.code` to `on_key`, so **Ctrl+C arrives as `KeyCode::Char('c')` and opens the
comment buffer.** In raw mode Ctrl+C is the universal abort, the terminal does not
generate SIGINT for us, and rv offers no other abort path — so a reviewer who
reaches for it gets a comment prompt and no way out but `Esc` then `q`.

**Interfaces:**
- Produces: `pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action>` —
  maps Ctrl+C to `Action::Quit`, otherwise delegates to `on_key(event.code)`.
  `on_key(KeyCode)` keeps its signature so every existing terminal-free test and
  the whole state machine are untouched.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn ctrl_c_quits_instead_of_opening_a_comment() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .expect("ctrl-c");

    assert_eq!(action, Action::Quit, "ctrl-c aborts the review");
    assert_eq!(app.mode(), &Mode::Browse, "and does not open the comment buffer");
}

#[test]
fn a_plain_c_still_opens_a_comment() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("c");

    assert_eq!(action, Action::Continue);
    assert_eq!(app.mode(), &Mode::Comment, "plain c is unchanged");
}
```

Import `crossterm::event::{KeyEvent, KeyModifiers}`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — `no method named on_key_event`.

- [ ] **Step 3: Implement it**

```rust
    /// Handles one key press, including its modifiers.
    ///
    /// Ctrl+C is intercepted here rather than in [`App::on_key`] because the
    /// state machine is written against plain [`KeyCode`]s and is tested without
    /// a terminal. In raw mode the terminal does not raise SIGINT for us, so
    /// without this a reviewer's reflexive abort would type a comment instead:
    /// `Char('c')` with CONTROL held is indistinguishable from a plain `c` once
    /// the modifiers are dropped.
    pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action> {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('c'))
        {
            return Ok(Action::Quit);
        }
        self.on_key(event.code)
    }
```

Then change `event_loop` to call `self.on_key_event(key)?` instead of
`self.on_key(key.code)?`, keeping the existing `KeyEventKind::Press` guard.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj describe -m "fix(rv): let ctrl-c abort the review instead of typing a comment" && jj new
```

---

## Task 11: Never clip content silently (dogfood finding D3)

**Files:**
- Modify: `rv/src/ui.rs`
- Test: `rv/tests/app.rs`

**Background:** neither `Paragraph` wraps or scrolls horizontally. Diff lines
clipped at 75 columns in the reviewer's terminal — and this repo contains 107- and
154-character lines — while comment bodies past 118 characters were typed blind. A
review tool that silently hides the code being judged, or the comment being
written, is failing at its one job.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_long_diff_line_is_marked_rather_than_silently_clipped() {
    let fixture = Fixture::with_long_line(200);
    let app = app_from(&fixture);

    let text = buffer_text(&render(&app, 60, 24));

    assert!(
        text.contains('…'),
        "a clipped line says so; silent truncation hides the code under review"
    );
}

#[test]
fn the_comment_buffer_shows_the_tail_while_typing_past_the_width() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Char('c')).expect("begin");
    let typed: String = std::iter::repeat_n('x', 200).collect();
    for character in typed.chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }

    let text = buffer_text(&render(&app, 40, 24));

    assert!(
        text.contains("xxx"),
        "what is being typed is on screen rather than scrolled off"
    );
}
```

`Fixture::with_long_line(n)` writes and commits a file containing one line of `n`
characters. Add it next to the existing fixture helpers.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — no ellipsis marker; the comment bar shows the head of the buffer.

- [ ] **Step 3: Implement it**

For diff lines: truncate to the pane's inner width and append `…` when the text
was longer, so the reviewer can see that there is more. Do **not** wrap diff lines
— wrapping breaks the one-row-per-diff-line assumption the row model in Task 4
depends on, and a reviewer counting lines against a file needs the correspondence.
Truncate by characters, not bytes.

For the comment bar: keep the **tail** of the buffer visible by rendering the last
`width` characters, so typing always shows what is being typed.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj describe -m "fix(rv): mark clipped diff lines and follow the comment buffer's tail" && jj new
```

---

## Task 12: Keep your place, and anchor to the right commit (dogfood findings D5, D2)

**Files:**
- Modify: `rv/src/app.rs`
- Test: `rv/tests/app.rs`

**Background:** two findings from the same read of the file.
`select_file` resets `line_index` to 0, so `]` then `[` loses the reviewer's place
— the agent spent 2,200 of 11,101 keystrokes on `j` and `]`. Separately,
`commit_comment` records `session.head_commit` for a `Side::Left` anchor even
though the text was read from the base blob. `commit_id` is advisory, but its one
job is reading the old blob while that commit still exists, and pointing it at the
wrong side defeats exactly that.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn leaving_a_file_and_coming_back_keeps_your_place() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char('j')).expect("down");
    let was = app.line_index();
    assert!(was > 0, "the fixture has enough lines to move");

    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('[')).expect("back");

    assert_eq!(app.line_index(), was, "the line came back with the file");
}

#[test]
fn a_left_side_comment_records_the_base_commit() {
    let fixture = Fixture::renamed();
    let mut app = app_from(&fixture);
    select_removed_line(&mut app);
    save_comment(&mut app, "you should not have removed this");

    let comment = &app.comments()[0];
    assert_eq!(comment.anchor.side, Side::Left);
    assert_eq!(
        comment.commit_id,
        app.session().base_commit,
        "a comment on removed text points at the commit that still has that text"
    );
}
```

`Fixture::renamed` and `select_removed_line` already exist from milestone 1's
Left-side anchor test; reuse them rather than writing new ones. Add
`pub fn session(&self) -> &Session` to `App` if it is not already exposed.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — the line resets to 0, and the commit id is the head.

- [ ] **Step 3: Implement it**

Store a per-file line position: replace the single `line_index` reset with a
`Vec<usize>` parallel to `files`, or remember the position on the way out and
restore it on the way in. Clamp the restored position to the file's line count,
since a diff may be shorter than where the reviewer last was.

In `commit_comment`, choose the commit the same way the path and line are already
chosen — from the anchored side: base for `Side::Left`, head otherwise. Put the
side, path, line, and commit selection in one place so they cannot drift, the same
reasoning that produced `anchored_side`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS, and milestone 1's existing Left-side anchor test still passes.

- [ ] **Step 5: Commit**

```bash
jj describe -m "fix(rv): keep the line position per file and anchor left comments to the base" && jj new
```

---

## Task 13: Browse comments in the sidebar and jump to the code

**Files:**
- Modify: `rv/src/app.rs`, `rv/src/ui.rs`
- Test: `rv/tests/app.rs`

**Background:** the reviewer should be able to move through comments the way they
move through files, and jump straight to the code a comment is about, rather than
traversing the diff to find their own remarks. The first real session on this tool
spent 2,200 of 11,101 keystrokes on `j` and `]`, with a known line costing up to
398 presses.

**Interfaces:**
- Produces:
  - `pub enum SidebarTab { Files, Comments }`
  - `pub fn sidebar_tab(&self) -> SidebarTab`
  - `pub fn browser_index(&self) -> usize` — the selected row of the Comments tab
  - `pub fn browsed_comment(&self) -> Option<&Comment>`
  - `Tab` switches the sidebar's tab from any focus; `Enter` on a Comments row
    jumps to that comment's code.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tab_switches_the_sidebar_between_files_and_comments() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    assert_eq!(app.sidebar_tab(), SidebarTab::Files, "files by default");

    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    app.on_key(KeyCode::Tab).expect("tab back");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
}

#[test]
fn the_comment_browser_lists_every_comment_in_the_review() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "first finding");
    app.on_key(KeyCode::Char(']')).expect("next file");
    save_comment(&mut app, "second finding");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    assert_eq!(app.browsed_comment().expect("first row").body, "first finding");
    app.on_key(KeyCode::Down).expect("next row");
    assert_eq!(app.browsed_comment().expect("second row").body, "second finding");
}

#[test]
fn enter_on_a_comment_row_jumps_to_its_code() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    // Comment on the second file, then walk away to the first.
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('j')).expect("move down");
    let commented_file = app.file_index();
    let commented_line = app.line_index();
    save_comment(&mut app, "look at this");
    app.on_key(KeyCode::Char('[')).expect("back to the first file");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Enter).expect("jump");

    assert_eq!(app.file_index(), commented_file, "landed on the right file");
    assert_eq!(app.line_index(), commented_line, "and the right line");
    assert_eq!(app.focus(), Focus::Diff, "with the diff focused, ready to act");
    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the comment is on the line we landed on"
    );
}

#[test]
fn d_from_the_comment_browser_deletes_behind_the_same_confirmation() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(matches!(app.mode(), Mode::ConfirmDelete { .. }), "it asked first");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert!(fresh_store(&fixture).comments().expect("read").is_empty());
}

#[test]
fn the_comment_browser_renders_path_line_and_state() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    save_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Tab).expect("comments tab");

    let text = buffer_text(&render(&app, 100, 24));

    assert!(text.contains("a.rs"), "the file is named");
    assert!(text.contains("needs a doc"), "the body is previewed");
    assert!(text.contains("open"), "the state is shown");
}

#[test]
fn an_empty_comment_browser_says_so() {
    let fixture = Fixture::new();
    let mut app = app_from(&fixture);
    app.on_key(KeyCode::Tab).expect("comments tab");

    let text = buffer_text(&render(&app, 100, 24));

    assert!(text.contains("no comments"), "an empty review explains itself");
}
```

The second test needs a fixture with at least two files, which Task 3 already
required.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app`
Expected: FAIL — `SidebarTab` not found.

- [ ] **Step 3: Implement the browser**

```rust
/// What the left column is listing. The sidebar browses comments the same way it
/// browses files, so a reviewer has one navigation idiom rather than two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    Files,
    Comments,
}
```

Add `sidebar_tab: SidebarTab` (default `Files`) and `browser_index: usize` to
`App`. Handle `KeyCode::Tab` in `on_key_browse` from any focus: flip the tab and
clamp `browser_index` to the comment count.

Route movement: when `focus == Focus::Sidebar` and the tab is `Comments`, `j`/`k`
move `browser_index` within `self.comments.len()`; when the tab is `Files`, they
move the file selection exactly as Task 3 implemented.

`browsed_comment` returns `self.comments.get(self.browser_index)`.

`Enter` from the Comments tab jumps:

```rust
    /// Selects the file and line a comment is anchored to and hands focus to the
    /// diff, so reading a comment and looking at the code are one keystroke apart.
    ///
    /// Two honest failure cases, both reported rather than papered over: the
    /// anchored file may no longer be in the review's file list (the range moved
    /// under the comment), and the anchored line may not be present in the
    /// current diff (the content moved). In the second case we still open the
    /// file — being in the right file with a warning beats staying put.
    fn jump_to_comment(&mut self, index: usize) -> Result<()> {
        let Some(comment) = self.comments.get(index) else {
            return Ok(());
        };
        let anchor = comment.anchor.clone();

        let Some(file_index) = self.review.files.iter().position(|file| {
            file.path == anchor.file || file.source_path.as_deref() == Some(anchor.file.as_str())
        }) else {
            self.status = format!("{} is not in this review's range any more", anchor.file);
            return Ok(());
        };

        self.file_index = file_index;
        self.load_selected()?;

        match self.line_of_anchor(&anchor) {
            Some(line) => {
                self.line_index = line;
                self.status = format!("{}:{}", anchor.file, anchor.line);
            }
            None => {
                self.line_index = 0;
                self.status = format!(
                    "{}: line {} is not in this diff any more",
                    anchor.file, anchor.line
                );
            }
        }
        self.focus = Focus::Diff;
        Ok(())
    }

    /// The diff line index whose anchor key matches `anchor`, using the same key
    /// function the save path uses so a jump can never disagree with storage.
    fn line_of_anchor(&self, anchor: &Anchor) -> Option<usize> {
        let diff = self.selected_diff()?;
        (0..diff.lines.len()).find(|index| {
            let line = &diff.lines[*index];
            self.anchor_key(line)
                == Some((anchor.file.clone(), anchor.side, anchor.line))
        })
    }
```

`d` from the Comments tab targets `browsed_comment()`; wire it into the same
`Mode::ConfirmDelete` path Task 7 built rather than duplicating the deletion.

In `ui.rs`, render the sidebar from the tab: the Files list as today, or one row
per comment showing `path:line`, the state, and the first line of the body,
truncated to the column width. An empty Comments tab renders `no comments yet`.
Title the block `Files (n)` or `Comments (n)` so the tab is unmistakable.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p rv --test app`
Expected: PASS.

- [ ] **Step 5: Add a property test**

```rust
proptest! {
    #[test]
    fn jumping_to_any_comment_lands_on_a_line_that_shows_it(count in 1usize..4) {
        let fixture = Fixture::new();
        let mut app = app_from(&fixture);
        for index in 0..count {
            app.on_key(KeyCode::Char('j')).expect("move");
            save_comment(&mut app, &format!("finding {index}"));
        }
        for index in 0..count {
            app.jump_to_comment_for_test(index).expect("jump");
            let on_line = app.comments_for_line(app.line_index());
            prop_assert!(
                on_line.iter().any(|c| c.id == app.comments()[index].id),
                "the comment jumped to is visible on the line landed on"
            );
        }
    }
}
```

Expose `jump_to_comment` to the test target under a clearly named
`#[doc(hidden)] pub fn jump_to_comment_for_test`, or drive it through `Tab`,
`Left`, `Down`×index, `Enter` if you prefer no test-only surface — the second is
better if it stays readable.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): browse comments in the sidebar and jump to their code" && jj new
```

---

## Self-review

Task 13 comes from the user's request to move through comments the way one moves
through files; it reuses `anchor_key` from Task 2 for the jump so a jump and a save
can never disagree about which line a comment belongs to, and it reuses Task 7's
confirmation rather than adding a second deletion path.

Tasks 10-12 come from the dogfood self-review rather than the spec; they are in
this plan because they touch the same two files and would otherwise need their own
review cycle. D6 (the context excerpt not marking which of its 11 lines is
anchored) is deliberately **not** here: fixing it honestly needs a new `Anchor`
field recording where the context starts, which is a stored-format change and
belongs with the anchoring work, not with rendering. D7 (the header reading as
though the whole repo were new) is correct behaviour with misleading presentation
in a remote-less repo, and is left alone.

Spec coverage, section by section. §3's keymap table maps to Tasks 3, 6, 7, and 8, with the `Enter`-on-empty-line and `Right`-from-stack rulings tested in Task 6 and Task 3 respectively. §4's box shape, blue border, reply-in-box, and collapsed one-liner map to Tasks 4 and 5; focus indication to Task 5; the windowing rework to Task 4. §5's comment loading and the shared `anchored_side` key to Task 2. §6's `remove_comment` and its write ordering to Task 1. §7's test list is distributed across the tasks that own each behaviour, with the row-model property tests in Task 4 step 5. §8's non-goals are respected: no editing, no state transitions, no persisted collapse, no mouse, no sidebar counts.

Type consistency: `Focus` is defined in Task 3 and consumed in 5, 6, 7, 8; `Mode::ConfirmDelete` in Task 7 changes `Mode` from `Copy`, which Task 7 step 3 calls out explicitly because it touches existing call sites; `comments_for_line` returns `Vec<&Comment>` and is used that way in every later task; `plan`/`window`/`Row` are defined in Task 4 and consumed only in Task 5.

Ordering: Task 1 has no dependencies; 2 depends on nothing but the store; 3 is independent; 4 depends on 2 only for the comment type; 5 depends on 4 and 3; 6 depends on 2 and 3; 7 depends on 1 and 6; 8 depends on 3 and 6; 9 depends on all. No task requires a later one.
