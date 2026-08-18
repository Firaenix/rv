//! Every symbol in scope, and the order you step through them in.
//!
//! Scope is the current view and nothing else: the whole bookmark, or one
//! change at a time. Built lazily and cached, like the diff and highlight
//! caches, because indexing at launch would delay startup for a feature the
//! reviewer may never use.
//!
//! # A pure model
//!
//! Nothing here reads a repository, a blob or a file. [`Index::of`] is handed
//! the files in scope and the bytes each one's symbols come from, and it
//! returns a list; that is the whole interface. Two things follow, and both
//! are the reason it is written this way:
//!
//! * **The caller owns the scope.** The bookmark view passes every changed
//!   file in the range; the commits view passes one change's files. This
//!   module has no opinion about which, so there is no second definition of
//!   "in scope" to drift from the one the sidebar is drawing.
//! * **The caller owns the numbering.** [`Scoped::file`] is whatever index the
//!   caller addresses a file by — a position in `App::files()` — and comes back
//!   on every [`Entry`] untouched. A model that renumbered files would hand
//!   back a jump the caller could not perform.
//!
//! # Which side a file's symbols come from
//!
//! The head one, except for a removed file, whose symbols come from the base:
//! you navigate code as it will exist, except where it will not exist at all.
//! That is [`indexed_side`], and it is not a second copy of the project's side
//! rule — it asks [`crate::app::anchored_side`] the same question about a whole
//! file that the diff pane asks about one line, because a removed file is
//! nothing but removed lines.
//!
//! # The order, and the cursor that walks it
//!
//! Entries are in the caller's file order and, within a file, in line order.
//! [`Index::next_after`] and [`Index::previous_before`] take the cursor's
//! position — a file and a line — rather than an entry, because most of the
//! time the cursor is sitting on neither a symbol nor even a line that has
//! one.
//!
//! **Stepping does not wrap.** Past the last symbol is `None`, and a caller
//! that wants to say "you are at the last symbol" can. Silent wrapping makes a
//! reviewer believe they have seen everything when they have looped, and a
//! review that felt complete and was not is the failure this whole tool exists
//! to prevent.
//!
//! Two consequences worth knowing before reading the tests:
//!
//! * **A file with no grammar contributes nothing and is still a place to step
//!   from.** It has no entries, but it has a position in the order, so `n` from
//!   inside it lands on the next file's first symbol rather than on nothing.
//! * **Two definitions on one line are one stop.** The cursor is a `(file,
//!   line)` pair, so it cannot tell them apart; `n` moves it to the next
//!   *line* that has a symbol, because a jump that lands where it started
//!   reads as a broken key. Both are still in [`Index::entries`], which is
//!   what the picker lists.

use std::collections::HashMap;

use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::symbols;
use rv_core::symbols::Symbol;

use crate::app::anchored_side;

/// One file in scope, with the bytes its symbols could come from.
///
/// Both blobs, not one: which of them is indexed is [`indexed_side`]'s
/// decision and this module's rule, so a caller cannot get it wrong by handing
/// over the wrong side. Either may be `None` — an added file has no base, a
/// removed file has no head, and a binary or unreadable file has neither —
/// and a file whose own side is missing simply contributes nothing.
#[derive(Clone, Copy, Debug)]
pub struct Scoped<'a> {
    /// How the caller addresses this file: a position in its own list, handed
    /// back on every [`Entry`] this file produces.
    pub file: usize,
    /// The path the file is listed under, which is also what selects the
    /// grammar. Head-side for everything but a removal, which has only the one
    /// path.
    pub path: &'a str,
    /// How the file changed, which is the whole of what decides its side.
    pub kind: ChangeKind,
    /// The change that touched it, in the commits view. `None` in the bookmark
    /// view, where a symbol belongs to the range rather than to one change.
    pub change_id: Option<&'a str>,
    pub base: Option<&'a [u8]>,
    pub head: Option<&'a [u8]>,
}

impl<'a> Scoped<'a> {
    /// The bytes this file's symbols come from: the blob on
    /// [`indexed_side`]`(self.kind)`, or `None` when there is none.
    #[must_use]
    pub fn blob(&self) -> Option<&'a [u8]> {
        match indexed_side(self.kind) {
            Side::Left => self.base,
            Side::Right => self.head,
        }
    }
}

/// Which side's blob a file's symbols come from.
///
/// [`Side::Left`] for a removed file and [`Side::Right`] for everything else,
/// decided by asking [`anchored_side`] rather than by repeating it: a removed
/// file is a file made entirely of removed lines, and this project has already
/// shipped one bug from two places disagreeing about which side a thing is on.
#[must_use]
pub fn indexed_side(kind: ChangeKind) -> Side {
    anchored_side(match kind {
        ChangeKind::Removed => LineKind::Removed,
        ChangeKind::Added | ChangeKind::Modified | ChangeKind::Renamed => LineKind::Added,
    })
}

/// One place a reviewer can jump to: a symbol, and enough about where it lives
/// to get there and to say so afterwards.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    /// The definition itself, exactly as [`rv_core::symbols`] reported it.
    pub symbol: Symbol,
    /// The file it is in, in the caller's numbering — see [`Scoped::file`].
    pub file: usize,
    /// That file's path, carried so a status line or a picker row can name it
    /// without going back to the caller's list.
    pub path: String,
    /// The change that touched the file, in the commits view; `None` in the
    /// bookmark view.
    pub change_id: Option<String>,
}

/// Every symbol in scope, in the order a reviewer walks them.
///
/// Built once by [`Index::of`] and read from thereafter; it holds no reference
/// to the blobs it was built from, so a caller may cache it beside its diffs
/// and drop the bytes.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// In the caller's file order, and in line order within a file.
    entries: Vec<Entry>,
    /// Where each in-scope file sits in the caller's order, which is what
    /// gives a `(file, line)` cursor a position in the walk.
    ///
    /// Every file in scope is in here, including the ones that contributed no
    /// entries: a file with no grammar is still somewhere to step *from*. A
    /// file listed twice keeps the rank of its first appearance, which is the
    /// only answer that leaves the order total.
    ranks: HashMap<usize, usize>,
}

impl Index {
    /// Indexes every file in `scope`, in the order given.
    ///
    /// Each file's symbols come from its own side's blob (see
    /// [`indexed_side`]) and are found by [`rv_core::symbols::of`], which never
    /// fails: a path no grammar claims, a language with no tags query, bytes
    /// that are not UTF-8 and source that does not parse each contribute
    /// nothing. A file that cannot be indexed is a file with no symbols, not
    /// an error — a review is not the place to learn that one unreadable blob
    /// can cost you the jump list for all the others.
    #[must_use]
    pub fn of(scope: &[Scoped<'_>]) -> Self {
        let mut entries = Vec::new();
        let mut ranks = HashMap::with_capacity(scope.len());
        for (rank, scoped) in scope.iter().enumerate() {
            ranks.entry(scoped.file).or_insert(rank);
            let Some(blob) = scoped.blob() else {
                continue;
            };
            entries.extend(
                symbols::of(blob, scoped.path)
                    .into_iter()
                    .map(|symbol| Entry {
                        symbol,
                        file: scoped.file,
                        path: scoped.path.to_owned(),
                        change_id: scoped.change_id.map(str::to_owned),
                    }),
            );
        }
        Self { entries, ranks }
    }

    /// Every symbol in scope, in walk order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there is nothing to jump to — a scope of binaries, of files no
    /// grammar claims, or of nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The first symbol after the cursor, crossing into the next file when
    /// this one has no more, and `None` past the last one.
    ///
    /// `None` also for a `file` that is not in scope: a file the caller did
    /// not put in the view has no position in a walk over it, and inventing
    /// one would drop the reviewer somewhere they never asked to be.
    #[must_use]
    pub fn next_after(&self, file: usize, line: u32) -> Option<&Entry> {
        let here = (*self.ranks.get(&file)?, line);
        self.entries.iter().find(|entry| self.place(entry) > here)
    }

    /// The last symbol before the cursor, crossing back into the previous file
    /// when this one has no more, and `None` before the first one.
    ///
    /// The mirror of [`Index::next_after`] in every respect, including the
    /// answer for a file that is not in scope.
    #[must_use]
    pub fn previous_before(&self, file: usize, line: u32) -> Option<&Entry> {
        let here = (*self.ranks.get(&file)?, line);
        self.entries
            .iter()
            .rev()
            .find(|entry| self.place(entry) < here)
    }

    /// Where an entry sits in the walk: its file's rank, then its line.
    ///
    /// The entries are built in this order, so both searches above may stop at
    /// the first match. An entry's file is always in `ranks` — they are filled
    /// from the same pass — and a missing one would be a file with entries and
    /// no place, which is why it sorts after everything rather than panicking.
    fn place(&self, entry: &Entry) -> (usize, u32) {
        (
            self.ranks.get(&entry.file).copied().unwrap_or(usize::MAX),
            entry.symbol.line,
        )
    }
}
