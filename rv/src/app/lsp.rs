//! Language-server refinement of `g d`, `g r` and `g i`: the grammar answers
//! at once, the server's answer replaces it when it lands and the reviewer is
//! still where they asked. The swap and staleness rules are in
//! `docs/superpowers/specs/2026-11-11-rv-lsp-navigation-design.md`.

mod answers;
mod hovertext;
mod status;

use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use rv_core::lsp::AnswerCache;
use rv_core::lsp::CacheKey;
use rv_core::lsp::LspHub;
use rv_core::lsp::RequestKind;
use rv_core::lsp::SearchPath;
use rv_core::lsp::ServerRegistry;
use rv_core::lsp::ServerStatus;
use rv_core::lsp::Ticket;

use super::App;
use super::JumpSource;
use super::Mode;
use super::anchor::SourcePosition;
use super::diffs::Target;
use crate::session::Review;

/// How long an unanswered request is waited for before it is dropped.
const PATIENCE: Duration = Duration::from_secs(30);

pub(in crate::app) const NOT_ON_DISK: &str = "grammar only — this revision is not on disk";
const LINE_NOT_ON_DISK: &str = "grammar only — this line is not on disk";

pub(in crate::app) struct Navigation {
    state: State,
    /// Why the latest answer could only come from the grammar.
    note: Option<&'static str>,
    /// Where the open jump list came from.
    list_source: JumpSource,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            state: State::Off,
            note: None,
            list_source: JumpSource::Grammar,
        }
    }
}

enum State {
    Off,
    NotOnDisk(ServerRegistry),
    Live(Box<Live>),
}

struct Live {
    hub: Option<LspHub>,
    registry: ServerRegistry,
    pending: Option<Pending>,
    generation: u64,
    cache: AnswerCache,
}

/// Shutting a server down can take over a second, so the hub is dropped off
/// the thread that draws the screen.
impl Drop for Live {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.take() {
            let _ = std::thread::Builder::new()
                .name("lsp-shutdown".into())
                .spawn(move || drop(hub));
        }
    }
}

struct Pending {
    generation: u64,
    question: Question,
    landing: Spot,
    ticket: Ticket,
    first_seen: Option<Instant>,
}

/// One request, and the reviewer's place when it was asked.
#[derive(Clone)]
pub(in crate::app) struct Question {
    pub(in crate::app) kind: RequestKind,
    pub(in crate::app) word: String,
    pub(in crate::app) file: usize,
    pub(in crate::app) path: String,
    pub(in crate::app) line: u32,
    pub(in crate::app) column: u32,
    pub(in crate::app) origin: Spot,
}

/// Where the reviewer is, by code position rather than plan row, so a
/// background diff swap that re-plans the pane does not count as moving.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct Spot {
    file: usize,
    position: Option<SourcePosition>,
    column: usize,
    mode: Mode,
}

/// What [`App::lsp_question`] found: nothing to ask (LSP off), a reason the
/// answer stays grammar-only, or a question to send.
pub(in crate::app) enum Asking {
    Off,
    GrammarOnly(&'static str),
    Ask(Question),
}

fn on_disk(review: &Review) -> bool {
    review
        .repo
        .head_is_working_copy(&review.session.head_commit)
}

/// A server binary checked into the repo under review is never run.
fn discover_servers(registry: ServerRegistry, review: &Review) -> ServerRegistry {
    let is_tracked = |relative: &std::path::Path| {
        relative
            .to_str()
            .is_none_or(|path| review.repo.tracks(path).unwrap_or(true))
    };
    registry.discovered(&SearchPath::for_repo(review.store.root()), &is_tracked)
}

impl Navigation {
    pub(in crate::app) fn open(registry: Option<ServerRegistry>, review: &Review) -> Self {
        let state = match registry {
            None => State::Off,
            Some(registry) if !on_disk(review) => State::NotOnDisk(registry),
            Some(registry) => State::Live(Box::new(Live {
                hub: Some(LspHub::new(
                    review.store.root(),
                    discover_servers(registry.clone(), review),
                )),
                registry,
                pending: None,
                generation: 0,
                cache: AnswerCache::default(),
            })),
        };
        Self {
            state,
            ..Self::default()
        }
    }

    /// The navigation a refreshed review carries: a running server survives
    /// while the head is still the working copy; its answers do not.
    pub(in crate::app) fn reopened(self, review: &Review) -> Self {
        let registry = match self.state {
            State::Off => return Self::default(),
            State::NotOnDisk(registry) => registry,
            State::Live(mut live) => {
                if on_disk(review) {
                    live.pending = None;
                    live.cache.clear();
                    return Self {
                        state: State::Live(live),
                        ..Self::default()
                    };
                }
                live.registry.clone()
            }
        };
        Self::open(Some(registry), review)
    }

    fn hub(&self) -> Option<&LspHub> {
        match &self.state {
            State::Live(live) => live.hub.as_ref(),
            _ => None,
        }
    }
}

impl App {
    pub(in crate::app) fn spot(&self) -> Spot {
        Spot {
            file: self.file_index,
            position: self.cursor_position(),
            column: self.column,
            mode: self.mode.clone(),
        }
    }

    /// Captured before the grammar acts, so the question is asked from where
    /// the reviewer stood rather than where a grammar jump landed them.
    pub(in crate::app) fn lsp_question(&self, kind: RequestKind, word: &str) -> Asking {
        match &self.navigation.state {
            State::Off => return Asking::Off,
            State::NotOnDisk(_) => return Asking::GrammarOnly(NOT_ON_DISK),
            State::Live(_) => {}
        }
        let line = self.selected_line();
        let right_line = line.as_ref().and_then(|line| line.right);
        let (Some(file), Some(line_number), Target::File(_)) =
            (self.selected_file(), right_line, self.shown_target())
        else {
            return Asking::GrammarOnly(LINE_NOT_ON_DISK);
        };
        let column = self
            .word_range()
            .map_or(0, |range| u32::try_from(range.start).unwrap_or(u32::MAX));
        Asking::Ask(Question {
            kind,
            word: word.to_owned(),
            file: self.file_index,
            path: file.path.clone(),
            line: line_number,
            column,
            origin: self.spot(),
        })
    }

    /// Sends `asking` after the grammar has answered. A cached answer is
    /// applied at once; otherwise the answer is polled for by
    /// [`App::collect_lsp`].
    pub(in crate::app) fn ask_language_server(&mut self, asking: Asking) {
        self.navigation.list_source = JumpSource::Grammar;
        let question = match asking {
            Asking::Off => {
                self.navigation.note = None;
                return;
            }
            Asking::GrammarOnly(note) => {
                self.navigation.note = Some(note);
                if let Some(hover) = self.hover.as_mut() {
                    hover.card.note = Some(note.to_owned());
                }
                return;
            }
            Asking::Ask(question) => question,
        };
        self.navigation.note = None;
        let landing = self.spot();
        let head = self.review.session.head_commit.clone();
        let State::Live(live) = &mut self.navigation.state else {
            return;
        };
        live.generation += 1;
        live.pending = None;
        let key = cache_key(&question);
        if let Some(answer) = live.cache.get(&key, &head).cloned() {
            self.apply_answer(&question, &landing, answer);
            return;
        }
        let Some(hub) = live.hub.as_mut() else {
            return;
        };
        let ticket = hub.request(
            question.kind,
            Path::new(&question.path),
            question.line,
            question.column,
        );
        live.pending = Some(Pending {
            generation: live.generation,
            question,
            landing,
            ticket,
            first_seen: None,
        });
    }

    /// Takes a landed answer, if any, and applies it when it is still the
    /// latest question. Also starts the selected file's server ahead of the
    /// first request. Never blocks.
    pub fn collect_lsp(&mut self, now: Instant) -> bool {
        self.warm_language_server();
        let head = self.review.session.head_commit.clone();
        let State::Live(live) = &mut self.navigation.state else {
            return false;
        };
        let Some(pending) = live.pending.as_mut() else {
            return false;
        };
        let first_seen = *pending.first_seen.get_or_insert(now);
        let Some(outcome) = pending.ticket.try_take() else {
            if now.saturating_duration_since(first_seen) > PATIENCE {
                live.pending = None;
            }
            return false;
        };
        let Some(pending) = live.pending.take() else {
            return false;
        };
        let Ok(answer) = outcome else {
            return false;
        };
        if pending.generation != live.generation {
            return false;
        }
        live.cache
            .insert(cache_key(&pending.question), &head, answer.clone());
        self.apply_answer(&pending.question, &pending.landing, answer);
        true
    }

    fn warm_language_server(&mut self) {
        let State::Live(live) = &mut self.navigation.state else {
            return;
        };
        let (Some(hub), Some(file)) = (live.hub.as_mut(), self.review.files.get(self.file_index))
        else {
            return;
        };
        let path = Path::new(&file.path);
        if hub.status_for(path) == Some(ServerStatus::Idle) {
            hub.start_for(path);
        }
    }

    /// Whether a language server answer is outstanding.
    #[must_use]
    pub fn lsp_pending(&self) -> bool {
        matches!(&self.navigation.state, State::Live(live) if live.pending.is_some())
    }
}

fn cache_key(question: &Question) -> CacheKey {
    CacheKey {
        path: question.path.clone().into(),
        line: question.line,
        column: question.column,
        kind: question.kind,
    }
}
