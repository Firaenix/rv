
# rv Milestone 1 — Self-Hosting Minimum Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `rv` binary that reviews the current jj change stack in a TUI, lets you attach line comments, and writes them to `.review/REVIEW-FEEDBACK.md` — the point at which `rv` can review its own development.

**Architecture:** Cargo workspace with `rv-core` (terminal-free: jj-lib access, diff production, anchors, `.review/` I/O) and `rv` (clap CLI + ratatui TUI). jj-lib is accessed in-process; difftastic is invoked as a subprocess for line alignment with a `similar` fallback. Comments anchor by content hash with a line-number fallback; tree-sitter node anchoring is Milestone 3, not this plan.

**Tech Stack:** Rust 2024 edition, `jj-lib` 0.44 (feature `git`), `pollster` + `futures` (jj-lib is async), `ratatui` 0.30, `crossterm` 0.29, `similar` 3.2, `blake3`, `serde`/`serde_json`/`toml`, `clap` 4, `thiserror`, `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-08-17-rv-branch-reviewer-design.md`

## Global Constraints

- **MSRV 1.89**, edition 2024 — forced by `jj-lib` 0.44 (`rust-version = "1.89"`, `edition = "2024"`).
- **`rv-core` MUST NOT depend on `ratatui`, `crossterm`, or `tui-textarea`.** The terminal-free boundary is what makes anchoring testable; Task 10 verifies it mechanically.
- **Never read the user's jj config.** No `~/.config/jj/config.toml`, no repo config, no `revset-aliases` lookup. `UserSettings` is built from a `StackedConfig` that `rv` populates itself.
- **`jj-lib` is imported only in `rv-core/src/vcs.rs`.** Every other module takes plain Rust types. This bounds the blast radius of a jj-lib bump to one file.
- **Read-only.** Never start a working-copy mutation, never begin a transaction, never write to `.jj/`. `rv` writes only under `.review/` and appends one link to `.git/info/exclude`.
- **`change_id` is identity; `commit_id` is advisory.** Never use `commit_id` to decide staleness.
- **change ids are stored and displayed via `ChangeId::reverse_hex()`** (the `z-k` alphabet, e.g. `nowwnlnmvkwo`), never raw hex — this is the `jj log` display form.
- **difft is invoked as `DFT_UNSTABLE=yes difft --display json <old> <new>`.** Read the `status` field (`"changed"` / `"unchanged"`); never trust `--exit-code`.
- Commits use jj, not git: `jj describe -m "…" && jj new`.
- Tests may shell out to the `jj` CLI to build fixture repos. Production code must not.

### Verified API reference

Confirmed by compiling and running a probe against this repo (jj-lib 0.44) on 2026-08-17.

```rust
let mut config = StackedConfig::with_defaults();
config.add_layer(ConfigLayer::parse(ConfigSource::Default,
    "user.name = \"rv\"\nuser.email = \"rv@localhost\"\n")?);
let settings = UserSettings::from_config(config)?;

let workspace = Workspace::load(&settings, path,
    &default_backend_factories(), &default_working_copy_factories())?;
// NOTE: async
let repo = pollster::block_on(workspace.repo_loader().load_at_head())?;

// Revsets yield STREAMS, not iterators.
let resolver = SymbolResolver::new(repo.as_ref(), &[] as &[Box<dyn SymbolResolverExtension>]);
let resolved = expr.resolve_user_expression(repo.as_ref(), &resolver)?;
let revset = RevsetExpression::evaluate(resolved, repo.as_ref())?;
let pairs: Vec<_> = pollster::block_on(revset.commit_change_ids().try_collect())?;

// Trees, rename-aware diffs, blob reads.
let tree = commit.tree();                       // sync
let mut cr = CopyRecords::default();
let recs = pollster::block_on(store.get_copy_records(None, &b, &h)?.try_collect())?;
cr.add_records(recs);
let mut s = base_tree.diff_stream_with_copies(&head_tree, &EverythingMatcher, &cr);
while let Some(entry) = pollster::block_on(s.next()) {
    let jj_lib::merge::Diff { before, after } = entry.values?;   // struct fields
    if let Some(TreeValue::File { id, .. }) = after.as_normal() {
        let mut r = pollster::block_on(store.read_file(path, id))?;
        let mut buf = Vec::new();
        pollster::block_on(futures::io::AsyncReadExt::read_to_end(&mut r, &mut buf))?;
    }
}
```

Required imports: `futures::{StreamExt as _, TryStreamExt as _}`, `jj_lib::object_id::ObjectId as _` (`.hex()`, `.reverse_hex()`), `jj_lib::repo::Repo`.

Vanilla `trunk()`, built with typed constructors — no alias table, no config:

```rust
fn trunk_expression() -> Arc<UserRevsetExpression> {
    let mut c = Vec::new();
    for remote in ["origin", "upstream"] {
        for name in ["main", "master", "trunk"] {
            c.push(RevsetExpression::remote_bookmarks(RemoteRefSymbolExpression {
                name: StringExpression::exact(name),
                remote: StringExpression::exact(remote),
            }, None));
        }
    }
    c.push(RevsetExpression::root());
    RevsetExpression::union_all(&c).latest(1)
}
```

---

## Task 1: Workspace scaffold and repository handle

**Files:** Create `Cargo.toml`, `rv-core/Cargo.toml`, `rv-core/src/lib.rs`, `rv-core/src/model.rs`, `rv-core/src/vcs.rs`, `rv-core/tests/fixture.rs`, `rv-core/tests/vcs.rs`
**Produces:** `rv_core::vcs::Repository::open(&Path) -> Result<Repository, Error>`; `Repository::stack(None,None) -> Result<Vec<ChangeRef>, Error>`; `ChangeRef { change_id, commit_id, description }` (change_id reverse-hex).

- [ ] Create workspace `Cargo.toml` with members `["rv-core","rv"]`, else/workspace edition `"2024"`, rust-version `"1.89"`, and workspace deps: jj-lib 0.44 (git), futures 0.3, pollster 1.0, similar 3.2, blake3 1, serde(derive), serde_json 1, toml 0.9, thiserror 2, clap 4(derive), tempfile 3.
- [ ] `rv-core/Cargo.toml` with those deps (dev-deps: tempfile).
- [ ] `rv-core/tests/fixture.rs`: `Fixture` with `tempdir`, `jj(["git","init","--colocate"])`, `jj(&[&str]) -> String`, `write(rel, contents)`, `commit(msg)` = `jj describe -m msg` + `jj new`.
- [ ] `rv-core/tests/vcs.rs`: test `stack_lists_changes_newest_first_with_reverse_hex_ids` (build 2 changes, assert descriptions `["","second change","first change"]`, assert every change_id in `k..=z`); test `empty_range_is_an_error_naming_endpoints` (`stack(Some("@"),Some("@"))`, assert err contains "empty").
- [ ] Run `cargo test -p rv-core --test vcs` — expect FAIL (unresolved import).
- [ ] `rv-core/src/model.rs`: `ChangeRef`, `Side {Left,Right}`, `ChangeKind { Added, Modified, Removed, Renamed }`, `FileChange { path, source_path, kind, binary }` — all Serialize/Deserialize/Clone/Debug/PartialEq.
- [ ] `rv-core/src/vcs.rs`: `LINKED_JJ_LIB = "0.44"`; `enum Error { NotAWorkspace(String), Incompatible { linked, source_message }, Unresolved(String), EmptyRange { base, head }, Jj(String) }` (thiserror); `settings()` builds a `StackedConfig` via `ConfigLayer::parse(ConfigSource::Default, ...)` (never reads user config); `trunk_expression()` as above; `Repository::open` maps load errors to NotAWorkspace / compatible; `evaluate(expr)` resolves+evals, collects `commit_change_ids()`, fetches each commit for description; `parse_or_default` (None => trunk, "@" => working_copy, else `RevsetExpression::symbol`); `stack` returns changes newest-first, errors Empty when empty.
- [ ] `rv-core/src/lib.rs`: `pub mod model; pub mod vcs;`
- [ ] Run tests — expect PASS.
- [ ] Commit: `jj describe -m "feat(rv-core): jj-lib repository handle and stack enumeration" && jj new`


## Task 2: File enumeration and blob reads

**Files:** Modify `rv-core/src/vcs.rs`, `rv-core/tests/vcs.rs`
**Produces:** `Repository::endpoints(base,head) -> Result<(String,String),Error>` (hex commit ids); `Repository::files(base_commit,head_commit) -> Result<Vec<FileChange>,Error>`; `Repository::read_blob(commit_id,path) -> Result<Option<Vec<u8>>,Error>` (None when absent).

- [ ] Add to `rv-core/tests/vcs.rs`: `files_reports_added_paths_and_reads_blobs` (write a.rs, commit, rename to b.rs + extend, commit; assert kind Added, blob equals, read_blob on base is None); `rename_between_two_changes_is_reported_with_its_source` (endpoints(None,Some("@-")).1 as base, then rename+commit, assert kind Renamed, source_path a.rs); `binary_files_are_flagged_not_decoded` (write logo.bin bytes `[0,159,146,150]`, commit, assert binary).
- [ ] Run — expect FAIL (no endpoints/files/read_blob).
- [ ] In vcs.rs add imports: `futures::StreamExt as _`, `jj_lib::backend::{CommitId,FileId,TreeValue}`, `jj_lib::copies::CopyRecords`, `jj_lib::matchers::EverythingMatcher`, `jj_lib::merge::MergedTreeValue`, `jj_lib::merged_tree::TreeDiffEntry`, `jj_lib::repo_path::{RepoPath,RepoPathBuf}`.
- [ ] Implement: `single_commit(expr)` (evals, first id from revset.stream, else Unresolved); `endpoints` (base=parse_or_default(None,trunk)+single_commit+`.hex()`, head default working_copy); `commit_by_hex` via `CommitId::try_from_hex`; `files`: get both commits, `.tree()` (sync), gather CopyRecords, `diff_stream_with_copies`, for each entry detect rename via source!=target, detect binary via side_looks_binary, destructure `Diff { before, after }`, map kind, sort by path; `value_looks_binary` reads first 8192 bytes and checks for a NUL; `read_blob` uses `commit.tree().path_value(&repo_path)` (async) match `as_normal() -> TreeValue::File { id,.. }`.
- [ ] Run — expect PASS (5 tests).
- [ ] Commit.

## Task 3: Diff production — difft JSON with similar fallback

**Files:** Create `rv-core/src/diff.rs`, `rv-core/tests/diff.rs`
**Produces:** `rv_core::diff::{FileDiff, DiffLine, LineKind, DiffSource, compute, compute_with}`. `compute(old: Option<&[u8]>, new: Option<&[u8]>, path:&str)->FileDiff`. `FileDiff { path, lines, source, suppressed }`. `DiffLine{kind,left:Option<u32>,right:Option<u32>,text}`. `DiffSource::{Difftastic{language}, Similar, Binary}`. binary if either side contains NUL.

- [ ] tests in `rv-core/tests/diff.rs`: changed_line (old `fn a() {\n    let x = 1;\n}\n`, new with 2; assert 1 Removed + 1 Added with line 2 both sides); reindentation_only_suppressed (assert suppressed true); binary (source==Binary, lines empty); fallback (`compute_with(...,false)` source==Similar, added line c); all_additions for a new file (None->blob, two Added).
- [ ] `compute_with(old,new,path,use_difft)` handles NUL binary check first, then if `std::env::var_os("RV_NO_DIFFT").is_none()` tries difft else similar. `compute` delegates.
- [ ] Run — expect PASS (5 tests).
- [ ] Commit.

## Task 4: Anchors — content hashing and the resolution cascade

**Files:** Create `rv-core/src/anchor.rs`, `rv-core/tests/anchor.rs`
**Produces:** `model::{Anchor, Confidence}`; `anchor::{normalize, content_hash, snapshot_of, create, resolve}`. `create(file, side, line, text)->Anchor`; `resolve(anchor,&str)->(Option<u32>,Confidence)`. Confidence: Exact|Moved|Weak|Outdated.

- [ ] `model.rs`: `Confidence` enum + `as_str()` (exact/moved/weak/outdated); `Anchor { file:String, side, line:u32, content_hash:String, context:Vec<String> }` all Serialize/Deserialize etc.
- [ ] tests: unchanged_resolves_exact; shifted_line_resolves_moved (prepend 2 lines, line 2->4 Moved); normalized hash survives reindent (spaces/tabs collapse -> Exact); deleted_line_outdated; duplicate_content_resolves_nearest (text `a\nb\nx\nc\n` line 3, after edit `x\na\nb\nx\nc\n` -> (Some(4),Moved)); snapshot_captures_context (center line 7 of 12 -> 11 context lines).
- [ ] `anchor.rs`: `normalize` = split_whitespace().join(" "); `content_hash` = blake3 hex of normalized; `snapshot_of(text,line)` = lines[line-6..line+5] (up to 5 before/after, 1-based, clamped); `create` hashes the target line; `resolve`: exact if same line hashes, else nearest by abs_diff in line, else Outdated.
- [ ] Run — expect PASS (6 tests).
- [ ] Commit.

## Task 5: .review/ store

**File:** Create `rv-core/src/store.rs`, `rv-core/tests/store.rs`
**Produces:** `Comment{id,change_id,commit_id,anchor,body,state,reply}`, `CommentState::{Open,AwaitingVerification,Resolved,Outdated}`, `Session{revset,base_commit,head_commit,changes,started_at}`; `Store::{open,ensure_excluded,write_session,read_session,comments,append_comment,markdown_path}`.

- [ ] tests: ensure_excluded_adds_review_exactly_once (first true, second false, exactly one `/\.review\//` in .git/info/exclude); appended_comments_persist_immediately (fresh Store sees them — write-through); same_id_updates; snapshot_file_written per comment; session_toml roundtrip.
- [ ] `Store::open` creates `.review/snapshots`; `ensure_excluded` appends `/.review/` to `.git/info/exclude` if absent; `append_comment` writes `snapshots/<id>` and updates `comments.json` via upsert, write-through.
- [ ] Run — expect PASS.
- [ ] Commit.

## Task 6: Markdown render and reply parsing

**Files:** Create `rv-core/src/markdown.rs`, `rv-core/tests/markdown.rs`
**Produces:** `render(session,comments)->String`; `parse_replies(&str)->Vec<(String,String)>`.

- [ ] `markdown.rs`: header `<!-- rv:v1 -->`, `# Review:` line, base→head line, PROTOCOL block ("**For LLMs:** fix each open comment ... Do not mark anything resolved — the human verifies ..."). Sections in order Open / Awaiting verification / Resolved / Outdated, each w/ count. Expanded vs `<details>` collapsed for Resolved/Outdated. Entry: `### n. \`path:line\`` then `<!-- rv:anchor id=... change=... commit=... side=... line=... hash=... -->` then a code fence with `context` lines then `**Comment:** body` then optional `**Reply:** reply`.
- [ ] tests: open_renders_expanded_with_anchor_and_protocol; resolved_and_outdated_render_collapsed (2 `<details>`); replies_parsed_by_id; hand_edited_prose_does_not_break_parsing; reply_binds_to_nearest_preceding_anchor.
- [ ] `parse_replies`: track last `<!-- rv:anchor id=`; upon `**Reply:**` push `(current_id, body)`.
- [ ] Run tests; commit.


## Task 7: CLI with render and status

**Files:** Create `rv/Cargo.toml`, `rv/src/lib.rs`, `rv/src/main.rs`, `rv/src/session.rs`, `rv/tests/cli.rs`
**Produces:** `rv::session::{Review, build}`: `build(repo_root,&str) -> anyhow::Result<Review>`, `Review { repo, store, session, files }`; binary subcommands `render` and `status [--json]`. Add the `rv` crate as `[lib] name=rv` + `[[bin]] name=rv` so tests use `CARGO_BIN_EXE_rv`.

- [ ] Add `anyhow = "1"` to workspace deps. `rv/Cargo.toml`: rv-core path dep, ratatui 0.30, crossterm 0.29, clap.workspace, anyhow, serde_json; dev-dep tempfile.
- [ ] `rv/tests/cli.rs`: `render_writes_markdown_and_excludes` (run `CARGO_BIN_EXE_rv render`, assert md starts `<!-- rv:v1 -->`, contains `**For LLMs:**`, .git/info/exclude contains `/.review/`); `status_json_reports_range_and_zero_comments` (assert v["comments"]["open"]==0, changes len 2, files contains a.rs); `empty_range_fails_naming_endpoints` (`rv --from @ --to @ status` non-zero, err contains "empty").
- [ ] `rv/src/session.rs`: `Review{build}` — open repo, `repo.stack(base,head)`, `repo.endpoints(base,head)`, `repo.files`, `Store::open`, `ensure_excluded`, build Session with `revset = "{base}..{head}"` (defaults "trunk()"/"@"), `started_at` = "epoch:{unix_secs}" (no chrono dep).
- [ ] `rv/src/main.rs`: clap `Cli { target:Option<String>, --from, --to, --repo }` + subcommands Render/Status{--json}. `main`: build review, then on Render write markdown via `markdown::render`; on Status print or JSON (revset, base, head, changes[{change_id,commit_id,description}], files[{path,kind,binary}], comments{open,awaiting_verification,resolved,outdated}). For now default = Render (TUI is Task 8).
- [ ] Add `pub mod app; pub mod session; pub mod ui;` to `rv/src/lib.rs`; create empty `app.rs`, `ui.rs`.
- [ ] Run `cargo test -p rv --test cli` — expect PASS.
- [ ] Commit.

## Task 8: TUI — browse the diff and write comments

**Files:** Modify `rv/src/app.rs`, `rv/src/ui.rs`, `rv/src/main.rs`; Test `rv/tests/app.rs`
**Produces:** `rv::app::{App, Mode, Action}` with `App::new(review)->Result<App>`, `App::on_key(&mut self, KeyCode)->Result<Action>`, `App::selected_file()/selected_diff()/mode()/buffer()`, `App::run(review)`; `rv::ui::draw(frame, app)`. `on_key` is terminal-free so the state machine is unit-tested.

- [ ] `rv/tests/app.rs`: first_file_selected_and_diff_available; typing_a_comment_persists_against_selected_line (j, c, type "needs a doc", Enter, then fresh Store sees `comments.len()==1`, body=="needs a doc", state Open, anchor.file=="a.rs"); escape_abandons (c, type x, Esc, mode Browse, store empty); frame_renders_file_list_and_diff (TestBackend 100x24, assert contains "a.rs" and "let x = 1;").
- [ ] `rv/src/app.rs`: `Mode::{Browse,Comment}`, `Action::{Continue,Quit}`. `App{diffs:Vec<Option<FileDiff>>, file_index, line_index, mode, buffer, status, review}`. `load_selected` reads blobs lazily for the selected file (`read_blob(base, source_path)`, `read_blob(head, path)`), `diff::compute`, cache. `on_key_browse`: q quit, j/k or Down/Up move line, ]/[ move file + load, c enters Comment (clear buffer). `on_key_comment`: Esc cancel, Backspace pop, Char push, Enter persist. `commit_comment`: pick Side by diff line kind (Removed->Left, else Right), pick commit=base for Left else head, pick path = source_path for Left else path, anchor = create(path, side, line, blob_text), comment id = first 4 hex of blake3(change_id:path:line:body), change from session.changes.first(), state Open, reply None, `append_comment`, rewrite markdown, status = "comment saved at {path}:{line}". `run`: set panic hook to restore terminal then default; `ratatui::init()`, loop draw + event::read + on_key, Quit breaks, `ratatui::restore()`.
- [ ] `rv/src/ui.rs`: vertical: status bar (1 or 3 rows in Comment mode) over panes (30% sidebar, 70% diff). Sidebar List of files with a `+ - -> ~` marker per ChangeKind, highlight selection. Diff pane: title from source (Difftastic{language}/fallback/Binary), `suppressed` -> "no semantic change", lines `{right.or(left):>5} {sigil}` where sigil +/-/space and color Green/Red/Gray, selected line Modifier::REVERSED. Sidebar scroll window around `line_index`. Comment mode bottom bar shows `app.buffer()` with title "Comment".
- [ ] `main.rs`: default (None) now calls `rv::app::App::run(review)`; Render and `status --json` are explicit subcommands.
- [ ] Add `blake3.workspace = true` to `rv/Cargo.toml`.
- [ ] Run `cargo test -p rv` — expect PASS (4 app + 3 cli).
- [ ] Commit.

## Task 9: Smoke test the binary against its own stack (dogfood)

**Files:** Create `README.md`; fix anything the smoke test reveals.
- [ ] `cargo test --workspace`; then `cargo install --path rv --root target` or use `target/release/rv`.
- [ ] `./target/release/rv status` in this repo — expect revset line, change count matching `jj log -r 'trunk()..@'`, one file per file this plan created.
- [ ] `./target/release/rv` launches the TUI; exercise every binding (sidebar lists files, ]/[ navigate and diff scrolls, j/k move highlight, `c` opens buffer, Esc discards, Enter saves -> status shows path:line, `q` exits, terminal is gamble (prompt ok, no stuck raw).
- [ ] `cat .review/REVIEW-FEEDBACK.md` shows the comment under `## Open (1)` with the anchor; `git status --short` and `jj status` do NOT show `.review/` (if they do, `ensure_excluded` is broken — fix first, else the change mutates every run).
- [ ] Simulate an LLM reply: `printf '\n**Reply:** Fixed by using unwrap_or(0).\n' >> .review/REVIEW-FEEDBACK.md`, then confirm `cargo test -p rv-core --test markdown` still passes (reply parsed; state transitions are Milestone 2).
- [ ] Write README explaining: no forge/no network, jj-native; usage `rv`, `rv <bookmark>`, `rv --from --to`, `rv render`, `rv status --json`; keybindings; `RV_NO_DIFFT=1`.
- [ ] Commit.

## Task 10: Enforce the constraints mechanically

**Files:** Create `rv-core/tests/constraints.rs`
- [ ] `rv_core_tests` read `Cargo.toml` + all `src/*.rs`: (1) rv-core manifest contains no `ratatui|crossterm|tui-textarea`; (2) only `vcs.rs` mentions `jj_lib`; (3) no source reads user config: no `config_path`, `ConfigSource::User`, `ConfigSource::Repo`.
- [ ] Run `cargo test --workspace` — expect all pass.
- [ ] Commit.

## Self-review

Spec §14 milestone-1 items map 1:1 to tasks 1-9; §6 zero-config -> Task 1 settings + Task 10 test; §10 exclude -> Task 5, verified in Task 9. Node anchoring/awaiting-verification/timeline belong to M2/M3/M4 — deliberately absent. `Confidence::Weak`/`Outdated` and `AwaitingVerification` defined but only produced in later milestones. Signatures consistent across tasks (`store.append_comment(&Comment)`, `diff::compute`->`FileDiff`, `Repository::endpoints`->`(String,String)` hex, `anchor::create(file,side,line,text)`). Keyboard: `rv` default launches TUI; `rv render`/`rv status` headless.

