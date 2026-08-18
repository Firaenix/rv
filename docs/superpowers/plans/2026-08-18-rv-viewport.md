# rv Viewport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the panes resizable, put the whole keymap behind `?`, colour the code inside the green and red, let the sidebar be a directory tree, and make the mouse work.

**Architecture:** One pure `layout()` function computes every rectangle; `draw` paints from it and `hit` reads from it, so a click can never land somewhere different from what was drawn. Highlight spans are produced in `rv-core` as plain data from tree-sitter and mapped to styles in `rv`, keeping the terminal-free boundary. Every new piece of state is session-only — nothing here reaches `.review/`.

**Tech Stack:** Rust 2024, ratatui 0.30, crossterm 0.29, tree-sitter 0.26 + tree-sitter-highlight, rstest + proptest (dev).

**Spec:** `docs/superpowers/specs/2026-08-18-rv-viewport-design.md`

## Global Constraints

- **`rv-core` MUST NOT depend on `ratatui`, `crossterm`, or `tui-textarea`.** `rv-core/tests/constraints.rs` enforces this and fails the build. Highlight spans are plain data; the mapping to colours lives in `rv`.
- **`jj_lib` appears only in `rv-core/src/vcs.rs`.** Same enforcement.
- **Never read the user's jj config** — no `config_path`, no `ConfigSource::User`, no `ConfigSource::Repo`. Same enforcement.
- **`App::on_key` and `App::on_mouse` stay terminal-free.** They take crossterm event *values* and return `Action`; neither may touch a terminal or a `Frame`. This is what makes the state machine testable without a pty, and every wave so far has depended on it.
- **The terminal is restored on every exit path including panic.** The existing hook does this; do not disturb it.
- **`rv` writes only under `.review/`** plus the one line in `.git/info/exclude`, always through the store's atomic helper.
- **One side rule.** Anything that asks "which side does this diff line belong to" calls `rv::app::anchored_side(kind)` — `Side::Left` for `LineKind::Removed`, `Side::Right` otherwise. This project shipped one bug where the pane and the anchor disagreed, and nearly shipped a second where a jump ignored the side; a third copy of that logic is how it comes back.
- **One layout.** After Task 1 no file computes a `Rect` outside `layout()`. If you need a rectangle, get it from the `Layout`.
- **One binding table.** After Task 3 no key is dispatched that is not in `BINDINGS`, and no entry is in `BINDINGS` that is not dispatched.
- **The arrows are the binding; `hjkl` are aliases.** Everywhere the keymap is presented — the `?` popup, the status bar, the README — the arrow leads and the vim key follows in parentheses: `↓ (j)`, `↑ (k)`, `← (h)`, `→ (l)`. rv is a tool a reviewer may open once a week, and the arrows are the keys someone can find without being told. Add `h` and `l` as aliases for Left and Right so the vim set is complete rather than half-present. This changes presentation order and adds two aliases; it removes nothing.
- **Every new preference is session-only.** No config file, no persistence, nothing new in `.review/`.
- **Never mutate a file in this repository to show a test can fail.** Vendor a copy into the scratch directory and mutate that. A coordinator once read a source file while another agent had it temporarily mutated, mistook the mutation for a shipped bug, and reported it up the chain before checking against git.
- Commits use jj: `jj describe -m "…" && jj new`, with the repo's two trailer lines.

### Interfaces that already exist

```rust
// rv/src/app.rs
pub enum Focus { Sidebar, Diff, Stack }
pub enum SidebarTab { Files, Comments }
pub enum Mode { Browse, Comment, ConfirmDelete { id: String, label: String } }
pub enum Action { Continue, Quit }
impl App {
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action>;
    pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action>;   // intercepts Ctrl+C
    pub fn focus(&self) -> Focus;
    pub fn sidebar_tab(&self) -> SidebarTab;
    pub fn files(&self) -> &[FileChange];
    pub fn file_index(&self) -> usize;
    pub fn line_index(&self) -> usize;
    pub fn comments(&self) -> &[Comment];
    pub fn comments_for_line(&self, index: usize) -> Vec<&Comment>;
    pub fn collapsed(&self) -> &HashSet<String>;
    pub fn comment_index(&self) -> usize;
    pub fn selected_comment(&self) -> Option<&Comment>;
    pub fn mode(&self) -> Mode;
    pub fn status(&self) -> &str;
    pub fn buffer(&self) -> &str;
}
pub fn anchored_side(kind: LineKind) -> Side;

// rv/src/rows.rs — the row model the diff pane already renders from
pub enum Row<'a> { Diff { index: usize, line: &'a DiffLine }, BoxTop { .. }, BoxBody { .. }, BoxCollapsed { .. }, BoxBottom { .. } }
pub struct Plan<'a> { pub rows: Vec<Row<'a>> }
pub fn plan<'a>(diff: &'a FileDiff, comments_for: &dyn Fn(usize) -> Vec<&'a Comment>, collapsed: &HashSet<String>, width: usize) -> Plan<'a>;
pub fn window(rows: usize, anchor: usize, height: usize) -> Range<usize>;
```

Read these in the tree before planning anything. Where this plan disagrees with the tree, **the tree wins and you say so in your report.**

---

## Task 1: One layout, two consumers

**Files:** Create `rv/src/layout.rs`, `rv/tests/layout.rs`; modify `rv/src/lib.rs`, `rv/src/ui.rs`

**Produces:**

```rust
pub struct Split { ratio: u16 }
impl Split {
    pub const DEFAULT: u16 = 30;
    pub const MIN_SIDEBAR: u16 = 12;
    pub const MIN_DIFF: u16 = 20;
    pub fn new(ratio: u16) -> Self;
    pub fn ratio(self) -> u16;
    pub fn nudged(self, delta: i16) -> Self;          // clamped to 5..=80
    pub fn sidebar_width(self, total: u16) -> u16;    // honours the minimums, or halves when it cannot
}

pub struct Layout {
    pub sidebar: Rect, pub divider: Rect, pub diff: Rect,
    pub bar: Rect,              // along the BOTTOM, under both panes
    pub popup: Option<Rect>,
    pub toast: Option<Rect>,    // floating, top-centre
}
pub struct Chrome { pub bar_rows: u16, pub help_open: bool, pub toast: bool }
pub enum Target { SidebarRow(usize), DiffRow(usize), Divider, Bar, Popup }

pub fn layout(area: Rect, split: Split, chrome: Chrome) -> Layout;
pub fn hit(layout: &Layout, column: u16, row: u16) -> Option<Target>;
```

**The bar moves to the bottom in this task.** It is drawn above the panes today; every terminal multiplexer puts it below, and so does the spec. There is no `Target::Toast` — the toast is drawn over the panes but is never a click target.

`SidebarRow` and `DiffRow` are indices **within the pane's inner area**, so row 0 is the first row under the pane's top border. The caller adds its own scroll offset.

- [ ] **Step 1: Write the failing tests** in `rv/tests/layout.rs`

```rust
#[test]
fn the_bar_sits_along_the_bottom_under_both_panes() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    assert_eq!(l.bar.height, 1);
    assert_eq!(l.bar.bottom(), 24, "the bar is the last row of the area");
    assert_eq!(l.bar.width, 100, "it spans both panes");
    assert_eq!(l.sidebar.bottom(), l.bar.y, "the panes stop where the bar starts");
    assert_eq!(l.diff.bottom(), l.bar.y);
    assert_eq!(l.sidebar.y, 0, "and start at the top of the area");
}

#[test]
fn the_panes_tile_the_area_with_a_divider_between_them() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    assert_eq!(l.bar.height, 1);
    assert_eq!(l.sidebar.x, 0);
    assert_eq!(l.divider.width, 1, "the divider is one column");
    assert_eq!(l.sidebar.right(), l.divider.x, "no gap before the divider");
    assert_eq!(l.divider.right(), l.diff.x, "no gap after it");
    assert_eq!(l.diff.right(), 100, "the panes reach the right edge");
    assert_eq!(l.sidebar.height, l.diff.height, "the panes are the same height");
}

#[rstest]
#[case(100, 30)]
#[case(40, 30)]
#[case(24, 50)]
fn the_sidebar_honours_its_minimum_or_the_area_halves(#[case] width: u16, #[case] ratio: u16) {
    let l = layout(Rect::new(0, 0, width, 24), Split::new(ratio), Chrome { bar_rows: 1, help_open: false, toast: false });
    let sidebar = l.sidebar.width;
    let diff = l.diff.width;
    assert!(sidebar > 0 && diff > 0, "neither pane vanishes at width {width}");
    if width >= Split::MIN_SIDEBAR + Split::MIN_DIFF + 1 {
        assert!(sidebar >= Split::MIN_SIDEBAR, "sidebar keeps its floor when there is room");
        assert!(diff >= Split::MIN_DIFF, "the diff keeps its floor when there is room");
    }
}

#[test]
fn nudging_the_split_stays_inside_its_bounds() {
    let mut split = Split::new(Split::DEFAULT);
    for _ in 0..100 { split = split.nudged(2); }
    assert!(split.ratio() <= 80, "cannot be dragged past the right bound");
    for _ in 0..200 { split = split.nudged(-2); }
    assert!(split.ratio() >= 5, "cannot be dragged past the left bound");
}

#[test]
fn a_click_on_the_divider_reports_the_divider() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    assert_eq!(hit(&l, l.divider.x, 5), Some(Target::Divider));
}

#[test]
fn a_click_in_a_pane_reports_the_row_under_the_pointer() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    let first = l.diff.y + 1;                       // +1 for the pane's top border
    assert_eq!(hit(&l, l.diff.x + 3, first), Some(Target::DiffRow(0)));
    assert_eq!(hit(&l, l.diff.x + 3, first + 4), Some(Target::DiffRow(4)));
    assert_eq!(hit(&l, l.sidebar.x + 1, first + 2), Some(Target::SidebarRow(2)));
}

#[test]
fn a_click_outside_everything_reports_nothing() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    assert_eq!(hit(&l, 200, 200), None);
}

#[test]
fn the_popup_takes_priority_over_whatever_is_beneath_it() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), Chrome { bar_rows: 1, help_open: true, toast: false });
    let popup = l.popup.expect("the popup has a rect when it is open");
    assert_eq!(hit(&l, popup.x + 2, popup.y + 2), Some(Target::Popup));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test layout` — expect FAIL, unresolved import `rv::layout`.

- [ ] **Step 3: Implement `layout.rs`**

Compute the bar, then split the remainder into sidebar / one-column divider / diff. `sidebar_width` applies the ratio, then clamps to the minimums, and when `total` cannot satisfy both minimums it splits what there is evenly rather than starving one pane. Use saturating arithmetic everywhere — a `u16` subtraction that underflows is the classic ratatui panic and this project has already killed one such mutant.

`hit` tests the popup first, then the divider, then each pane, converting a row inside a pane to a zero-based index by subtracting the pane's `y` and its top border.

- [ ] **Step 4: Make `ui.rs` render from it**

Delete the inline `Layout::vertical`/`horizontal` calls in `ui::draw` and take the rectangles from `layout()`. Behaviour must not change: the same panes at the same places, and the whole existing suite still green.

- [ ] **Step 5: Add the round-trip property**

```rust
proptest! {
    #[test]
    fn every_painted_cell_hits_the_pane_that_painted_it(
        width in 8u16..120, height in 4u16..40, ratio in 5u16..80,
    ) {
        let area = Rect::new(0, 0, width, height);
        let l = layout(area, Split::new(ratio), Chrome { bar_rows: 1, help_open: false, toast: false });
        for (rect, name) in [(l.sidebar, "sidebar"), (l.diff, "diff")] {
            for row in rect.y + 1..rect.bottom() {
                for column in rect.x..rect.right() {
                    let target = hit(&l, column, row);
                    prop_assert!(target.is_some(), "{name} cell ({column},{row}) hits nothing");
                }
            }
        }
        prop_assert_eq!(hit(&l, l.divider.x, l.divider.y), Some(Target::Divider));
    }
}
```

Prove it can fail: vendor a copy, make `hit` off-by-one on the pane's top border, and show the property failing 3/3 with an unmutated control.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): one layout function that drawing and hit-testing share" && jj new
```

---

## Task 2: Resizable panes

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`

**Consumes:** `Split`, `layout` (Task 1). **Produces:** `App::split() -> Split`; `<` and `>` resize.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn angle_brackets_resize_the_panes() {
    let mut app = workspace().app();
    let start = app.split().ratio();
    app.on_key(KeyCode::Char('>')).expect(">");
    assert!(app.split().ratio() > start, "the sidebar grew");
    app.on_key(KeyCode::Char('<')).expect("<");
    assert_eq!(app.split().ratio(), start, "and shrank back");
}

#[test]
fn resizing_never_leaves_the_bounds_however_long_you_hold_it() {
    let mut app = workspace().app();
    for _ in 0..200 { app.on_key(KeyCode::Char('>')).expect(">"); }
    assert!(app.split().ratio() <= 80);
    for _ in 0..400 { app.on_key(KeyCode::Char('<')).expect("<"); }
    assert!(app.split().ratio() >= 5);
}

#[test]
fn a_resized_pane_actually_renders_at_its_new_width() {
    let mut app = workspace().app();
    let before = frame_at(&app, 100, 24);
    for _ in 0..5 { app.on_key(KeyCode::Char('>')).expect(">"); }
    let after = frame_at(&app, 100, 24);
    assert_ne!(before, after, "the frame reflects the resize");
}

#[test]
fn the_split_is_not_written_anywhere() {
    let workspace = workspace();
    let mut app = workspace.app();
    let tree = workspace_tree(&workspace);
    app.on_key(KeyCode::Char('>')).expect(">");
    assert_eq!(workspace_tree(&workspace), tree, "resizing is a view preference, not review state");
}
```

Use the helpers the tree actually has — `workspace()`, `frame_at`, `workspace_tree` — rather than inventing new ones.

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Hold `split: Split` in `App`, add the accessor, bind `<` and `>` in `on_key_browse` through the binding table once Task 3 lands (until then, plain match arms).

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): resize the panes with < and >" && jj new
```

---

## Task 3: One binding table, and the `?` popup

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**Produces:**

```rust
pub struct Binding { pub keys: &'static str, pub group: Group, pub what: &'static str }
pub enum Group { Move, Focus, Comment, View, Quit }
pub const BINDINGS: &[Binding];
impl App { pub fn help_open(&self) -> bool; }
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn question_mark_opens_the_help_and_esc_closes_it() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('?')).expect("?");
    assert!(app.help_open());
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(frame.contains("comment"), "the popup lists what the keys do");
    app.on_key(KeyCode::Esc).expect("esc");
    assert!(!app.help_open());
}

#[test]
fn q_closes_the_help_rather_than_quitting() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let action = app.on_key(KeyCode::Char('q')).expect("q");
    assert_eq!(action, Action::Continue, "q in help closes the help");
    assert!(!app.help_open());
    assert_eq!(app.on_key(KeyCode::Char('q')).expect("q"), Action::Quit, "and quits once it is closed");
}

#[rstest]
#[case(KeyCode::Char('c'))]
#[case(KeyCode::Char('d'))]
#[case(KeyCode::Char('j'))]
#[case(KeyCode::Enter)]
fn keys_are_inert_while_the_help_is_open(#[case] key: KeyCode) {
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let before = workspace_tree(&workspace);
    app.on_key(KeyCode::Char('?')).expect("?");
    let mode = app.mode();
    app.on_key(key).expect("key");
    assert_eq!(app.mode(), mode, "{key:?} did nothing while help was open");
    assert_eq!(workspace_tree(&workspace), before, "and wrote nothing");
}

#[test]
fn every_binding_the_handler_dispatches_appears_in_the_popup() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 120, 40));
    for binding in rv::app::BINDINGS {
        assert!(frame.contains(binding.keys), "the popup lists {}", binding.keys);
    }
}

#[test]
fn the_whole_keymap_fits_at_80x24_without_scrolling() {
    // 80x24 is what a reviewer over ssh actually has, and a keymap you must
    // scroll to read is a keymap you will not read. This is what forces the
    // column layout: twenty bindings in one list need twenty rows and do not
    // fit beside their own borders and headings.
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 80, 24));
    for binding in rv::app::BINDINGS {
        assert!(frame.contains(binding.keys), "{} is on screen at 80x24", binding.keys);
    }
    assert!(!frame.contains("more"), "nothing is hidden behind a scroll indicator");
}

#[test]
fn a_binding_that_does_nothing_here_is_dimmed_rather_than_hidden() {
    // `d` means nothing in the Files tab. A reviewer learning the tool should
    // see that the key exists and why it is inert, not wonder whether they
    // misread the manual.
    let mut app = workspace().app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = frame_at(&app, 100, 30);
    assert!(buffer_text(&frame).contains('d'), "the binding is still listed");
    assert!(is_dim(&frame, cell_of_binding(&frame, "d")), "and shown as inactive here");
}

#[test]
fn the_help_renders_in_a_pane_too_small_for_it() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let _ = frame_at(&app, 20, 6);   // must not panic
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Define `BINDINGS` once. `on_key_browse` must dispatch from it — a match whose arms are the table's keys, so a binding cannot exist without an entry. The popup renders the table grouped by `Group`, scrollable with `j`/`k` when it does not fit. While `help_open`, `on_key` handles only `?`, `Esc`, `q`, `j`, `k` and ignores everything else.

- [ ] **Step 5: Prove the anti-drift guarantee**

Vendor a copy, add a key to `on_key_browse` without adding it to `BINDINGS`, and show the "every binding appears" test failing — or, better, show that the shape of the code makes it impossible and say why in your report.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): a ? popup generated from the binding table" && jj new
```

---

## Task 4: Syntax highlighting

**Files:** Create `rv-core/src/highlight.rs`, `rv-core/tests/highlight.rs`; modify `rv-core/src/lib.rs`, `rv-core/Cargo.toml`, workspace `Cargo.toml`

**Produces:**

```rust
pub struct Span { pub line: u32, pub start: u32, pub end: u32, pub capture: Capture }
pub enum Capture { Keyword, Function, Type, String, Number, Comment, Punctuation, Variable, Constant, Other }
pub struct Highlights { spans: Vec<Span>, language: Option<&'static str> }
impl Highlights {
    pub fn of(source: &[u8], path: &str) -> Highlights;   // never fails; unknown language yields none
    pub fn language(&self) -> Option<&'static str>;
    pub fn line(&self, line: u32) -> &[Span];             // spans on that 1-based line, in column order
}
```

`rv-core` gains `tree-sitter` and `tree-sitter-rust`. **It must not gain ratatui** — `Capture` is plain data and `rv` maps it to colours.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rust_keywords_and_types_are_captured() {
    let source = b"fn parse(s: &str) -> Result<Ast> {\n    let raw = s.trim();\n}\n";
    let highlights = Highlights::of(source, "parse.rs");
    assert_eq!(highlights.language(), Some("rust"));
    let first = highlights.line(1);
    assert!(first.iter().any(|s| s.capture == Capture::Keyword), "fn is a keyword");
    assert!(first.iter().any(|s| s.capture == Capture::Function), "parse is a function name");
    assert!(highlights.line(2).iter().any(|s| s.capture == Capture::Keyword), "let is a keyword");
}

#[test]
fn spans_never_overlap_and_stay_inside_their_line() {
    let source = b"fn a() { let x = \"s\"; }\n";
    let highlights = Highlights::of(source, "a.rs");
    let line = highlights.line(1);
    let text_len = 23u32;
    for pair in line.windows(2) {
        assert!(pair[0].end <= pair[1].start, "spans are disjoint and ordered");
    }
    for span in line {
        assert!(span.start < span.end && span.end <= text_len, "a span stays inside its line");
    }
}

#[rstest]
#[case("notes.txt")]
#[case("Makefile")]
#[case("archive.tar.gz")]
fn a_file_with_no_grammar_reports_none_rather_than_guessing(#[case] path: &str) {
    let highlights = Highlights::of(b"anything at all\n", path);
    assert_eq!(highlights.language(), None);
    assert!(highlights.line(1).is_empty());
}

#[test]
fn source_that_does_not_parse_still_returns_something() {
    let highlights = Highlights::of(b"fn (((( unterminated\n", "broken.rs");
    let _ = highlights.line(1);   // must not panic; tree-sitter recovers
}

#[test]
fn invalid_utf8_is_not_a_panic() {
    let _ = Highlights::of(&[0xff, 0xfe, b'\n'], "weird.rs");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Use `tree-sitter-highlight` with the Rust grammar's `highlights.scm`, mapping its capture names onto `Capture`. Detect the language by extension only — no content sniffing, because guessing wrong is worse than rendering plain. Clamp every span to its line's length before returning it, so a consumer indexing by column cannot panic on a bad span.

- [ ] **Step 5: Add properties**

Totality over arbitrary bytes and arbitrary paths; spans always disjoint, ordered, and within the line; the same source highlighted twice gives identical spans. Prove each can fail against a vendored mutant.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv-core): tree-sitter highlight spans as plain data" && jj new
```

---

## Task 5: Paint the highlighting

**Files:** Modify `rv/src/ui.rs`, `rv/src/app.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**Consumes:** `Highlights` (Task 4), `layout` (Task 1), `anchored_side`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_added_line_has_a_green_wash_and_coloured_code() {
    let mut app = rust_workspace().app();
    let frame = frame_at(&app, 100, 24);
    let added = row_of_first_added_line(&frame);
    assert!(bg_of(&frame, added).is_some(), "the line carries a background tint");
    let foregrounds = distinct_foregrounds(&frame, added);
    assert!(foregrounds.len() > 1, "the code is syntax coloured, not one colour: {foregrounds:?}");
}

#[test]
fn a_removed_line_takes_its_colours_from_the_base_blob() {
    // A rewrite that does NOT move: same path, same line number on both sides,
    // opposite sides. A rename cannot catch a side-blind lookup, because a rename
    // already encodes the side in the path.
    let mut app = rewrite_workspace().app();
    let frame = frame_at(&app, 100, 24);
    assert_eq!(
        text_of_row(&frame, row_of_first_removed_line(&frame)).trim_start_matches(|c: char| !c.is_alphanumeric()),
        BASE_SIDE_FIRST_TOKEN,
        "the removed half shows the base blob's text, so its spans must come from there"
    );
}

#[test]
fn the_selected_line_is_not_drawn_with_reversed_video() {
    let app = rust_workspace().app();
    let frame = frame_at(&app, 100, 24);
    let selected = row_of_selected_line(&frame);
    assert!(!style_of_row(&frame, selected).add_modifier.contains(Modifier::REVERSED),
        "reversing swaps the tint and the syntax colours into each other");
}

#[test]
fn a_file_with_no_grammar_renders_plain_and_says_so() {
    let app = text_workspace().app();
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(frame.contains("no highlighting"), "the title says why the code is plain");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Cache `Highlights` per `(commit, path)` in `App`, parsed lazily on first render of that file, beside the existing diff cache. A diff line looks up its spans **on its own side** via `anchored_side`. Map `Capture` to the 16 ANSI colours. Tint by kind: `Added` dim green background, `Removed` dim red, `Context` none. Selection becomes a brighter background rather than `REVERSED`.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): syntax colours inside the green and red" && jj new
```

---

## Task 6: The sidebar as a tree

**Files:** Create `rv/src/tree.rs`, `rv/tests/tree.rs`; modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`

**Produces:**

```rust
pub struct Node { pub label: String, pub depth: usize, pub kind: NodeKind }
pub enum NodeKind {
    Commit { change_id: String, collapsed: bool },
    Dir { collapsed: bool },
    File { index: usize },
}

pub struct Group<'a> { pub change_id: &'a str, pub description: &'a str, pub paths: &'a [&'a str] }

/// The bookmark view: every changed file, as a directory tree or a flat list.
pub fn build(paths: &[&str], collapsed: &HashSet<String>, tree: bool) -> Vec<Node>;

/// The commits view: each change holds the files it touched, and `tree` chooses
/// whether those files are a directory tree or a flat list beneath it.
pub fn build_grouped(groups: &[Group<'_>], collapsed: &HashSet<String>, tree: bool) -> Vec<Node>;
```

**`NodeKind::Commit` is defined now even though the commits view itself lands with the navigation work.** A third node kind costs almost nothing while the tree is being written and is a retrofit afterwards. Implement `build_grouped` and test it; wiring `1`/`2` to it is not this plan's job.

A commit node is a directory in every respect that matters here: it collapses with `s`, it holds children, and it aggregates its subtree's gradient. One tree with three node kinds, not two widgets — a separate commits list would mean a second selection model, a second collapse rule and a second place to compute the gradient, and those would drift.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_single_child_chain_collapses_into_one_row() {
    let nodes = build(&["docs/superpowers/specs/a.md", "docs/superpowers/specs/b.md"], &HashSet::new());
    let dirs: Vec<&str> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Dir { .. })).map(|n| n.label.as_str()).collect();
    assert_eq!(dirs, ["docs/superpowers/specs"], "one row, not three");
}

#[test]
fn the_tree_lists_exactly_the_files_the_flat_list_does() {
    let paths = ["a.rs", "src/b.rs", "src/deep/c.rs", "d.rs"];
    let nodes = build(&paths, &HashSet::new());
    let mut files: Vec<usize> = nodes.iter().filter_map(|n| match n.kind { NodeKind::File { index } => Some(index), _ => None }).collect();
    files.sort_unstable();
    assert_eq!(files, [0, 1, 2, 3], "a tree that loses a file is worse than no tree");
}

#[test]
fn a_collapsed_directory_hides_its_files_but_stays_visible() {
    let collapsed = HashSet::from(["src".to_owned()]);
    let nodes = build(&["a.rs", "src/b.rs", "src/c.rs"], &collapsed);
    assert!(nodes.iter().any(|n| n.label == "src"), "the directory row remains");
    assert!(!nodes.iter().any(|n| n.label.ends_with("b.rs")), "its children are hidden");
    assert!(nodes.iter().any(|n| n.label.ends_with("a.rs")), "siblings are unaffected");
}

#[test]
fn t_toggles_the_sidebar_between_a_list_and_a_tree() {
    let mut app = workspace().app();
    let list = buffer_text(&frame_at(&app, 100, 24));
    app.on_key(KeyCode::Char('t')).expect("t");
    let tree = buffer_text(&frame_at(&app, 100, 24));
    assert_ne!(list, tree, "the sidebar changed shape");
}

#[test]
fn a_commit_holds_its_files_the_way_a_directory_holds_its_own() {
    let groups = [
        Group { change_id: "ytskpxpw", description: "close the alias bypass", paths: &["rv-core/tests/constraints.rs"] },
        Group { change_id: "zmomvwzm", description: "enforce the constraints", paths: &["rv-core/src/store.rs", "rv-core/tests/store.rs"] },
    ];
    let nodes = build_grouped(&groups, &HashSet::new(), true);

    let commits: Vec<&str> = nodes.iter().filter_map(|n| match &n.kind {
        NodeKind::Commit { change_id, .. } => Some(change_id.as_str()), _ => None }).collect();
    assert_eq!(commits, ["ytskpxpw", "zmomvwzm"], "one node per change, in order");
    assert!(nodes.iter().all(|n| matches!(n.kind, NodeKind::Commit { .. }) || n.depth > 0),
        "everything else hangs beneath a commit");
}

#[test]
fn collapsing_a_commit_hides_its_files_and_leaves_its_siblings_alone() {
    let groups = [
        Group { change_id: "aaaa", description: "first", paths: &["a.rs"] },
        Group { change_id: "bbbb", description: "second", paths: &["b.rs"] },
    ];
    let nodes = build_grouped(&groups, &HashSet::from(["aaaa".to_owned()]), false);
    assert!(nodes.iter().any(|n| matches!(&n.kind, NodeKind::Commit { change_id, .. } if change_id == "aaaa")),
        "the commit row remains");
    assert!(!nodes.iter().any(|n| n.label.ends_with("a.rs")), "its files are hidden");
    assert!(nodes.iter().any(|n| n.label.ends_with("b.rs")), "the other change is untouched");
}

#[test]
fn a_file_touched_by_two_commits_appears_under_each() {
    let groups = [
        Group { change_id: "aaaa", description: "first", paths: &["shared.rs"] },
        Group { change_id: "bbbb", description: "second", paths: &["shared.rs"] },
    ];
    let nodes = build_grouped(&groups, &HashSet::new(), false);
    let count = nodes.iter().filter(|n| n.label.ends_with("shared.rs")).count();
    assert_eq!(count, 2, "each change shows what it touched, not what is unique to it");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`s` on a directory row toggles it, reusing the project's existing verb for *collapse the thing under the cursor*. In the Comments tab `t` sets a status saying it applies to the file list.

- [ ] **Step 5: Add the conservation property** — for arbitrary path sets, the tree's file indices are exactly `0..paths.len()`. Prove it can fail.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): a collapsible directory tree in the sidebar" && jj new
```

---

## Task 7: Mouse

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**Produces:** `App::on_mouse(MouseEvent) -> Result<Action>`, terminal-free like `on_key`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn clicking_a_diff_line_selects_it_and_focuses_the_diff() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_mouse(click(60, 6)).expect("click in the diff");
    assert_eq!(app.focus(), Focus::Diff);
    assert!(app.line_index() > 0, "the clicked line is selected");
}

#[test]
fn scrolling_moves_the_view_without_moving_the_selection() {
    let mut app = workspace().app();
    let selected = app.line_index();
    app.on_mouse(scroll_down(60, 6)).expect("scroll");
    assert_eq!(app.line_index(), selected, "scrolling is looking, not choosing");
}

#[test]
fn dragging_the_divider_resizes_and_changes_nothing_else() {
    let mut app = workspace().app();
    let before = (app.file_index(), app.line_index(), app.focus());
    let divider = divider_column(&app, 100);
    app.on_mouse(press(divider, 6)).expect("press");
    app.on_mouse(drag(divider + 10, 6)).expect("drag");
    app.on_mouse(release(divider + 10, 6)).expect("release");
    assert!(app.split().ratio() > Split::DEFAULT, "the split followed the pointer");
    assert_eq!((app.file_index(), app.line_index(), app.focus()), before, "nothing else moved");
}

#[test]
fn no_gesture_deletes_anything() {
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let before = workspace_tree(&workspace);
    for event in [click(60, 6), click(10, 4), scroll_up(60, 6), press(40, 6), drag(50, 6), release(50, 6)] {
        app.on_mouse(event).expect("gesture");
    }
    assert_eq!(workspace_tree(&workspace), before, "the mouse cannot destroy review state");
}

#[test]
fn clicking_a_comment_box_selects_that_comment() {
    let mut app = workspace().app();
    write_comment(&mut app, "a finding");
    let row = row_of_first_box(&app, 100, 24);
    app.on_mouse(click(60, row)).expect("click the box");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(app.selected_comment().expect("selected").body, "a finding");
}
```

Write the `click`/`press`/`drag`/`release`/`scroll_*` helpers as thin `MouseEvent` constructors at the top of the test file.

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`App` remembers the last rendered `Layout` so `on_mouse` can consult `hit`; `ui::draw` stores it. Enable mouse reporting in `App::run` and disable it on every exit path including the panic hook — a terminal left in reporting mode prints escape noise on every click after rv exits, which is exactly the class of damage the existing panic hook was written to prevent.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): click, scroll and drag" && jj new
```

---

## Task 8: The change gradient

**Files:** Create `rv/src/gradient.rs`, `rv/tests/gradient.rs`; modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/tree.rs`, `rv/tests/app.rs`

**Consumes:** `rv_core::diff::compute_with` (the in-process `similar` path), `Node` (Task 6).

**Produces:**

```rust
pub struct Stat { pub added: u32, pub removed: u32 }
impl Stat {
    pub fn total(self) -> u32;
    pub fn added_ratio(self) -> Option<f32>;   // None when nothing changed
}

pub struct Rgb(pub u8, pub u8, pub u8);
pub const ADDED: Rgb;
pub const REMOVED: Rgb;

/// The seam the two halves meet at: a step brighter than the lighter endpoint in
/// Oklab `L`, capped short of white. Relative rather than absolute, so it reads
/// as a highlight on a dark terminal and does not vanish on a light one.
pub fn pivot() -> Rgb;

/// The colour for column `column` of a `width`-wide row at the given ratio.
/// Green on the left, red on the right, meeting at a tight `pivot()` seam:
/// each half only ever desaturates toward the pivot, so no cell is a mixture
/// of the two hues.
pub fn column_colour(ratio: f32, column: u16, width: u16) -> Rgb;

pub fn oklab_mix(a: Rgb, b: Rgb, t: f32) -> Rgb;
```

- [ ] **Step 1: Write the failing tests** in `rv/tests/gradient.rs`

```rust
#[test]
fn a_pure_addition_is_green_all_the_way_across() {
    for column in 0..40 {
        assert_eq!(column_colour(1.0, column, 40), ADDED, "column {column} is green");
    }
}

#[test]
fn a_pure_deletion_is_red_all_the_way_across() {
    for column in 0..40 {
        assert_eq!(column_colour(0.0, column, 40), REMOVED);
    }
}

#[test]
fn an_even_split_changes_hand_at_the_middle() {
    assert_eq!(column_colour(0.5, 0, 40), ADDED, "the left end is fully green");
    assert_eq!(column_colour(0.5, 39, 40), REMOVED, "the right end is fully red");
}

#[test]
fn the_boundary_is_blended_rather_than_a_hard_edge() {
    let middle: Vec<Rgb> = (17..23).map(|column| column_colour(0.5, column, 40)).collect();
    let distinct: std::collections::HashSet<_> = middle.iter().map(|c| (c.0, c.1, c.2)).collect();
    assert!(distinct.len() > 2, "the boundary interpolates: {middle:?}");
}

#[test]
fn the_seam_is_the_brightest_part_of_the_row() {
    // The whole point of pivoting through a light neutral: green and red sit at
    // opposite ends of Oklab's `a` axis, so blending them directly crosses a
    // dull mid-grey exactly where the eye is trying to read the boundary. The
    // seam must be brighter than both ends, not darker.
    let luma = |c: Rgb| 0.2126 * c.0 as f32 + 0.7152 * c.1 as f32 + 0.0722 * c.2 as f32;
    let seam = luma(column_colour(0.5, 20, 40));
    assert!(seam > luma(ADDED), "the seam is lighter than the green end");
    assert!(seam > luma(REMOVED), "and lighter than the red end");
}

#[test]
fn no_cell_is_ever_a_mixture_of_the_two_hues() {
    // Each half desaturates toward the pivot and back, so a cell is green-ish or
    // red-ish or neutral — never olive, never brown.
    for column in 0..40 {
        let Rgb(r, g, _) = column_colour(0.5, column, 40);
        let muddy = r > 90 && g > 90 && r.abs_diff(g) < 25 && (r as u16 + g as u16) < 380;
        assert!(!muddy, "column {column} is mud: {:?}", column_colour(0.5, column, 40));
    }
}

#[test]
fn the_seam_is_tight_enough_to_still_read_as_a_proportion() {
    // A wide blend destroys the thing the bar is drawing: you can no longer see
    // where two thirds ends and one third begins.
    let flat_green = (0..40).filter(|c| column_colour(0.66, *c, 40) == ADDED).count();
    let flat_red = (0..40).filter(|c| column_colour(0.66, *c, 40) == REMOVED).count();
    assert!(flat_green + flat_red >= 34, "at most a few columns are in the seam");
    assert!(flat_green > flat_red, "and two thirds still reads as two thirds");
}

#[rstest]
#[case(0.0)]
#[case(0.5)]
#[case(1.0)]
fn a_one_column_row_still_produces_a_colour(#[case] ratio: f32) {
    let _ = column_colour(ratio, 0, 1);
}

#[test]
fn a_file_with_no_line_changes_has_no_ratio() {
    assert_eq!(Stat { added: 0, removed: 0 }.added_ratio(), None);
    assert_eq!(Stat { added: 3, removed: 1 }.added_ratio(), Some(0.75));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test gradient` — expect FAIL, unresolved import `rv::gradient`.

- [ ] **Step 3: Implement the colour maths**

Oklab: sRGB → linear → LMS → cube root → Oklab, lerp, and back. Clamp on the way out; `f32` rounding can push a channel a hair past 255 and a wrapping cast would produce a black cell in the middle of the gradient.

`pivot()` takes the lighter of `ADDED` and `REMOVED`, raises its Oklab `L` by a fixed step capped just short of white, and drops chroma to near zero. Relative, not absolute — pure white flares on a dark terminal and disappears on a light one.

`column_colour` maps the column to a position and blends over a window of `min(4, width / 4)` columns centred on `ratio * width`. **Within the window it interpolates in two halves — `ADDED → pivot()` on the left of centre, `pivot() → REMOVED` on the right** — so no cell is ever a mix of green and red. Outside the window it returns the flat end colours, which is what keeps the proportion readable.

A degenerate width (0 or 1) and a ratio at either extreme must still produce a colour rather than dividing by zero; the tests above pin both.

- [ ] **Step 4: Compute the stats**

In `session::build` or `App::new`, diff every file once through `rv_core::diff::compute_with(old, new, path, false)` — the in-process `similar` path, **never the difftastic path**, because a subprocess per file before the first frame is seconds of startup on a large review. Count `Added` and `Removed` lines into a `Stat` per file. Report the total startup cost of this step in your report, measured on this repository.

- [ ] **Step 5: Paint the rows**

Sidebar rows draw each cell's background from `column_colour`, at whatever depth the terminal supports: truecolour when `COLORTERM` advertises it, the 256-colour cube otherwise, and a hard split with no blend at 16 colours. A directory row in the tree aggregates its subtree's stats. A file with no line changes renders neutral.

- [ ] **Step 6: Test it end to end**

```rust
#[test]
fn the_sidebar_tints_a_row_by_the_shape_of_its_change() {
    let app = mixed_workspace().app();       // one file mostly added, one mostly removed
    let frame = frame_at(&app, 100, 24);
    let added_row = sidebar_row_for(&frame, "added.rs");
    let removed_row = sidebar_row_for(&frame, "removed.rs");
    assert_ne!(bg_of(&frame, added_row, 2), bg_of(&frame, removed_row, 2),
        "the two files are tinted differently");
}

#[test]
fn a_pure_rename_is_left_neutral() {
    let app = rename_workspace().app();
    let frame = frame_at(&app, 100, 24);
    assert_eq!(bg_of(&frame, sidebar_row_for(&frame, "b.rs"), 2), None,
        "a gradient over zero changed lines would be inventing a ratio");
}

#[test]
fn the_colours_do_not_move_as_you_browse() {
    let mut app = mixed_workspace().app();
    let before = sidebar_backgrounds(&frame_at(&app, 100, 24));
    for _ in 0..3 { app.on_key(KeyCode::Char(']')).expect("next file"); }
    assert_eq!(sidebar_backgrounds(&frame_at(&app, 100, 24)), before,
        "stats are computed once at startup, never lazily recoloured");
}
```

- [ ] **Step 7: Add a property**

For arbitrary ratios and widths, every column produces a colour, the leftmost column is green whenever the ratio is above zero, the rightmost is red whenever it is below one, and no column's channels are ever outside `0..=255`. Prove it can fail against a vendored mutant that skips the clamp.

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat(rv): tint each file row by the shape of its change" && jj new
```

---

## Task 9: The powerline status bar

**Files:** Create `rv/src/statusbar.rs`, `rv/tests/statusbar.rs`; modify `rv/src/ui.rs`, `rv/tests/app_cases.rs`

**Consumes:** `Layout::bar` (Task 1, already at the bottom), `Stat` (Task 8).

**Produces:**

```rust
pub struct Segment { pub text: String, pub role: Role }
pub enum Role { Mode, Position, Scope, Comments, Hint }
pub fn segments(app: &App) -> Vec<Segment>;
pub fn render(segments: &[Segment], width: u16, ascii: bool) -> Line<'static>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_bar_names_the_mode_the_file_and_the_scope() {
    let app = workspace().app();
    let text = buffer_text_of_row(&frame_at(&app, 100, 24), 23);
    assert!(text.contains("BROWSE"), "the mode is visible: {text}");
    assert!(text.contains("a.rs"), "and the selected file");
    assert!(text.contains("trunk()"), "and what is being reviewed");
}

#[test]
fn the_mode_segment_changes_with_the_mode() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('c')).expect("c");
    let text = buffer_text_of_row(&frame_at(&app, 100, 24), 23);
    assert!(text.contains("COMMENT"), "typing is visibly a different mode: {text}");
}

#[test]
fn rv_ascii_replaces_the_powerline_glyphs() {
    let bar = render(&sample_segments(), 80, false);
    let plain = render(&sample_segments(), 80, true);
    assert!(line_text(&bar).contains('\u{e0b0}'), "arrows by default");
    assert!(!line_text(&plain).contains('\u{e0b0}'), "RV_ASCII=1 uses no patched glyphs");
    assert!(line_text(&plain).contains("BROWSE"), "and loses no information");
}

#[rstest]
#[case(20)]
#[case(40)]
#[case(200)]
fn the_bar_fills_its_width_exactly_and_never_overflows(#[case] width: u16) {
    let rendered = render(&sample_segments(), width, false);
    assert_eq!(line_width(&rendered), width as usize, "exactly {width} columns");
}

#[test]
fn a_bar_too_narrow_for_everything_drops_segments_rather_than_truncating_mid_word() {
    let rendered = line_text(&render(&sample_segments(), 24, false));
    assert!(rendered.contains("BROWSE"), "the mode survives first: {rendered}");
    assert!(!rendered.contains("trunk()"), "the scope is dropped whole, not cut in half");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Segments are dropped in priority order when the width is short — hint, then scope, then position, then comments; the mode is never dropped, because it is the one that tells you what the next keystroke does. Read `RV_ASCII` once at startup, not per frame.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): a powerline status bar along the bottom" && jj new
```

---

## Task 10: Focus colours the border

**Files:** Modify `rv/src/ui.rs`, `rv/src/gradient.rs`, `rv/tests/app.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_focused_pane_border_is_the_accent_and_the_other_is_not() {
    let mut app = workspace().app();
    let frame = frame_at(&app, 100, 24);
    let diff_border = border_colour(&frame, diff_pane_corner(&app, 100, 24));
    let sidebar_border = border_colour(&frame, sidebar_corner(&app, 100, 24));
    assert_eq!(diff_border, Some(ACCENT), "the diff has focus at launch");
    assert_ne!(sidebar_border, Some(ACCENT), "the sidebar does not");

    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let frame = frame_at(&app, 100, 24);
    assert_eq!(border_colour(&frame, sidebar_corner(&app, 100, 24)), Some(ACCENT));
    assert_ne!(border_colour(&frame, diff_pane_corner(&app, 100, 24)), Some(ACCENT));
}

#[test]
fn the_accent_is_none_of_the_colours_that_already_mean_something() {
    // green is an addition, red a removal, blue a comment, orange an alert.
    for taken in [gradient::ADDED, gradient::REMOVED, ui::COMMENT_BLUE, ui::ALERT_ORANGE] {
        assert_ne!(ACCENT, taken, "the focus accent must be unambiguous");
    }
}

#[test]
fn the_title_marker_survives_the_colour() {
    // Redundant on purpose: a 16-colour terminal, or a reader who does not
    // separate magenta from red, still needs to know where the keys go.
    let app = workspace().app();
    assert!(buffer_text(&frame_at(&app, 100, 24)).contains('▸'));
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Borders become rounded. The accent is magenta, defined once beside the other palette constants in `gradient.rs` so the whole palette is declared in one place.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): colour the focused pane's border" && jj new
```

---

## Task 11: Alerts that float and fade

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**Produces:**

```rust
pub struct Alert { pub message: String, pub raised: Instant }
impl App {
    pub fn alert(&mut self, message: impl Into<String>, now: Instant);
    pub fn expire_alerts(&mut self, now: Instant);
    pub fn alerts(&self) -> &[Alert];
    pub fn next_deadline(&self, now: Instant) -> Option<Duration>;   // what the event loop waits for
}
```

**Nothing inside `App` calls `Instant::now()`.** The event loop supplies the time; a test supplies whatever it likes. Every state machine in this project has stayed testable by refusing ambient input, and a clock is ambient input.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_alert_appears_then_leaves_on_its_own() {
    let mut app = workspace().app();
    let t0 = Instant::now();
    app.alert("src/old.rs is no longer in this range", t0);
    assert_eq!(app.alerts().len(), 1);
    assert!(buffer_text(&frame_at(&app, 100, 24)).contains("no longer in this range"));

    app.expire_alerts(t0 + Duration::from_secs(2));
    assert_eq!(app.alerts().len(), 1, "still up at two seconds");

    app.expire_alerts(t0 + Duration::from_secs(6));
    assert!(app.alerts().is_empty(), "gone by six");
    assert!(!buffer_text(&frame_at(&app, 100, 24)).contains("no longer in this range"));
}

#[test]
fn the_toast_takes_no_key_and_steals_no_focus() {
    let mut app = workspace().app();
    let focus = app.focus();
    let line = app.line_index();
    app.alert("something went wrong", Instant::now());
    app.on_key(KeyCode::Char('j')).expect("j");
    assert_eq!(app.focus(), focus, "focus is untouched");
    assert_eq!(app.line_index(), line + 1, "and j still moved the line");
    assert_eq!(app.alerts().len(), 1, "the key did not dismiss it either");
}

#[test]
fn the_border_dims_as_the_deadline_approaches() {
    let mut app = workspace().app();
    let t0 = Instant::now();
    app.alert("careful", t0);
    let bright = toast_border_colour(&frame_at_time(&app, 100, 24, t0));
    let faded = toast_border_colour(&frame_at_time(&app, 100, 24, t0 + Duration::from_millis(4600)));
    assert_ne!(bright, faded, "the toast fades rather than vanishing abruptly");
    assert!(luma(faded) < luma(bright), "and it fades down, not up");
}

#[test]
fn the_event_loop_is_told_when_to_wake_up() {
    let mut app = workspace().app();
    let t0 = Instant::now();
    assert_eq!(app.next_deadline(t0), None, "an idle rv waits for a key, forever");
    app.alert("careful", t0);
    let wait = app.next_deadline(t0).expect("a live alert gives the loop a timeout");
    assert!(wait <= Duration::from_secs(5), "and the timeout is bounded by the deadline");
}

#[test]
fn several_alerts_stack_without_overlapping() {
    let mut app = workspace().app();
    let t0 = Instant::now();
    app.alert("first", t0);
    app.alert("second", t0);
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(text.contains("first") && text.contains("second"));
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`ui::draw` needs the current time to pick the fade step; give it a `now: Instant` parameter rather than calling the clock inside the renderer, for the same reason. The fade is an Oklab lightness ramp using Task 8's `oklab_mix`, in four steps over the final second; at 16 colours the toast disappears at its deadline without fading, because a fade that degrades into a flicker is worse than none.

Then raise alerts where the code currently swallows or merely statuses a real failure — an unreadable blob, a jump whose anchored file has left the range, a stale export. Ordinary confirmations stay in the status bar; the toast is for what went wrong.

- [ ] **Step 5: Change the event loop**

`event::read()` becomes `event::poll(timeout)` where the timeout is `app.next_deadline(Instant::now())`, and `None` means block as before. Call `expire_alerts` each pass. Verify by hand on a pty that a toast raised with no further input still disappears — an alert that needs a keystroke to expire is the bug this step exists to prevent, and no unit test can see it.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): alerts that float at the top and fade out" && jj new
```

---

## Task 11a: The syntax colours come from the terminal's theme (defect)

**Files:** Modify `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**This is a defect report from a user running the tool: comments render too white.**

The cause is that code text is being painted in colours rv chose rather than colours the terminal chose. The 16 indexed ANSI colours are a pass-through to the user's own scheme — emit index 4 and the terminal substitutes whatever *its* theme calls blue — while any `Color::Rgb` dictates an exact value and ignores the scheme entirely. Comments in particular must map to **index 8, bright black**, which every scheme defines as its muted tone against its own background; `Color::White` (index 7) or any RGB grey is wrong on every theme at once.

- [ ] **Step 1: Audit and write the failing tests**

Read `ui.rs`'s capture-to-style mapping and report what each capture currently produces. Then:

```rust
#[test]
fn code_is_painted_only_in_indexed_colours() {
    // Indexed colours are the user's theme. An Rgb value overrides it, which is
    // how a tool ends up needing a theme option it should never have needed.
    let app = rust_workspace().app();
    let frame = frame_at(&app, 100, 30);
    for (column, row) in diff_pane_cells(&frame) {
        let fg = frame[(column, row)].fg;
        assert!(
            !matches!(fg, Color::Rgb(..)),
            "code cell ({column},{row}) dictates an exact colour instead of using the terminal's"
        );
    }
}

#[test]
fn a_comment_uses_the_terminals_muted_tone() {
    let app = rust_workspace().app();          // fixture must contain a `// comment`
    let frame = frame_at(&app, 100, 30);
    assert_eq!(
        colour_of_first_comment(&frame),
        Color::DarkGray,
        "comments are index 8, the tone every scheme defines for exactly this"
    );
}

#[rstest]
#[case(Capture::Keyword, Color::Magenta)]
#[case(Capture::Function, Color::Blue)]
#[case(Capture::Type, Color::Cyan)]
#[case(Capture::String, Color::Green)]
#[case(Capture::Comment, Color::DarkGray)]
fn every_capture_maps_to_an_indexed_colour(#[case] capture: Capture, #[case] expected: Color) {
    assert_eq!(ui::capture_colour(capture), expected);
}
```

- [ ] **Step 2: Run to verify they fail.** Paste what the comment colour actually is today.

- [ ] **Step 3: Fix the mapping** to the table in the spec's §6, and leave punctuation, variables and anything unrecognised **unstyled** so they inherit the terminal's own foreground rather than a colour rv guessed.

- [ ] **Step 4: Run to verify they pass**

- [ ] **Step 5: Confirm the exception is bounded.** The sidebar's change gradient, the alert orange and the focus magenta may use RGB — they are decorations rv owns and the gradient cannot exist in 16 colours. Assert that the diff pane's *text* contains no `Color::Rgb` while allowing the sidebar's to, so the boundary is enforced rather than remembered.

- [ ] **Step 6: Commit**

```bash
jj describe -m "fix(rv): let the terminal's own theme colour the code" && jj new
```

---

## Task 11b: A long comment can actually be read (defect)

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/rows.rs`, `rv/tests/app.rs`, `rv/tests/rows.rs`, `rv/tests/app_cases.rs`

**This is a defect in shipped behaviour, not a feature.** Do it before the remaining feature tasks.

**The defect:** a comment taller than the diff pane cannot be read — not "is awkward", cannot. The pane anchors its window on the row of the selected *diff line* (`rv/src/ui.rs`, `plan.row_of_line(line)`), and `j` moves the selection to the next diff line. A comment box sits between two diff rows, so with a box taller than the pane you see its top from the line above and its bottom from the line below, and **no cursor position anywhere puts the middle rows in the window.** Scrolling looks like it "jumps through" the comment because it is not scrolling the comment at all — it is stepping over it.

**The fix:** `j`/`k` move a cursor over the plan's **rows**, not over diff lines. A box is rows, so the cursor can walk into it and every row becomes reachable by construction. The selection everything else depends on — `c`, `d`, `comments_for_line`, the anchor a comment saves against — becomes the diff line that **owns** the row under the cursor: a diff row owns itself, a box row is owned by the line its box hangs from.

**The row cursor is the state and the line index is derived, not the reverse.** Two cursors kept in step is precisely what caused this: the window's anchor and the user's cursor were different things.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_row_of_a_tall_comment_can_be_brought_on_screen() {
    // The defect, stated as a test. A box taller than the pane must not have
    // rows that no cursor position can show.
    let mut app = workspace().app();
    write_comment(&mut app, &"a very long finding. ".repeat(40));
    let height = 10;

    let mut seen = std::collections::HashSet::new();
    for _ in 0..80 {
        for row in visible_row_indices(&app, 100, height) { seen.insert(row); }
        app.on_key(KeyCode::Char('j')).expect("j");
    }
    let total = row_count(&app, 100);
    let missed: Vec<usize> = (0..total).filter(|r| !seen.contains(r)).collect();
    assert!(missed.is_empty(), "rows unreachable at any cursor position: {missed:?}");
}

#[test]
fn j_walks_into_a_comment_box_rather_than_over_it() {
    let mut app = workspace().app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    app.on_key(KeyCode::Char('j')).expect("j");
    assert_eq!(app.line_index(), line, "the cursor is still inside this line's comment");
    assert!(app.cursor_row() > 0, "and it has moved down a row");
}

#[test]
fn commenting_from_inside_a_box_targets_the_line_the_box_belongs_to() {
    let mut app = workspace().app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    app.on_key(KeyCode::Char('j')).expect("step into the box");
    write_comment(&mut app, "a second finding");
    assert_eq!(app.comments_for_line(line).len(), 2, "both comments are on the same line");
}

#[test]
fn stepping_past_the_last_row_of_a_box_lands_on_the_next_diff_line() {
    let mut app = workspace().app();
    write_comment(&mut app, "short");
    let line = app.line_index();
    for _ in 0..8 { app.on_key(KeyCode::Char('j')).expect("j"); }
    assert!(app.line_index() > line, "the cursor eventually leaves the box");
}
```

Write `visible_row_indices` and `row_count` as helpers that build the same plan and window `ui::body` does; if that is awkward from a test, expose exactly what is needed rather than duplicating the arithmetic.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rv --test app` — expect the reachability test to fail with a non-empty list of unreachable rows. **Paste that list**; it is the defect, measured.

- [ ] **Step 3: Implement the row cursor**

`App` holds `cursor_row: usize` over the current file's plan. `line_index()` derives from it. Clamp on file change, on collapse or expand, and after a delete — anything that rebuilds the plan can shorten it.

- [ ] **Step 4: Run to verify they pass**

- [ ] **Step 5: Add the reachability property**

```rust
proptest! {
    #[test]
    fn every_row_is_reachable(rows in 1usize..60, height in 1usize..20) {
        let mut seen = std::collections::HashSet::new();
        for cursor in 0..rows {
            for row in window(rows, cursor, height) { seen.insert(row); }
        }
        prop_assert_eq!(seen.len(), rows, "some row is in no window at any cursor");
    }
}
```

This is the assertion the defect would have failed, and no example test would reliably have caught it — it only bites when a box is taller than the pane, which no fixture happened to build. Prove it can fail by vendoring a copy and restoring line-anchored windowing.

- [ ] **Step 6: Commit**

```bash
jj describe -m "fix(rv): let the cursor walk into a comment so a long one can be read" && jj new
```

---

## Task 11c: Counts and sorting in the sidebar

**Files:** Modify `rv/src/tree.rs`, `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/tree.rs`, `rv/tests/app.rs`

**Consumes:** `Stat` and the palette from `gradient` (Task 8), `Node`/`NodeKind` (Task 6).

**Produces:**

```rust
pub enum Sort { Natural, Added, Removed }
impl App { pub fn sort(&self) -> Sort; }
pub fn abbreviate(n: u32) -> String;   // 42 -> "42", 1234 -> "1.2k"
// tree::build and build_grouped gain a `sort: Sort` parameter and a stats lookup
```

The counts are the `Stat` the gradient already computes at startup — this task renders and orders them, it does not compute anything new.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_row_shows_what_it_costs_to_review() {
    let app = mixed_workspace().app();       // added.rs +40 −0, removed.rs +0 −25
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(frame.contains("+40"), "additions are shown: {frame}");
    assert!(frame.contains("25"), "and removals");
}

#[test]
fn a_directory_shows_its_subtrees_total() {
    let nodes = build(&["src/a.rs", "src/b.rs"], &HashSet::new(), true, Sort::Natural, &stats_of([("src/a.rs", 10, 2), ("src/b.rs", 5, 3)]));
    let dir = nodes.iter().find(|n| n.label == "src").expect("the directory row");
    assert_eq!(dir.stat, Stat { added: 15, removed: 5 }, "a collapsed row that hides its weight is a row you must expand to judge");
}

#[rstest]
#[case(42, "42")]
#[case(999, "999")]
#[case(1234, "1.2k")]
#[case(45678, "46k")]
fn large_counts_abbreviate(#[case] n: u32, #[case] expected: &str) {
    assert_eq!(abbreviate(n), expected);
}

#[test]
fn o_cycles_the_order_and_the_title_says_which() {
    let mut app = mixed_workspace().app();
    assert!(sidebar_title(&frame_at(&app, 100, 24)).contains("natural"));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(sidebar_title(&frame_at(&app, 100, 24)).contains("added"));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(sidebar_title(&frame_at(&app, 100, 24)).contains("removed"));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(sidebar_title(&frame_at(&app, 100, 24)).contains("natural"), "it cycles");
}

#[test]
fn sorting_by_additions_puts_the_biggest_first() {
    let stats = stats_of([("small.rs", 3, 0), ("huge.rs", 300, 0), ("mid.rs", 30, 0)]);
    let order: Vec<&str> = build(&["small.rs", "huge.rs", "mid.rs"], &HashSet::new(), false, Sort::Added, &stats)
        .iter().map(|n| n.label.as_str()).collect();
    assert_eq!(order, ["huge.rs", "mid.rs", "small.rs"]);
}

#[test]
fn sorting_does_not_flatten_the_tree() {
    // A reviewer asked for both; they compose. Siblings sort against each other
    // and directories keep their children.
    let stats = stats_of([("src/small.rs", 1, 0), ("src/huge.rs", 99, 0), ("top.rs", 50, 0)]);
    let nodes = build(&["src/small.rs", "src/huge.rs", "top.rs"], &HashSet::new(), true, Sort::Added, &stats);
    assert!(nodes.iter().any(|n| matches!(n.kind, NodeKind::Dir { .. })), "still a tree");
    let files_under_src: Vec<&str> = nodes.iter().filter(|n| n.depth > 0).map(|n| n.label.as_str()).collect();
    assert_eq!(files_under_src, ["huge.rs", "small.rs"], "siblings sorted, nesting intact");
}

#[test]
fn a_directory_sorts_among_its_siblings_by_its_aggregate() {
    let stats = stats_of([("a/x.rs", 1, 0), ("b/y.rs", 5, 0), ("b/z.rs", 5, 0)]);
    let nodes = build(&["a/x.rs", "b/y.rs", "b/z.rs"], &HashSet::new(), true, Sort::Added, &stats);
    let dirs: Vec<&str> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Dir { .. })).map(|n| n.label.as_str()).collect();
    assert_eq!(dirs, ["b", "a"], "b totals 10 and outranks a's 1");
}

#[test]
fn a_narrow_sidebar_drops_the_counts_before_the_path() {
    let mut app = mixed_workspace().app();
    for _ in 0..30 { app.on_key(KeyCode::Char('<')).expect("<"); }   // squeeze the sidebar
    let frame = buffer_text(&frame_at(&app, 60, 24));
    assert!(frame.contains("added.rs") || frame.contains("added"), "the path survives");
    // the gradient still carries the ratio, so nothing is truly lost
}

#[test]
fn the_order_never_reaches_disk() {
    let workspace = mixed_workspace();
    let mut app = workspace.app();
    let tree = workspace_tree(&workspace);
    app.on_key(KeyCode::Char('o')).expect("o");
    assert_eq!(workspace_tree(&workspace), tree, "a view preference, not review state");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`natural` means "the order the thing already has" — path order for files, stack order for commits — which is why it is one mode rather than two. Add `o` to `BINDINGS` with its contexts, never beside it.

- [ ] **Step 5: Add the conservation property** — under every `Sort` and both groupings, the set of files in the tree is unchanged; only their order moves. A sort that loses a file is worse than no sort, and this is the same property the tree already carries for grouping.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): show what each file costs to review, and sort by it" && jj new
```

---

## Task 11d: Responsive layout and a collapsible sidebar

**Files:** Modify `rv/src/layout.rs`, `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/layout.rs`, `rv/tests/app.rs`

**Consumes:** `Layout`, `hit`, `Target` (Task 1); `Split` (Task 2); the mouse routing (Task 7).

**Produces:** `Layout::sidebar_toggle: Rect`, `Target::SidebarToggle`, `App::sidebar_collapsed() -> bool`, and `z` bound in `BINDINGS`.

**Why:** a reviewer on a phone over ssh has forty columns. Two panes do not fit in forty columns at any ratio, and "read this diff on my phone" is exactly when someone reaches for a terminal instead of a browser.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn z_collapses_the_sidebar_and_gives_the_diff_the_width() {
    let mut app = workspace().app();
    let wide = diff_pane_width(&frame_at(&app, 120, 30));
    app.on_key(KeyCode::Char('z')).expect("z");
    assert!(app.sidebar_collapsed());
    assert!(diff_pane_width(&frame_at(&app, 120, 30)) > wide, "the diff took the space");
    app.on_key(KeyCode::Char('z')).expect("z again");
    assert!(!app.sidebar_collapsed());
}

#[test]
fn clicking_the_chevron_collapses_it_too() {
    // A control that exists only for the mouse is a control an ssh user without
    // mouse reporting does not have — and that user is who this is for.
    let mut app = workspace().app();
    let l = layout_of(&app, 120, 30);
    let cell = (l.sidebar_toggle.x, l.sidebar_toggle.y);
    assert_eq!(hit(&l, cell.0, cell.1), Some(Target::SidebarToggle));
    app.on_mouse(click(cell.0, cell.1)).expect("click the chevron");
    assert!(app.sidebar_collapsed());
}

#[rstest]
#[case(40, 20)]
#[case(50, 24)]
#[case(60, 30)]
fn a_narrow_terminal_shows_one_pane_and_still_renders(#[case] width: u16, #[case] height: u16) {
    let app = workspace().app();
    let l = layout(Rect::new(0, 0, width, height), Split::new(30), Chrome { bar_rows: 1, help_open: false, toast: false });
    assert!(l.sidebar.width == 0 || l.diff.width == 0, "one pane at a time under 80 columns");
    let _ = frame_at(&app, width, height);   // must not panic at phone size
}

#[test]
fn widening_restores_the_ratio_the_reviewer_chose() {
    // The breakpoint overrides the split; it must not overwrite it, or the
    // reviewer sets their layout twice.
    let mut app = workspace().app();
    for _ in 0..5 { app.on_key(KeyCode::Char('>')).expect(">"); }
    let chosen = app.split().ratio();
    let _ = frame_at(&app, 40, 20);          // render narrow, forcing one pane
    let _ = frame_at(&app, 120, 30);         // and back to wide
    assert_eq!(app.split().ratio(), chosen, "the preference survived the breakpoint");
}

#[test]
fn the_chevron_shows_which_way_it_will_go() {
    let mut app = workspace().app();
    assert!(buffer_text(&frame_at(&app, 120, 30)).contains('▾'), "expanded points down");
    app.on_key(KeyCode::Char('z')).expect("z");
    assert!(buffer_text(&frame_at(&app, 120, 30)).contains('▸'), "collapsed points right");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

The breakpoint lives in `layout()`, which is where every rectangle is already decided; `App` holds only the collapsed flag and the remembered ratio. Add `z` to `BINDINGS` with its contexts, never beside it.

- [ ] **Step 4b: Fold a folder with `Enter` or `Space`**

```rust
#[rstest]
#[case(KeyCode::Enter)]
#[case(KeyCode::Char(' '))]
fn a_directory_row_folds_with_enter_or_space(#[case] key: KeyCode) {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('t')).expect("tree view");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    select_first_directory_row(&mut app);
    let rows = sidebar_row_count(&frame_at(&app, 100, 30));

    app.on_key(key).expect("fold");
    assert!(sidebar_row_count(&frame_at(&app, 100, 30)) < rows, "its children are hidden");
    app.on_key(key).expect("unfold");
    assert_eq!(sidebar_row_count(&frame_at(&app, 100, 30)), rows, "and back");
}

#[rstest]
#[case(KeyCode::Enter)]
#[case(KeyCode::Char(' '))]
fn on_a_file_row_both_keys_still_move_to_the_diff(#[case] key: KeyCode) {
    // A file is a thing to look at; a directory is a thing to open.
    let mut app = workspace().app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(key).expect("open");
    assert_eq!(app.focus(), Focus::Diff);
}
```

Add `Space` to `BINDINGS` beside `Enter`, with its contexts, and present it as `enter / space` wherever the keymap is shown. `s` keeps working everywhere — it stays the general verb; these are the keys a reviewer will reach for first in a tree.

- [ ] **Step 5: Sweep phone-sized frames** — every state (sidebar focused, diff focused, comment open, help open, toast up) at 40x20, 50x24 and 60x30, asserting no panic and no overlap. Small terminals are where layout arithmetic breaks, and a 1x1 sweep does not reach this code because the panes get zero height before the interesting paths run.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): collapse the sidebar, and fit on a phone" && jj new
```

---

## Task 11e: Resolve and abandon a comment

**Files:** Modify `rv-core/src/store.rs`, `rv-core/src/markdown.rs`, `rv-core/tests/store.rs`, `rv-core/tests/markdown.rs`, `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`, `README.md`

**Spec:** `docs/superpowers/specs/2026-08-17-rv-storage-model-design.md` §3.

Deleting, resolving and abandoning are **three different acts**. Deleting says the comment should never have existed and removes the record. Resolving says it was **addressed**. Abandoning says it was **dropped without being addressed** — and a reviewer returning to a stack needs to tell those two apart, because one is work that happened and the other is work that was decided against.

**Produces:** `CommentState::{Open, Resolved, Abandoned, Outdated}` replacing whatever is in the tree, a `settled_by` field recording `user` or `agent`, and `r` / `a` bound in `BINDINGS`.

- [ ] **Step 1: Write the failing tests**

```rust
#[rstest]
#[case(KeyCode::Char('r'), CommentState::Resolved)]
#[case(KeyCode::Char('a'), CommentState::Abandoned)]
fn a_comment_can_be_settled_without_being_deleted(#[case] key: KeyCode, #[case] expected: CommentState) {
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(key).expect("settle it");

    let stored = fresh_store(&workspace).comments().expect("read");
    assert_eq!(stored.len(), 1, "settling is not deleting — the record survives");
    assert_eq!(stored[0].state, expected);
    assert_eq!(stored[0].settled_by.as_deref(), Some("user"));
}

#[rstest]
#[case(KeyCode::Char('r'))]
#[case(KeyCode::Char('a'))]
fn settling_asks_no_question_because_it_can_be_undone(#[case] key: KeyCode) {
    // Confirm what cannot be undone; do not interrupt what can.
    let mut app = workspace().app();
    write_comment(&mut app, "needs a doc");
    app.on_key(key).expect("settle");
    assert_eq!(app.mode(), Mode::Browse, "no confirmation prompt");
    app.on_key(key).expect("again");
    assert_eq!(app.comments()[0].state, CommentState::Open, "the same key returns it to open");
}

#[test]
fn deleting_still_asks_because_it_cannot_be_undone() {
    let mut app = workspace().app();
    write_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Char('d')).expect("d");
    assert!(matches!(app.mode(), Mode::ConfirmDelete { .. }));
}

#[test]
fn a_settled_comment_renders_collapsed_and_says_which_it_is() {
    let mut app = workspace().app();
    write_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Char('r')).expect("resolve");
    let text = buffer_text(&frame_at(&app, 100, 30));
    assert!(text.contains("resolved"), "the box says what happened to it: {text}");
    app.on_key(KeyCode::Char('a')).expect("abandon it instead");
    assert!(buffer_text(&frame_at(&app, 100, 30)).contains("abandoned"));
}

#[test]
fn the_export_separates_what_was_fixed_from_what_was_dropped() {
    // Counting them together would misreport what the review concluded.
    let session = sample_session();
    let comments = vec![
        comment_in(CommentState::Open, "still open"),
        comment_in(CommentState::Resolved, "was fixed"),
        comment_in(CommentState::Abandoned, "was dropped"),
    ];
    let doc = markdown::render(&session, &comments);
    assert!(doc.contains("## Open (1)"));
    assert!(doc.contains("## Resolved (1)"));
    assert!(doc.contains("## Abandoned (1)"));
}

#[test]
fn the_sidebar_counts_only_what_is_still_open() {
    let mut app = workspace().app();
    write_comment(&mut app, "one");
    write_comment(&mut app, "two");
    app.on_key(KeyCode::Char('r')).expect("resolve one");
    assert!(bar_text(&frame_at(&app, 110, 30)).contains("1 open"), "settled comments leave the count");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`r` and `a` act on the same target `d` does: the selected comment in the stack or the browser, the line's newest from the diff. Add both to `BINDINGS` with their contexts. An agent writing a reply does not settle anything — `settled_by` distinguishes `user` from `agent` precisely so an agent-settled comment renders distinctly rather than being indistinguishable from a human's decision.

- [ ] **Step 4b: `e` exports the markdown without leaving the reviewer**

```rust
#[test]
fn e_writes_the_export_and_says_where_it_went() {
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let path = workspace.root().join(".review/REVIEW-FEEDBACK.md");
    let _ = std::fs::remove_file(&path);

    app.on_key(KeyCode::Char('e')).expect("export");

    let doc = std::fs::read_to_string(&path).expect("the export exists");
    assert!(doc.contains("needs a doc"), "and holds the review");
    assert!(app.status().contains("REVIEW-FEEDBACK.md"), "the status names the file: {}", app.status());
}

#[test]
fn exporting_ingests_replies_first_so_nothing_a_model_wrote_is_lost() {
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Char('e')).expect("export");
    append_reply(&workspace, "Fixed in the next change.");

    app.on_key(KeyCode::Char('e')).expect("export again");

    assert_eq!(
        fresh_store(&workspace).comments().expect("read")[0].reply.as_deref(),
        Some("Fixed in the next change."),
        "a reply written into the export survives the next export"
    );
}

#[test]
fn the_bar_stops_calling_the_export_stale_once_it_is_written() {
    let mut app = workspace().app();
    write_comment(&mut app, "needs a doc");
    assert!(bar_text(&frame_at(&app, 110, 30)).contains("stale"), "a save makes the export stale");
    app.on_key(KeyCode::Char('e')).expect("export");
    assert!(!bar_text(&frame_at(&app, 110, 30)).contains("stale"));
}
```

`e` goes through the same path `rv render` does — **ingest replies, then write** — so the two cannot diverge. Without this a reviewer has to quit to produce the file the whole LLM loop depends on, which is a strange thing to make someone leave the tool for. Add `e` to `BINDINGS` with its contexts.

- [ ] **Step 5: Migrate honestly.** A `.review/` written before this change may carry the older state vocabulary. Read it, map it to the nearest new state, and say in the report which mapping you chose — never silently drop a comment whose state you do not recognise; an unknown state becomes `Open`, because showing a reviewer something they must look at again is the safe direction.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(rv): resolve or abandon a comment, which is not the same as deleting it" && jj new
```

---

## Task 12: The keymap follows what you are looking at

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/statusbar.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**Consumes:** `BINDINGS` (Task 3), the status bar (Task 9), the tree's `NodeKind` (Task 6).

**Produces:**

```rust
pub enum Context { Files, Commit, Comments, Diff, Comment, Typing, Confirm }
impl App { pub fn context(&self) -> Context; }
// Binding gains:
pub struct Binding { pub keys: &'static str, pub group: Group, pub what: &'static str, pub contexts: &'static [Context] }
```

- [ ] **Step 1: Write the failing tests**

```rust
#[rstest]
#[case::diff(&[], Context::Diff)]
#[case::files(&[KeyCode::Left], Context::Files)]
#[case::comments(&[KeyCode::Tab, KeyCode::Left], Context::Comments)]
fn the_context_follows_the_cursor(#[case] keys: &[KeyCode], #[case] expected: Context) {
    let mut app = workspace().app();
    for key in keys { app.on_key(*key).expect("key"); }
    assert_eq!(app.context(), expected);
}

#[test]
fn stepping_into_a_comment_stack_is_its_own_context() {
    let mut app = workspace().app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.context(), Context::Comment);
}

#[test]
fn typing_and_confirming_are_contexts_too() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('c')).expect("c");
    assert_eq!(app.context(), Context::Typing);
    app.on_key(KeyCode::Esc).expect("esc");
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('d')).expect("d");
    assert_eq!(app.context(), Context::Confirm);
}

#[test]
fn the_status_bar_names_the_context_not_just_the_mode() {
    let mut app = workspace().app();
    assert!(bar_text(&frame_at(&app, 100, 24)).contains("DIFF"));
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert!(bar_text(&frame_at(&app, 100, 24)).contains("FILES"));
}

#[test]
fn the_popup_leads_with_the_context_you_are_in() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 100, 30));
    let files_at = frame.find("files").expect("the files group is titled");
    let diff_at = frame.find("diff").expect("the diff group is still listed");
    assert!(files_at < diff_at, "the context you are in comes first");
}

#[test]
fn the_popup_still_lists_everything_it_does_not_hide_by_context() {
    // A keymap that hides is a keymap that teaches less: a reviewer wants to
    // know what `d` does BEFORE they move onto a comment, not after.
    let mut app = workspace().app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 100, 30));
    for binding in rv::app::BINDINGS {
        assert!(frame.contains(binding.keys), "{} is listed even here", binding.keys);
    }
}

#[test]
fn a_binding_inert_in_this_context_is_dimmed_and_does_nothing() {
    // One table drives both: a key shown as active cannot be inert, and a key
    // that is inert cannot be shown as active.
    let workspace = workspace();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Left).expect("focus the file list");
    let before = workspace_tree(&workspace);

    app.on_key(KeyCode::Char('d')).expect("d");
    assert_eq!(app.mode(), Mode::Browse, "d is inert on a file row");
    assert_eq!(workspace_tree(&workspace), before, "and wrote nothing");

    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = frame_at(&app, 100, 30);
    assert!(is_dim(&frame, cell_of_binding(&frame, "d")), "and the popup says so");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

`context()` is derived from the mode, the focus, the sidebar tab and the kind of row under the cursor — **never stored**, because a stored copy would need invalidating on every one of those changes and nothing would be watching. `Context::Commit` is reachable once the commits view lands; until then derive it from the node kind under the sidebar cursor so it is correct the moment that view exists.

Give `Binding` its `contexts` field and have **both** `on_key_browse` and the popup consult it. That is what makes the dimming honest rather than decorative.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(rv): the keymap and the status bar follow what you are looking at" && jj new
```

---

## Task 13: Document it

**Files:** Modify `README.md`, `rv/tests/app_cases.rs`

- [ ] Document `<`/`>`, `?`, `t`, and the mouse gestures; state that **Shift-drag** selects text natively, because mouse reporting is on and a reader will otherwise think copy is gone. Note that syntax highlighting covers the grammars rv ships and that other files render plain. Say what the sidebar's colour bar measures — **lines of text, not semantic change** — so a reader is not surprised when a reindentation shows a gradient while the pane calls it no semantic change. Extend the existing README-versus-code test to cover every new binding.
- [ ] Run `cargo test --workspace`; expect green, clippy and fmt clean.
- [ ] Commit.

---

## Self-review

Spec coverage: §3 → Task 1; §4 → Tasks 1-2 and the drag in 7; §5 → Task 3; §6 → Tasks 4-5; §7's tree and commit nodes → Task 6, §7's change gradient → Task 8; §8 → Task 7; §9's status bar → Task 9, its focus border → Task 10, its alerts → Task 11; §5's contextual keymap and §9's context-aware mode indicator → Task 12; §10's test list is distributed across the tasks that own each behaviour. §10's non-goals are respected — no config file, no themes, no rv-side selection, no destructive gesture, no horizontal scrolling, no third pane.

Ordering: Task 1 has no dependencies and everything visual depends on it; Task 4 is independent of 1-3 and can run alongside; Task 5 needs 1 and 4; Task 6 needs 1; Task 7 needs 1, 2 and 6; Task 8 needs 6, because a directory row aggregates its subtree; Task 12 needs 3, 6 and 9, since it extends the binding table with contexts, derives one of them from the tree's node kind, and renames what the bar's first segment shows; Task 13 needs all. Tasks 2, 3, 5, 6, 7 and 8 all touch `rv/src/app.rs` and must not run concurrently with each other.

Type consistency: `Split` and `Layout` are defined in Task 1 and consumed by 2, 5, 6, 7 and 8; `Capture`/`Highlights` in Task 4 and consumed only by Task 5; `Node` in Task 6 and consumed by Task 8's directory aggregation; `Target` in Task 1 and consumed only by Task 7; `Stat`/`Rgb` in Task 8 and consumed only by its own renderer.

One thing to watch when implementing Task 8 alongside Task 5: both paint the same cells with backgrounds. The diff pane's tint marks *added or removed*, and the sidebar's gradient marks *how much was added versus removed*. They never appear in the same pane, but they should read as one palette — use the same green and the same red for both, defined once in `gradient.rs` and imported by the diff renderer, rather than two near-identical pairs that drift apart.
