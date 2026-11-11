//! `g i`: what the symbol under the cursor is, where it is declared and how
//! often it is used.
//!
//! The card is computed once, when asked for, and drawn from [`App::hover`]
//! every frame after — the renderer must never rebuild the plan or the index
//! to show it (issue #27).

use std::borrow::Cow;

use anyhow::Result;
use rv_core::model::Side;
use rv_core::symbols::SymbolKind;

use super::App;
use super::bindings::Command;
use super::bindings::DiffCommand;
use crate::index::indexed_side;
use crate::layout::CardChrome;
use crate::ui;

/// The most rows a card takes, borders included.
const MAX_ROWS: usize = 12;

/// The widest a card asks to be; the layout narrows it further on a small pane.
const MAX_COLUMNS: usize = 80;

/// How many lines of the declaration a card quotes.
const DECLARATION_LINES: usize = 6;

/// Where an answer came from, so a card can say how far to trust it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpSource {
    /// The tree-sitter index: names, not types.
    Grammar,
    LanguageServer,
    /// jj's annotation of the file: the card names a line's change, not a symbol.
    Annotation,
}

impl JumpSource {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            JumpSource::Grammar => "tree-sitter",
            JumpSource::LanguageServer => "language server",
            JumpSource::Annotation => "last touched this line",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub path: String,
    pub line: u32,
    /// The declaration's first lines, dedented, with control characters gone.
    pub text: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverCard {
    pub name: String,
    pub kind: Option<SymbolKind>,
    pub declaration: Option<Declaration>,
    pub references: usize,
    pub source: JumpSource,
    /// The language server's hover text, sanitized and clipped.
    pub server_text: Vec<String>,
    /// Why the card is the grammar's answer only, where that is worth saying.
    pub note: Option<String>,
    /// The keys that act on the symbol, spelled from the runtime keymap.
    pub hints: String,
}

impl HoverCard {
    #[must_use]
    pub fn heading(&self) -> String {
        match self.kind {
            Some(kind) => format!("{} {}", kind.label(), self.name),
            None => self.name.clone(),
        }
    }

    #[must_use]
    pub fn location(&self) -> Option<String> {
        self.declaration
            .as_ref()
            .map(|declaration| format!("{}:{}", declaration.path, declaration.line))
    }

    /// The body rows: the declaration, or why there is none.
    #[must_use]
    pub fn body(&self) -> Vec<String> {
        let mut body = match &self.declaration {
            _ if !self.server_text.is_empty() => self.server_text.clone(),
            Some(declaration) => declaration.text.clone(),
            None => vec![format!("no definition of {} in this review", self.name)],
        };
        body.extend(self.note.clone());
        body
    }

    #[must_use]
    pub fn usage(&self) -> String {
        if self.source == JumpSource::Annotation {
            return self.source.label().to_owned();
        }
        let noun = if self.references == 1 {
            "reference"
        } else {
            "references"
        };
        format!("{} {noun} · {}", self.references, self.source.label())
    }

    fn size(&self) -> (usize, usize) {
        let header = self.heading().chars().count()
            + self
                .location()
                .map_or(0, |location| location.chars().count() + 2);
        let body = self.body();
        let widest = body
            .iter()
            .map(|line| line.chars().count() + 2)
            .chain([
                header,
                self.usage().chars().count(),
                self.hints.chars().count(),
            ])
            .max()
            .unwrap_or(0);
        let rows = (body.len() + 3 + 2).min(MAX_ROWS);
        (rows, (widest + 4).min(MAX_COLUMNS))
    }
}

/// An open card, and the diff pane content row it describes.
pub(in crate::app) struct Hover {
    pub(in crate::app) card: HoverCard,
    pub(in crate::app) row: u16,
    /// Whether the pointer raised it, which is what lets moving off close it.
    pub(in crate::app) by_pointer: bool,
}

impl App {
    #[must_use]
    pub fn hover(&self) -> Option<&HoverCard> {
        self.hover.as_ref().map(|hover| &hover.card)
    }

    /// The card's size and anchor, for the layout to place.
    #[must_use]
    pub fn hover_chrome(&self) -> Option<CardChrome> {
        let hover = self.hover.as_ref()?;
        let (rows, columns) = hover.card.size();
        Some(CardChrome {
            row: hover.row,
            rows: u16::try_from(rows).unwrap_or(u16::MAX),
            columns: u16::try_from(columns).unwrap_or(u16::MAX),
        })
    }

    pub(super) fn close_hover(&mut self) -> bool {
        self.hover.take().is_some()
    }

    pub(super) fn show_hover(&mut self) -> Result<()> {
        let Some(name) = self.symbol_under_cursor() else {
            self.status = "no symbol under the cursor".to_owned();
            return Ok(());
        };
        let wanted = self.use_kind_under_cursor();
        let asking = self.lsp_question(rv_core::lsp::RequestKind::Hover, &name);
        self.open_card_at_cursor(&name, wanted);
        self.ask_language_server(asking);
        Ok(())
    }

    /// The card `g d` and `g r` raise when they find nothing to go to, so the
    /// reviewer still learns what the grammar knows about the name.
    pub(super) fn hover_after_miss(&mut self, name: &str) {
        let wanted = self.use_kind_under_cursor();
        self.open_card_at_cursor(name, wanted);
    }

    fn open_card_at_cursor(&mut self, name: &str, wanted: Option<SymbolKind>) {
        let card = self.card_for(name, wanted);
        let pane = self.painted.get().diff;
        let row = ui::diff_screen_row(self, pane, self.cursor_row()).unwrap_or(0);
        self.hover = Some(Hover {
            card,
            row: u16::try_from(row).unwrap_or(u16::MAX),
            by_pointer: false,
        });
    }

    pub(super) fn card_for(&mut self, name: &str, wanted: Option<SymbolKind>) -> HoverCard {
        let definition = self.definitions_of(name, wanted).into_iter().next();
        let mut places: Vec<(usize, u32)> = self
            .index()
            .uses_named(name)
            .iter()
            .map(|found| (found.file, found.reference.line))
            .collect();
        places.sort_unstable();
        places.dedup();
        HoverCard {
            name: name.to_owned(),
            kind: definition.as_ref().map(|entry| entry.symbol.kind),
            declaration: definition.map(|entry| Declaration {
                text: self.declaration_text(entry.file, entry.symbol.line),
                path: entry.path,
                line: entry.symbol.line,
            }),
            references: places.len(),
            source: JumpSource::Grammar,
            server_text: Vec::new(),
            note: None,
            hints: self.hover_hints(),
        }
    }

    fn hover_hints(&self) -> String {
        [
            (DiffCommand::Definition, "definition"),
            (DiffCommand::References, "references"),
        ]
        .into_iter()
        .filter_map(|(command, what)| {
            self.keymap
                .chord_for(Command::Diff(command))
                .map(|chord| format!("{chord} {what}"))
        })
        .chain(["Esc close".to_owned()])
        .collect::<Vec<_>>()
        .join(" · ")
    }

    /// The declaration's line and the deeper-indented lines under it.
    fn declaration_text(&self, file: usize, line: u32) -> Vec<String> {
        let Some(blob) = self.indexed_blob(file) else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&blob);
        let mut lines = text.lines().skip(line.saturating_sub(1) as usize);
        let Some(first) = lines.next() else {
            return Vec::new();
        };
        let depth = indent(first);
        let mut quoted = vec![first];
        quoted.extend(
            lines
                .take(DECLARATION_LINES - 1)
                .take_while(|next| !next.trim().is_empty() && indent(next) > depth),
        );
        quoted
            .into_iter()
            .map(|line| printable(line.get(depth..).unwrap_or(line.trim_start())))
            .collect()
    }

    /// The bytes `file`'s symbols were indexed from: the loaded diff's blob
    /// where there is one, else a read of the same side.
    fn indexed_blob(&self, file: usize) -> Option<Cow<'_, [u8]>> {
        let kind = self.review.files.get(file)?.kind;
        if let Some((base, head)) = self.blobs.get(file).and_then(Option::as_ref) {
            return Some(Cow::Borrowed(match indexed_side(kind) {
                Side::Left => base,
                Side::Right => head,
            }));
        }
        let scoped = self
            .scoped_files(&self.whole_review())
            .into_iter()
            .find(|scoped| scoped.file == file)?;
        self.read_indexable(scoped).blob.map(Cow::Owned)
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `text` safe to draw in a cell grid: tabs as spaces, other controls dropped.
fn printable(text: &str) -> String {
    text.trim_end()
        .chars()
        .flat_map(|character| match character {
            '\t' => vec![' '; 4],
            other if other.is_control() => Vec::new(),
            other => vec![other],
        })
        .collect()
}
