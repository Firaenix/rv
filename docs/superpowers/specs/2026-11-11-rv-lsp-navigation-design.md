# rv — language-server navigation in the TUI

Date: 2026-11-11. Plan: Phase 2 of the whole-repo/LSP/agent plan
(`rv-core/src/lsp/` is the client; this note covers the TUI half,
`rv/src/app/lsp.rs` and `rv/src/app/lsp/`).

## Shape

Tree-sitter answers `g d`, `g r` and `g i` immediately, exactly as before.
When `lsp = "auto"` and the reviewed head is the working copy, the same
keystroke also sends one request to the language server and returns. The event
loop polls the answer (`App::collect_lsp`, beside `collect_refined` and
`collect_merged`) and swaps it in under the rules below — the same
render-now-refine-later shape as `similar` → difftastic. No app code waits on a
ticket.

## When the server is asked at all

- `lsp = "off"` (default): never; no `nav:` segment, the reviewer sees today's rv.
- Head is not the working copy (`Repository::head_is_working_copy`): never. The
  bar says `nav: grammar`; the jump list title and the hover card say
  `grammar only — this revision is not on disk`.
- The cursor's line has no right-hand (head) line number — a removed line, or a
  `before` view — or the pane shows a commits-tab change's own diff rather than
  the bookmark's: never (`grammar only — this line is not on disk`). The
  server reads the disk, and only the head side of the bookmark diff is the
  disk.
- Position sent: the head line (1-based) and the start of the word under the
  column cursor (0-based characters); rv-core converts to UTF-16.

## Staleness

- One request is outstanding at a time. Each `g d`/`g r`/`g i` bumps a
  generation and replaces the pending ticket; an answer carrying an older
  generation is dropped. So a newer request always wins, whatever order the
  server answers in.
- A request unanswered 30 s after the loop first saw it is dropped silently
  (rv-core has no request timeout; the clock is read only by the event loop).
- An error answer changes nothing; the bar's `nav: unavailable` says why.
- Answers are cached per `(path, line, column, kind)` against the head commit
  id. A refresh keeps the running server (if the head is still the working
  copy) but drops the pending request and the cache.

## Swap rules

"Unmoved" means the reviewer's *spot* — selected file, cursor code position
(path, side, line — not plan row, so a background diff swap does not count as
moving), column cursor and mode — equals the spot recorded right after the
grammar acted.

Definition (`g d`):

1. The grammar's Definitions list is still open for the same word → its rows
   are replaced by the server's, list cursor kept on the same place if present.
2. Else, if unmoved and the server gave several targets → that list opens.
3. Else, if unmoved and the single target is openable → the cursor jumps there
   (unless already there) and the column goes to the server's column. If the
   grammar had already jumped, its intermediate stop is removed from the `g b`
   stack, so one `g b` undoes the whole `g d`.
4. Single target outside the review, or the reviewer moved → the cursor stays;
   only the status reports `rust-analyzer: name — path:line`.

The rule is deliberately "never move a reviewer who has moved": a jump arriving
seconds later under someone reading elsewhere is worse than a stale answer.

References (`g r`): replaced in place if the grammar's list (References or
Lines) is still open for the same word; opened if the grammar found nothing
and the reviewer is unmoved; otherwise dropped.

Hover (`g i`): if the key-opened card for the same name is still open, the
server's text fills it (`source = LanguageServer`) in place of the quoted
declaration. Pointer-raised cards never ask. The text is untrusted: ANSI/OSC
escapes and control characters are removed, code fences and `---` dropped,
six lines of at most 160 characters kept.

Locations outside the review's file list (a dependency, `target/`, a file the
range did not touch) are listed with an `outside` mark and the path, and `Enter`
on one says it cannot be opened. In a tree review every tracked file is in the
list, so only untracked or out-of-repo paths are outside.

## Status bar

`nav: grammar` (head not on disk, no server for the file, or not started) ·
`nav: rust-analyzer starting` · `nav: rust-analyzer indexing 42%` ·
`nav: rust-analyzer` · `nav: unavailable`. Absent when `lsp = "off"`. Ranked
lowest of all segments, so at 80 columns it drops before anything else —
including the status.

## Lifecycle

The selected file's server is started from the poll as soon as the file is
shown, so it indexes while the reviewer reads. Dropping the hub can take ~1.5 s
per server, so it is dropped on a background thread; at exit the process does
not wait, and the server sees its stdin close.

## Discovery

`lsp = "auto"` means *find one*, not *rust-analyzer only*. Each language has an
ordered candidate list (`rv-core/src/lsp/discovery.rs`); the first program
found wins, and its absolute path is pinned into the command so the status bar
and `rv lsp` name exactly what runs. A configured command is never discovered
over, and an empty one disables the language, discovery included; a configured
bare program name is resolved through the same search.

Search order: the repo's `node_modules/.bin`, `.venv/bin`, `venv/bin`; `PATH`;
`~/.cargo/bin`, `$GOPATH/bin` or `~/go/bin`, `$XDG_DATA_HOME/nvim/mason/bin`,
`~/.local/bin`. The installer directories are there because servers installed
by rustup, `go install` or Mason are often off a GUI-launched terminal's `PATH`.

Repo-local hits are accepted only when jj does not track the file. The tree
under review is untrusted input: without this, a branch could commit
`node_modules/.bin/typescript-language-server` and rv would execute it the
moment a reviewer opened a `.ts` file. Installed (ignored) tool directories
are the reason to look there at all, and those are untracked by definition.

A language with nothing installed gets no server, so the bar reads
`nav: grammar` rather than `nav: unavailable`; `unavailable` is kept for a
server that was chosen and failed. Discovery runs when the hub is built (at
startup, or when a refresh brings the head back onto disk); a refresh that
keeps the running hub does not re-discover, so a newly installed server needs a
restart.
