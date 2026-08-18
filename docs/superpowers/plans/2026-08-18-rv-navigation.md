# rv Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Jump around code by its structure — step between symbols across every changed file, search them by name, and switch between reviewing the whole bookmark and reviewing one commit at a time.

**Architecture:** `rv-core` extracts symbols as plain data using the tree-sitter grammars the highlighter already ships; `rv` holds the in-scope index, the picker and the stepping. Scope is the current view and nothing else, so the rule a reviewer has to learn is one sentence.

**Tech Stack:** Rust 2024, tree-sitter 0.26 + tree-sitter-tags 0.26, nucleo 0.5, ratatui 0.30, rstest + proptest (dev).

**Spec:** `docs/superpowers/specs/2026-08-17-rv-navigation-design.md`

## Global Constraints

- **`rv-core` MUST NOT depend on `ratatui`, `crossterm`, or `tui-textarea`.** Enforced by `rv-core/tests/constraints.rs`, which fails the build. A `Symbol` is plain data; anything terminal-shaped lives in `rv`.
- **`jj_lib` appears only in `rv-core/src/vcs.rs`.** Same enforcement.
- **Never read the user's jj config.** Same enforcement.
- **`App::on_key` stays terminal-free.** It takes a key value and returns an `Action`. Every wave of this project has depended on that property to test the state machine without a pty.
- **One side rule.** Anything asking which side a diff line belongs to calls `rv::app::anchored_side(kind)`. This project shipped one bug where the pane and the anchor disagreed and nearly shipped a second where a jump ignored the side.
- **One layout.** No file computes a `Rect` outside `rv::layout::layout()`. The picker and any overlay get their rect from there.
- **One binding table.** Every key dispatched is in `BINDINGS`, with its `contexts`, so the `?` popup cannot fall out of step. New keys here (`/`, `n`, `N`, `1`, `2`) are added to it, not beside it.
- **The arrows are the binding; `hjkl` are aliases.** Present arrows first everywhere a key is shown.
- **Every new preference is session-only.** Nothing here reaches `.review/`.
- **Never mutate a file in this repository to show a test can fail.** Vendor a copy into the scratch directory and mutate that. A coordinator once read a source file mid-experiment, mistook a mutation for a shipped bug, and reported it up the chain before checking it against git.
- Commits use jj: `jj describe -m "…" && jj new`, with the repo's two trailer lines.

### What already exists

```rust
// rv-core/src/highlight.rs — tree-sitter is already a dependency, Rust grammar shipped
pub struct Highlights { /* … */ }
impl Highlights { pub fn of(source: &[u8], path: &str) -> Highlights; pub fn language(&self) -> Option<&'static str>; pub fn line(&self, line: u32) -> &[Span]; }

// rv/src/tree.rs — the sidebar model, commits included
pub enum NodeKind { Commit { change_id: String, collapsed: bool }, Dir { collapsed: bool }, File { index: usize } }
pub fn build(paths: &[&str], collapsed: &HashSet<String>, tree: bool) -> Vec<Node>;
pub fn build_grouped(groups: &[Group<'_>], collapsed: &HashSet<String>, tree: bool) -> Vec<Node>;

// rv-core/src/vcs.rs
impl Repository {
    pub fn stack(&self, base: Option<&str>, head: Option<&str>) -> Result<Vec<ChangeRef>, Error>;
    pub fn endpoints(&self, base: Option<&str>, head: Option<&str>) -> Result<(String, String), Error>;
    pub fn files(&self, base_commit: &str, head_commit: &str) -> Result<Vec<FileChange>, Error>;
    pub fn read_blob(&self, commit: &str, path: &str) -> Result<Option<Vec<u8>>, Error>;
}
```

**`build_grouped` is written and tested but nothing calls it.** Task 5 is where it finally gets used, which is why it was built ahead of time.

Read all of these in the tree before planning anything. Where this plan disagrees with the tree, **the tree wins and you say so in your report.**

---

## Task 1: Symbols as plain data

**Files:** Create `rv-core/src/symbols.rs`, `rv-core/tests/symbols.rs`; modify `rv-core/src/lib.rs`, `rv-core/Cargo.toml`, workspace `Cargo.toml`

**Produces:**

```rust
pub struct Symbol { pub name: String, pub kind: SymbolKind, pub line: u32 }
pub enum SymbolKind { Function, Struct, Enum, Trait, Impl, Module, Constant, Type, Macro, Other }
pub fn of(source: &[u8], path: &str) -> Vec<Symbol>;   // never fails; unknown language yields none
```

**Ruling to implement, and to record in the module doc:** the spec says one parse serves both highlighting and symbols. It does not, and pretending otherwise would be worse than saying so — `tree-sitter-highlight` and `tree-sitter-tags` each parse internally, and unifying them would mean reimplementing the highlighter's injection and locals handling to save milliseconds on review-sized files. **One grammar serves both; each runs its own parse.** Write that in the module doc so the next reader does not go looking for a shared parse that was never there.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rust_items_are_found_with_their_kinds_and_lines() {
    let source = br#"
struct Anchor { line: u32 }

impl Anchor {
    fn resolve(&self, text: &str) -> Option<u32> { None }
}

fn parse(s: &str) -> Anchor { Anchor { line: 0 } }
"#;
    let symbols = of(source, "anchor.rs");
    let named: Vec<(&str, SymbolKind, u32)> =
        symbols.iter().map(|s| (s.name.as_str(), s.kind, s.line)).collect();
    assert!(named.contains(&("Anchor", SymbolKind::Struct, 2)), "got {named:?}");
    assert!(named.iter().any(|(n, k, _)| *n == "resolve" && *k == SymbolKind::Function));
    assert!(named.iter().any(|(n, k, _)| *n == "parse" && *k == SymbolKind::Function));
}

#[test]
fn a_method_and_a_free_function_of_the_same_name_are_both_reported() {
    // Navigation must not silently drop one of two things a reviewer could mean.
    let source = b"fn write(x: u8) {}\nstruct S;\nimpl S { fn write(&self) {} }\n";
    let count = of(source, "s.rs").iter().filter(|s| s.name == "write").count();
    assert_eq!(count, 2, "both are jump targets");
}

#[rstest]
#[case("notes.txt")]
#[case("Makefile")]
#[case("data.json")]
fn a_file_with_no_grammar_yields_no_symbols_rather_than_guessing(#[case] path: &str) {
    assert!(of(b"anything at all\n", path).is_empty());
}

#[test]
fn source_that_does_not_parse_still_returns_what_it_can() {
    let _ = of(b"fn a( { struct \n", "broken.rs");   // must not panic
}

#[test]
fn symbols_come_back_in_line_order() {
    let source = b"fn c() {}\nfn a() {}\nfn b() {}\n";
    let lines: Vec<u32> = of(source, "x.rs").iter().map(|s| s.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "callers step through these in order");
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p rv-core --test symbols`, expect unresolved import.

- [ ] **Step 3: Implement with `tree-sitter-tags`** and the Rust grammar's `tags.scm`. Detect the language by extension only, matching `highlight.rs` — no content sniffing, because guessing wrong is worse than yielding nothing. Lines are 1-based, as everywhere else in this codebase.

- [ ] **Step 4: Run to verify they pass**

- [ ] **Step 5: Add properties** — totality over arbitrary bytes and paths; every reported line is within the source's line count; the same source twice gives identical output. Prove each can fail against a vendored mutant, reporting the kill rate as a fraction over at least 3 runs.

- [ ] **Step 6: Commit** — `jj describe -m "feat(rv-core): extract symbols with tree-sitter tags" && jj new`

---

## Task 2: The in-scope index

**Files:** Create `rv/src/index.rs`, `rv/tests/index.rs`; modify `rv/src/app.rs`, `rv/src/lib.rs`

**Produces:**

```rust
pub struct Entry { pub symbol: Symbol, pub file: usize, pub path: String, pub change_id: Option<String> }
pub struct Index { entries: Vec<Entry> }
impl Index {
    pub fn entries(&self) -> &[Entry];
    pub fn next_after(&self, file: usize, line: u32) -> Option<&Entry>;
    pub fn previous_before(&self, file: usize, line: u32) -> Option<&Entry>;
}
impl App { pub fn index(&mut self) -> &Index; }   // built lazily, rebuilt when the scope changes
```

**Scope is the current view and nothing else**: in the bookmark view, every changed file in the range; in the commits view, only the selected change's files. Symbols come from the **head-side blob** for added, modified and renamed files, and the **base-side blob** for removed ones — you navigate code as it will exist, except where it will not exist at all. That side choice is the one `anchored_side` already encodes; do not write a second copy.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_index_covers_every_file_in_scope() {
    let mut app = rust_workspace().app();          // two .rs files, symbols in both
    let paths: HashSet<&str> = app.index().entries().iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains("a.rs") && paths.contains("b.rs"), "got {paths:?}");
}

#[test]
fn stepping_crosses_a_file_boundary() {
    let mut app = rust_workspace().app();
    let last = last_symbol_of_first_file(&mut app);
    let next = app.index().next_after(last.file, last.symbol.line).expect("a next symbol");
    assert_ne!(next.file, last.file, "n at the last symbol of a file moves to the next file");
}

#[test]
fn stepping_does_not_wrap_around() {
    // Silent wrapping makes a reviewer believe they have seen everything when
    // they have looped.
    let mut app = rust_workspace().app();
    let last = final_symbol(&mut app);
    assert!(app.index().next_after(last.file, last.symbol.line).is_none());
}

#[test]
fn a_file_with_no_grammar_contributes_nothing_and_is_skipped() {
    let mut app = mixed_workspace().app();          // one .rs, one .txt
    assert!(app.index().entries().iter().all(|e| e.path.ends_with(".rs")));
}

#[test]
fn a_removed_file_is_indexed_from_the_base_side() {
    let mut app = deletion_workspace().app();
    let entry = app.index().entries().iter().find(|e| e.path == "gone.rs").expect("indexed");
    assert_eq!(entry.symbol.name, BASE_SIDE_SYMBOL, "its symbols come from the side that still has them");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Build lazily on first use and cache by `(commit, path)`, exactly as the diff and highlight caches already do. Entering the picker parses every in-scope file not yet cached; at review sizes that is milliseconds, and indexing at launch would delay startup for a feature the reviewer may never use.

- [ ] **Step 5: Add the conservation property** — the index's entries are exactly the union of the per-file symbol sets for the files in scope: nothing invented, nothing lost. Prove it can fail.

- [ ] **Step 6: Commit** — `jj describe -m "feat(rv): an index of the symbols in scope" && jj new`

---

## Task 3: Step between symbols with `n` and `N`

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn n_moves_to_the_next_symbol_and_says_where_it_landed() {
    let mut app = rust_workspace().app();
    app.on_key(KeyCode::Char('n')).expect("n");
    assert!(app.status().contains("fn "), "the status names the symbol: {}", app.status());
}

#[test]
fn n_at_the_end_of_a_file_opens_the_next_one() {
    let mut app = rust_workspace().app();
    let file = app.file_index();
    for _ in 0..40 { app.on_key(KeyCode::Char('n')).expect("n"); }
    assert_ne!(app.file_index(), file, "stepping crossed into another file");
}

#[test]
fn n_past_the_last_symbol_stays_put_and_says_so() {
    let mut app = rust_workspace().app();
    for _ in 0..200 { app.on_key(KeyCode::Char('n')).expect("n"); }
    let file_and_line = (app.file_index(), app.line_index());
    app.on_key(KeyCode::Char('n')).expect("n");
    assert_eq!((app.file_index(), app.line_index()), file_and_line);
    assert!(app.status().contains("last"), "it says why nothing happened: {}", app.status());
}

#[test]
fn capital_n_goes_back_the_way_you_came() {
    let mut app = rust_workspace().app();
    let start = (app.file_index(), app.line_index());
    app.on_key(KeyCode::Char('n')).expect("n");
    app.on_key(KeyCode::Char('N')).expect("N");
    assert_eq!((app.file_index(), app.line_index()), start);
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

A jump selects the file, loads its diff, puts the cursor on the symbol's line, and **reports where it landed** — symbol, file, and change. A jump that moves the cursor invisibly is how a reviewer loses their place.

- [ ] **Step 5: Add the round-trip property** — `n` applied `len()` times from the first symbol visits every entry exactly once, in order. Prove it can fail.

- [ ] **Step 6: Commit** — `jj describe -m "feat(rv): step between symbols across the whole range" && jj new`

---

## Task 4: Search symbols with `/`

**Files:** Create `rv/src/picker.rs`, `rv/tests/picker.rs`; modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/layout.rs`, `rv/tests/app.rs`; add `nucleo` to the workspace and `rv` manifests

**Produces:** `Mode::Picker { query: String, selected: usize }`, a `Layout::picker` rect, and `picker::matches(index, query) -> Vec<usize>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn typing_narrows_the_list() {
    let index = sample_index();          // parse, parse_inner, write_markdown, Anchor
    let all = matches(&index, "");
    let some = matches(&index, "parse");
    assert!(some.len() < all.len() && !some.is_empty());
    assert!(some.iter().all(|i| index.entries()[*i].symbol.name.contains("pars")));
}

#[test]
fn a_query_matches_on_the_path_as_well_as_the_name() {
    let index = sample_index();
    let hits = matches(&index, "store write");
    assert!(!hits.is_empty(), "'store write' finds write_markdown in store.rs");
}

#[test]
fn enter_jumps_to_the_selected_symbol() {
    let mut app = rust_workspace().app();
    app.on_key(KeyCode::Char('/')).expect("/");
    for c in "parse".chars() { app.on_key(KeyCode::Char(c)).expect("type"); }
    app.on_key(KeyCode::Enter).expect("enter");
    assert_eq!(app.mode(), Mode::Browse, "the picker closed");
    assert!(app.status().contains("parse"), "and it landed on the symbol");
}

#[test]
fn esc_restores_exactly_where_you_were() {
    let mut app = rust_workspace().app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    let before = (app.file_index(), app.line_index(), app.focus());
    app.on_key(KeyCode::Char('/')).expect("/");
    for c in "parse".chars() { app.on_key(KeyCode::Char(c)).expect("type"); }
    app.on_key(KeyCode::Esc).expect("esc");
    assert_eq!((app.file_index(), app.line_index(), app.focus()), before, "cancelling costs nothing");
}

#[test]
fn the_picker_says_so_when_nothing_matches() {
    let mut app = rust_workspace().app();
    app.on_key(KeyCode::Char('/')).expect("/");
    for c in "zzzznotathing".chars() { app.on_key(KeyCode::Char(c)).expect("type"); }
    assert!(buffer_text(&frame_at(&app, 100, 30)).contains("no match"));
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

Match on the symbol name primarily and the path secondarily, so `store write` finds `write_markdown` in `store.rs`. The picker is a `Mode`, so while it is open the browse keys are inert by construction rather than by a check in every handler.

- [ ] **Step 5: Commit** — `jj describe -m "feat(rv): fuzzy symbol search" && jj new`

---

## Task 5: The commits view, and scope

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/index.rs`, `rv/tests/app.rs`, `rv/tests/app_cases.rs`

**This is where `tree::build_grouped` finally gets called.** It was written and tested in the viewport work precisely so this task would be wiring rather than design.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tab_cycles_files_then_commits_then_comments() {
    // A reviewer reaches for Tab to change what the left column shows, and
    // having reached for it should arrive. One mechanism, not two.
    let mut app = workspace().app();
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
    assert!(buffer_text(&frame_at(&app, 100, 30)).contains("second change"), "commits are listed");
    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files, "it cycles");
}

#[test]
fn a_commit_expands_to_the_files_it_touched() {
    let mut app = two_change_workspace().app();
    app.on_key(KeyCode::Tab).expect("commits tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let collapsed = sidebar_row_count(&frame_at(&app, 100, 30));
    app.on_key(KeyCode::Enter).expect("expand the commit");
    assert!(sidebar_row_count(&frame_at(&app, 100, 30)) > collapsed, "its files appeared");
}

#[rstest]
#[case(true)]
#[case(false)]
fn t_chooses_whether_a_commits_files_are_a_tree_or_a_flat_list(#[case] tree: bool) {
    let mut app = two_change_workspace().app();
    app.on_key(KeyCode::Tab).expect("commits tab");
    if tree { app.on_key(KeyCode::Char('t')).expect("tree"); }
    let text = buffer_text(&frame_at(&app, 100, 30));
    assert_eq!(text.contains("src"), tree, "a directory row appears only in tree mode");
}

#[test]
fn one_and_two_jump_straight_to_a_tab_without_cycling() {
    let mut app = workspace().app();
    app.on_key(KeyCode::Char('2')).expect("2");
    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
    app.on_key(KeyCode::Char('1')).expect("1");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
}

#[test]
fn the_commits_view_scopes_the_symbol_index_to_the_selected_change() {
    let mut app = two_change_workspace().app();
    let whole = app.index().entries().len();
    app.on_key(KeyCode::Char('2')).expect("2");
    let scoped = app.index().entries().len();
    assert!(scoped < whole, "scope narrowed to one change: {scoped} < {whole}");
    assert!(app.index().entries().iter().all(|e| e.change_id.as_deref() == Some(app.selected_change())));
}

#[test]
fn the_status_bar_names_the_scope_in_words() {
    let mut app = two_change_workspace().app();
    assert!(bar_text(&frame_at(&app, 120, 30)).contains("all"), "the bookmark view says how much is in scope");
    app.on_key(KeyCode::Char('2')).expect("2");
    assert!(bar_text(&frame_at(&app, 120, 30)).contains("change"), "and the commits view says which change");
}

#[test]
fn switching_views_keeps_the_file_you_were_on() {
    let mut app = two_change_workspace().app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    let path = app.files()[app.file_index()].path.clone();
    app.on_key(KeyCode::Char('2')).expect("2");
    assert_eq!(app.files()[app.file_index()].path, path, "landing at the top of an unrelated list is disorienting");
}
```

- [ ] **Step 2-4: Run (expect FAIL), implement, run (expect PASS)**

A change's diff is its parent's tree against its own — `Repository::files` and `read_blob` already take arbitrary commit ids, so the commits view needs no new rv-core surface.

**A commit row's counts and sort key are the sum of its files' `Stat`s**, which the viewport work computes at startup. Wire that sum here so a commit row shows `+148 −12` like every other row, and so `o` can order commits by additions or removals as well as by time. `Sort::Natural` for commits means stack order, newest first — the order they already have. **The status bar names the scope in words at all times**, because a jump that silently searches a different set than the reviewer assumes is worse than no jump.

- [ ] **Step 5: Commit** — `jj describe -m "feat(rv): review the whole bookmark or one commit at a time" && jj new`

---

## Task 6: Document it

**Files:** Modify `README.md`, `rv/tests/app_cases.rs`

- [ ] Document `/`, `n`, `N`, `1` and `2` in the keybinding table, arrows-first convention intact. Say which languages have symbols (the grammars rv ships) and that other files simply have none. State the scope rule in one sentence — *the bookmark view searches every changed file; the commits view searches the selected change* — because it is the thing a user will otherwise have to infer from behaviour. Extend the README-versus-code test to the new bindings.
- [ ] `cargo test --workspace`, clippy and fmt clean. Commit.

---

## Self-review

Spec coverage: §3's two views → Task 5; §4's symbol model → Task 1, its index and scope → Task 2 and Task 5, its `/` picker → Task 4, its `n`/`N` stepping → Task 3; §5's ergonomics — the status bar naming the scope, every jump reporting where it landed, `Esc` backing out without cost — are requirements inside Tasks 3, 4 and 5 rather than a task of their own. §6's test list is distributed across the tasks that own each behaviour. §7's non-goals are respected: no reference search, no `gd`/`gr`, no symbol timeline, no `NodePath` anchoring, no whole-repository index.

Deliberate deviation, recorded in Task 1: the spec claims one parse serves both highlighting and symbols. It does not — `tree-sitter-highlight` and `tree-sitter-tags` each parse internally — and the module doc says so rather than leaving a reader hunting for a shared parse that never existed. One grammar serves both; that is the part that matters, because it is what keeps the language set from diverging.

Ordering: Task 1 has no dependencies; Task 2 needs 1; Task 3 needs 2; Task 4 needs 2; Task 5 needs 2 and the viewport work's `tree::build_grouped`; Task 6 needs all. Tasks 2 through 5 all touch `rv/src/app.rs` and must not run concurrently.
